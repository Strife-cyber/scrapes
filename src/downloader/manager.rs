//! Orchestrateur de préparation et (futur) pilotage du téléchargement.
//!
//! Rôle:
//! - Préparer la liste des segments et les fichiers temporaires associés.
//! - À terme, lancer les téléchargements parallèles (HTTP `Range`) et agréger la progression.
//!
//! Performance:
//! - Chaque fichier de chunk est pré‑alloué à la taille exacte de son segment
//!   pour éviter des réallocations et garantir des écritures positionnées efficaces.
use std::{io};
use reqwest::Client;
use tokio::fs::{OpenOptions};
use anyhow::{Context, Result};
use tokio::io::{AsyncWriteExt};
use std::path::{Path, PathBuf};
use futures::stream::{self, StreamExt};
use reqwest::header::{ACCEPT_RANGES, CONTENT_LENGTH, RANGE};
use super::utils::{create_empty_file, merge_chunks};
use super::types::{DownloadTask, Chunk};

pub struct DownloadManager;

impl DownloadManager {
    /// Initialise un nouveau gestionnaire de téléchargement
    pub fn new() -> Self {
        Self
    }

    /// Prépare les métadonnées des chunks et les fichiers disque associés.
    ///
    /// Détails:
    /// - Génère les segments via `DownloadTask::create_chunks`.
    /// - Pour chaque segment, crée un fichier temporaire `output.part<index>` si absent,
    ///   avec une taille pré‑allouée correspondant exactement à `[start..=end]`.
    pub fn prepare(&self, task: &DownloadTask) -> io::Result<Vec<Chunk>> {
        tracing::info!(url = %task.url, total_size = task.total_size, chunk_size = task.chunk_size, "Préparation des segments");
        let chunks = task.create_chunks();

        for chunk in &chunks {
            // Créer le fichier part si absent, pré‑alloué à la taille réelle du chunk
            if !chunk.path.exists() {
                tracing::debug!(index = chunk.index, start = chunk.start, end = chunk.end, path = %chunk.path.display(), "Création du fichier de partie");
                let part_len = (chunk.end - chunk.start) + 1;
                create_empty_file(&chunk.path, part_len)?;
            }
        }

        Ok(chunks)
    }

    /// Démarre un téléchargement parallèle par plages HTTP (`Range`).
    ///
    /// Stratégie:
    /// - Détecte `content-length` et support `accept-ranges` via HEAD si nécessaire.
    /// - Prépare les fichiers de parties pour chaque segment.
    /// - Télécharge les segments en parallèle avec une limite de concurrence.
    /// - Fusionne les parties en un fichier final à la fin.
    pub async fn start(&self, mut task: DownloadTask) -> Result<()> {
        tracing::info!(url = %task.url, "Démarrage du téléchargement");
        let client = Client::builder().build().context("Créer client HTTP")?;

        // Déterminer la taille et le support des ranges si absent
        let (total_size, supports_range) = self
            .detect_remote_metadata(&client, &task)
            .await
            .context("Détecter métadonnées distantes")?;
        task.total_size = total_size;
        tracing::info!(total_size, supports_range, "Métadonnées distantes récupérées");

        // Si le serveur ne supporte pas les ranges, télécharger en 1 requête
        if !supports_range {
            tracing::warn!("Serveur sans support Range: téléchargement en une requête");
            self.download_whole(&client, &task).await?;
            return Ok(());
        }

        // Préparer les chunks et fichiers
        let chunks = self.prepare(&task).context("Préparer chunks")?;

        // Reprise: ignorer les segments déjà complétés (présence d'un marqueur .done)
        let to_download: Vec<Chunk> = chunks
            .iter()
            .cloned()
            .filter(|c| {
                let marker = done_marker_path(&c.path);
                !marker.exists()
            })
            .collect();
        tracing::info!(pending = to_download.len(), total = chunks.len(), "Segments à télécharger");

        // Concurrence bornée
        let max_concurrency = 8usize;
        tracing::info!(max_concurrency, "Téléchargements parallèles");

        let url = task.url.clone();
        stream::iter(to_download.clone())
            .map(|chunk| {
                let client = client.clone();
                let url = url.clone();
                async move {
                    if let Err(e) = download_chunk(&client, &url, &chunk).await {
                        Err(anyhow::anyhow!("chunk {}: {}", chunk.index, e))
                    } else {
                        Ok(())
                    }
                }
            })
            .buffer_unordered(max_concurrency)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        // Fusion des fichiers partiels
        let part_paths: Vec<_> = chunks.iter().map(|c| c.path.as_path()).collect();
        tracing::info!(file = %task.output.display(), parts = part_paths.len(), "Fusion des parties en sortie");
        merge_chunks(&part_paths, &task.output).context("Fusionner chunks")?;
        
        // Nettoyage des fichiers temporaires
        self.cleanup_temp_files(&chunks).context("Nettoyer fichiers temporaires")?;
        
        tracing::info!(file = %task.output.display(), "Téléchargement terminé");
        Ok(())
    }

    /// Effectue une requête HEAD pour récupérer `content-length` et `accept-ranges`.
    async fn detect_remote_metadata(&self, client: &Client, task: &DownloadTask) -> Result<(u64, bool)> {
        if task.total_size > 0 {
            // On connaît déjà la taille; supposer support des ranges et laisser le serveur répondre 206
            return Ok((task.total_size, true));
        }

        let resp = client.head(&task.url).send().await.context("HEAD request")?;
        resp.error_for_status_ref().context("HEAD status")?;

        let len = resp
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .context("En‑tête content-length manquant/invalide")?;

        let supports_range = resp
            .headers()
            .get(ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("bytes"))
            .unwrap_or(false);

        Ok((len, supports_range))
    }

    /// Télécharge tout le fichier en une seule requête (fallback sans `Range`).
    async fn download_whole(&self, client: &Client, task: &DownloadTask) -> Result<()> {
        let resp = client.get(&task.url).send().await.context("GET complet")?;
        let mut resp = resp.error_for_status().context("GET status")?;

        // Écrire directement dans le fichier final
        let mut file = OpenOptions::new().create(true).truncate(true).write(true).open(&task.output).await?;
        let mut downloaded: u64 = 0;
        while let Some(chunk) = resp.chunk().await.context("Lire chunk HTTP")? {
            downloaded += chunk.len() as u64;
            file.write_all(&chunk).await?;
            tracing::debug!(downloaded, "Téléchargement plein en cours");
        }
        file.flush().await?;
        Ok(())
    }

    /// Nettoie les fichiers temporaires après fusion réussie
    fn cleanup_temp_files(&self, chunks: &[Chunk]) -> io::Result<()> {
        tracing::info!("Nettoyage des fichiers temporaires");
        
        for chunk in chunks {
            // Supprimer le fichier part
            if chunk.path.exists() {
                std::fs::remove_file(&chunk.path)?;
                tracing::debug!(path = %chunk.path.display(), "Fichier part supprimé");
            }
            
            // Supprimer le marqueur .done
            let marker = done_marker_path(&chunk.path);
            if marker.exists() {
                std::fs::remove_file(&marker)?;
                tracing::debug!(path = %marker.display(), "Marqueur .done supprimé");
            }
        }
        
        tracing::info!("Nettoyage terminé");
        Ok(())
    }
}

/// Télécharge un segment unique via HTTP `Range` et l'écrit dans le fichier part.
async fn download_chunk(client: &Client, url: &str, chunk: &Chunk) -> Result<()> {
    tracing::info!(index = chunk.index, start = chunk.start, end = chunk.end, "Téléchargement du segment");
    let range_header = format!("bytes={}-{}", chunk.start, chunk.end);
    let resp = client
        .get(url)
        .header(RANGE, range_header)
        .send()
        .await
        .context("GET range")?;

    // 206 attendu pour une réponse de plage partielle
    let mut resp = resp.error_for_status().context("GET status")?;

    // Ouvrir le fichier part et écrire en flux
    let part_path = &chunk.path;
    let mut file = OpenOptions::new().write(true).truncate(true).open(part_path).await?;

    let mut downloaded: u64 = 0;
    while let Some(bytes) = resp.chunk().await.context("Lire chunk HTTP")? {
        downloaded += bytes.len() as u64;
        file.write_all(&bytes).await?;
        tracing::debug!(index = chunk.index, downloaded, "Flux reçu pour le segment");
    }
    file.flush().await?;
    // Marquer ce segment comme complété
    let marker = done_marker_path(part_path);
    let _ = OpenOptions::new().create(true).write(true).open(marker).await?;
    tracing::info!(index = chunk.index, "Segment complété");
    Ok(())
}

fn done_marker_path(part_path: &Path) -> PathBuf {
    let name = part_path.file_name().unwrap_or_else(|| std::ffi::OsStr::new("part"));
    let mut s = name.to_string_lossy().to_string();
    s.push_str(".done");
    part_path.with_file_name(s)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::types::DownloadTask;
    use tempfile::tempdir;
    use std::fs;
    use std::net::TcpListener as StdTcpListener;
    use hyper::{Body, Request, Response, Server, Method};
    use hyper::service::{make_service_fn, service_fn};
    use hyper::header::{CONTENT_LENGTH as H_CONTENT_LENGTH, CONTENT_RANGE as H_CONTENT_RANGE, RANGE as H_RANGE, ACCEPT_RANGES as H_ACCEPT_RANGES};
    use hyper::StatusCode;
    use tokio::sync::oneshot;

    #[test]
    fn test_prepare_creates_chunks_and_files() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("file.bin");

        let task = DownloadTask {
            url: "https://example.com/file".to_string(),
            output: output_path.clone(),
            total_size: 3_000,
            chunk_size: 1_000,
            num_chunks: 0,
        };

        let manager = DownloadManager::new();
        let chunks = manager.prepare(&task).unwrap();

        // Should create 3 chunks
        assert_eq!(chunks.len(), 3);

        // Each chunk file should exist
        for chunk in &chunks {
            assert!(chunk.path.exists(), "Chunk file {:?} should exist", chunk.path);
            let metadata = fs::metadata(&chunk.path).unwrap();
            // Each file should be preallocated to chunk_size
            assert_eq!(metadata.len(), task.chunk_size);
        }

        // Check chunk boundaries
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks[0].end, 999);
        assert_eq!(chunks[2].start, 2000);
        assert_eq!(chunks[2].end, 2999);
    }

    #[test]
    fn test_prepare_existing_files() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("file.bin");

        let task = DownloadTask {
            url: "https://example.com/file".to_string(),
            output: output_path.clone(),
            total_size: 2_000,
            chunk_size: 1_000,
            num_chunks: 0,
        };

        // Pre-create one of the chunk files manually
        let precreated_file = output_path.with_extension("part0");
        fs::File::create(&precreated_file).unwrap();

        let manager = DownloadManager::new();
        let chunks = manager.prepare(&task).unwrap();

        // All chunk files should exist
        for chunk in &chunks {
            assert!(chunk.path.exists());
        }

        // The precreated file should not be overwritten, size should be 0
        let metadata = fs::metadata(&precreated_file).unwrap();
        assert_eq!(metadata.len(), 0);
    }

    #[test]
    fn test_prepare_zero_total_size() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("file.bin");

        let task = DownloadTask {
            url: "https://example.com/file".to_string(),
            output: output_path.clone(),
            total_size: 0,
            chunk_size: 1_000,
            num_chunks: 0,
        };

        let manager = DownloadManager::new();
        let chunks = manager.prepare(&task).unwrap();

        // No chunks should be returned
        assert!(chunks.is_empty());
    }

    async fn start_test_server(data: Vec<u8>, support_range: bool) -> (String, oneshot::Sender<()>) {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = oneshot::channel::<()>();

        let make_svc = make_service_fn(move |_| {
            let data = data.clone();
            async move {
                Ok::<_, hyper::Error>(service_fn(move |req: Request<Body>| {
                    let data = data.clone();
                    async move {
                        match (req.method().clone(), req.uri().path()) {
                            (m, "/file") if m == Method::HEAD => {
                                let mut builder = Response::builder()
                                    .status(StatusCode::OK)
                                    .header(H_CONTENT_LENGTH, data.len().to_string());
                                if support_range {
                                    builder = builder.header(H_ACCEPT_RANGES, "bytes");
                                }
                                Ok::<_, hyper::Error>(builder.body(Body::empty()).unwrap())
                            }
                            (m, "/file") if m == Method::GET => {
                                if support_range {
                                    if let Some(hv) = req.headers().get(H_RANGE) {
                                        if let Ok(s) = hv.to_str() {
                                            // attend "bytes=start-end"
                                            let s = s.trim();
                                            if let Some(range) = s.strip_prefix("bytes=") {
                                                let mut it = range.split('-');
                                                let start: usize = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                                                let end_opt = it.next().and_then(|v| v.parse::<usize>().ok());
                                                let end = end_opt.unwrap_or_else(|| data.len().saturating_sub(1));
                                                let start = start.min(data.len());
                                                let end = end.min(data.len().saturating_sub(1));
                                                let slice = if end >= start { &data[start..=end] } else { &data[0..0] };
                                                let content_range = format!("bytes {}-{}/{}", start, start + slice.len().saturating_sub(1), data.len());
                                                return Ok::<_, hyper::Error>(Response::builder()
                                                    .status(StatusCode::PARTIAL_CONTENT)
                                                    .header(H_CONTENT_LENGTH, slice.len())
                                                    .header(H_CONTENT_RANGE, content_range)
                                                    .body(Body::from(slice.to_vec()))
                                                    .unwrap());
                                            }
                                        }
                                    }
                                }
                                // Pas de Range ou pas support -> 200 plein
                                Ok::<_, hyper::Error>(Response::builder()
                                    .status(StatusCode::OK)
                                    .header(H_CONTENT_LENGTH, data.len())
                                    .body(Body::from(data.clone()))
                                    .unwrap())
                            }
                            _ => Ok::<_, hyper::Error>(Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap()),
                        }
                    }
                }))
            }
        });

        let server = Server::from_tcp(listener).unwrap().serve(make_svc);
        tokio::spawn(async move {
            let _ = server.with_graceful_shutdown(async move { let _ = rx.await; }).await;
        });

        (format!("http://{}:{}/file", addr.ip(), addr.port()), tx)
    }

    #[tokio::test]
    async fn test_start_ranged_download() {
        // Données de test
        let data: Vec<u8> = (0u8..=255).cycle().take(16 * 1024).collect(); // 16 KiB motif
        let (url, shutdown) = start_test_server(data.clone(), true).await;

        let dir = tempdir().unwrap();
        let output_path = dir.path().join("out_ranged.bin");

        let task = DownloadTask {
            url,
            output: output_path.clone(),
            total_size: 0, // sera détecté via HEAD
            chunk_size: 4096, // 4 KiB
            num_chunks: 0,
        };

        let manager = DownloadManager::new();
        manager.start(task).await.expect("ranged download should succeed");

        // Vérifier contenu
        let out = fs::read(&output_path).unwrap();
        assert_eq!(out.len(), data.len());
        assert_eq!(out, data);

        // Arrêt du serveur
        let _ = shutdown.send(());
    }

    #[tokio::test]
    async fn test_start_whole_download_no_range() {
        let data = b"Hello full body without range".to_vec();
        let (url, shutdown) = start_test_server(data.clone(), false).await;

        let dir = tempdir().unwrap();
        let output_path = dir.path().join("out_whole.bin");

        let task = DownloadTask {
            url,
            output: output_path.clone(),
            total_size: 0, // via HEAD
            chunk_size: 4096,
            num_chunks: 0,
        };

        let manager = DownloadManager::new();
        manager.start(task).await.expect("whole download should succeed");

        let out = fs::read(&output_path).unwrap();
        assert_eq!(out, data);
        let _ = shutdown.send(());
    }
}
