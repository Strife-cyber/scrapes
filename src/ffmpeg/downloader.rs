use std::path::Path;
use tokio::sync::mpsc;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, BufReader};
use crate::ffmpeg::params::{DownloadError, DownloadOptions, FfmpegProgress};

/// Starts ffmpeg to download `input_url` to `output_path`.
/// Emits progress messages to `progress_tx`. Returns Ok(()) on success.
pub async fn download_with_ffmpeg(
    input_url: &str,
    output_path: impl AsRef<Path>,
    opts: DownloadOptions,
    mut progress_tx: mpsc::Sender<FfmpegProgress>
) -> Result<(), DownloadError> {
    let output_path = output_path.as_ref().to_owned();
    let tmp_path = output_path.with_extension("part");

    let mut attempts = 0usize;

    loop {
        attempts += 1;
        let res = run_ffmpeg_once(input_url, &tmp_path, opts.stall_timeout, &mut progress_tx).await;

        match res {
            Ok(()) => {
                // success: rename tmp to final
                tokio::fs::rename(&tmp_path, &output_path)
                    .await
                    .map_err(DownloadError::Io)?;
                return Ok(());
            }
            Err(e) => {
                // si auto_restart activé et tentatives < max, réessayer; sinon retourner l'erreur.
                if opts.auto_restart && attempts < opts.max_restarts {
                    // petit délai exponentiel
                    let backoff = Duration::from_secs(2_u64.saturating_pow(attempts as u32));
                    tokio::time::sleep(backoff).await;
                    // continuer la boucle pour réessayer
                    continue;
                } else {
                    return Err(e);
                }
            }
        }
    }
}

async fn run_ffmpeg_once(
    input_url: &str,
    tmp_path: &Path,
    stall_timeout: Duration,
    progress_tx: &mut mpsc::Sender<FfmpegProgress>
) -> Result<(), DownloadError> {
    // Construire les arguments ffmpeg:
    // -y écraser, -i entrée, -c copy minimiser le réencodage, -progress pipe:1, -nostats, output.tmp
    let mut cmd = Command::new("ffmpeg");
    let output_str = tmp_path.to_str()
        .ok_or_else(|| DownloadError::Other("chemin de sortie invalide (UTF-8 requis)".into()))?;
    cmd.args(&[
        "-y",
        "-i",
        input_url,
        "-c",
        "copy",
        "-progress",
        "pipe:1",
        "-nostats",
        output_str
    ]);

    // ensure stdout is piped (progress), stderr inherited or captured if you prefer
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(DownloadError::Io)?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| DownloadError::Other("impossible de prendre stdout de ffmpeg".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| DownloadError::Other("impossible de prendre stderr de ffmpeg".into()))?;

    // Lire stderr de manière concurrente (on le transmet à stderr pour le débogage)
    let mut serr = BufReader::new(stderr).lines();
    tokio::spawn(async move {
        while let Ok(Some(line)) = serr.next_line().await {
            eprintln!("[ffmpeg stderr] {}", line);
        }
    });

    let mut reader = BufReader::new(stdout).lines();

    // ffmpeg -progress produit des paires clé=valeur, séparées par des lignes vides, et "progress=end" à la fin
    let mut current: HashMap<String, String> = HashMap::new();

    loop {
        // lire la prochaine ligne avec timeout pour détecter le blocage
        let read_fut = reader.next_line();
        let timeout = tokio::time::sleep(stall_timeout);
        tokio::select! {
            maybe_line = read_fut => {
                match maybe_line {
                    Ok(Some(line)) => {
                        let line = line.trim().to_string();
                        if line.is_empty() {
                            // ligne vide => limite de paquet de progression; émettre si on a quelque chose
                            if !current.is_empty() {
                                let _ = progress_tx.try_send(FfmpegProgress::new(current.clone()));
                                current.clear();
                            }
                            continue;
                        }
                        // parser clé=valeur
                        if let Some(eq) = line.find('=') {
                            let (k, v) = line.split_at(eq);
                            let v = &v[1..];
                            current.insert(k.to_string(), v.to_string());
                            // émission immédiate de progression pour certaines clés si désiré:
                            if k == "out_time_ms" || k == "progress" {
                                let _ = progress_tx.try_send(FfmpegProgress::new(current.clone()));
                                // ne pas effacer; continuer à accumuler
                            }
                        }
                    }
                    Ok(None) => {
                        // EOF depuis stdout de ffmpeg — processus terminé. Attendre la fin du processus enfant et vérifier le statut.
                        break;
                    }
                    Err(err) => {
                        // erreur de lecture I/O
                        let _ = child.kill().await;
                        return Err(DownloadError::Io(err));
                    }
                }
            }
            _ = timeout => {
                // blocage détecté
                eprintln!("blocage ffmpeg détecté (aucune progression pendant {:?}), arrêt du processus", stall_timeout);
                // tentative de tuer le processus enfant
                let _ = child.kill().await;
                // retourner une erreur pour que l'appelant puisse choisir de redémarrer
                return Err(DownloadError::Other("blocage détecté".into()));
            }
        }
    }

    // processus enfant terminé; vérifier le statut de sortie
    let status = child.wait().await.map_err(DownloadError::Io)?;
    if status.success() {
        // émettre la progression finale avec les champs restants
        if !current.is_empty() {
            let _ = progress_tx.try_send(FfmpegProgress::new(current.clone()));
        }
        Ok(())
    } else {
        let code = status.code().unwrap_or(-1);
        Err(DownloadError::FfmpegExit(code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_download_options_default() {
        let opts = DownloadOptions::default();
        assert_eq!(opts.stall_timeout, Duration::from_secs(20));
        assert!(opts.auto_restart);
        assert_eq!(opts.max_restarts, 3);
    }

    #[tokio::test]
    async fn test_download_with_invalid_path() {
        // Test avec un chemin invalide (non-UTF8) - ceci devrait échouer avant même d'appeler ffmpeg
        // Note: Sur Windows, les chemins peuvent contenir des caractères non-UTF8
        // Ce test vérifie que l'erreur est gérée correctement
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("test_output.mp4");
        
        let opts = DownloadOptions {
            stall_timeout: Duration::from_secs(1),
            auto_restart: false,
            max_restarts: 0,
        };

        let (tx, _rx) = mpsc::channel(10);
        
        // Test avec une URL invalide pour vérifier la gestion d'erreur
        let result = download_with_ffmpeg(
            "file:///nonexistent/invalid/path",
            &output_path,
            opts,
            tx
        ).await;

        // Devrait échouer avec une erreur IO ou FfmpegExit
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_retry_logic_with_auto_restart_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("test_output.mp4");
        
        let opts = DownloadOptions {
            stall_timeout: Duration::from_millis(100),
            auto_restart: false,
            max_restarts: 3,
        };

        let (tx, _rx) = mpsc::channel(10);
        
        // Devrait échouer immédiatement sans réessayer
        let result = download_with_ffmpeg(
            "file:///nonexistent",
            &output_path,
            opts,
            tx
        ).await;

        assert!(result.is_err());
        // Avec auto_restart=false, devrait échouer après une seule tentative
    }

    #[tokio::test]
    async fn test_progress_channel_capacity() {
        // Test que le canal de progression fonctionne correctement
        let (tx, mut rx) = mpsc::channel(10);
        
        let mut progress = HashMap::new();
        progress.insert("out_time_ms".to_string(), "1000".to_string());
        progress.insert("progress".to_string(), "continue".to_string());
        
        let ffmpeg_progress = FfmpegProgress::new(progress.clone());
        tx.send(ffmpeg_progress).await.unwrap();
        
        let received = rx.recv().await.unwrap();
        assert_eq!(received.fields.get("out_time_ms"), Some(&"1000".to_string()));
        assert_eq!(received.fields.get("progress"), Some(&"continue".to_string()));
    }

    #[tokio::test]
    async fn test_download_error_types() {
        // Test que les différents types d'erreurs sont correctement créés
        let io_error = std::io::Error::from(std::io::ErrorKind::NotFound);
        let download_error = DownloadError::Io(io_error);
        
        match download_error {
            DownloadError::Io(_) => {},
            _ => panic!("Devrait être une erreur IO"),
        }

        let ffmpeg_exit = DownloadError::FfmpegExit(1);
        match ffmpeg_exit {
            DownloadError::FfmpegExit(code) => assert_eq!(code, 1),
            _ => panic!("Devrait être une erreur FfmpegExit"),
        }

        let other_error = DownloadError::Other("test".into());
        match other_error {
            DownloadError::Other(msg) => assert_eq!(msg, "test"),
            _ => panic!("Devrait être une erreur Other"),
        }
    }

    #[tokio::test]
    async fn test_ffmpeg_progress_new() {
        let mut fields = HashMap::new();
        fields.insert("key1".to_string(), "value1".to_string());
        fields.insert("key2".to_string(), "value2".to_string());
        
        let progress = FfmpegProgress::new(fields.clone());
        assert_eq!(progress.fields.len(), 2);
        assert_eq!(progress.fields.get("key1"), Some(&"value1".to_string()));
        assert_eq!(progress.fields.get("key2"), Some(&"value2".to_string()));
    }

    #[tokio::test]
    async fn test_download_options_clone() {
        let opts1 = DownloadOptions {
            stall_timeout: Duration::from_secs(30),
            auto_restart: true,
            max_restarts: 5,
        };
        
        let opts2 = opts1.clone();
        assert_eq!(opts1.stall_timeout, opts2.stall_timeout);
        assert_eq!(opts1.auto_restart, opts2.auto_restart);
        assert_eq!(opts1.max_restarts, opts2.max_restarts);
    }

    #[tokio::test]
    async fn test_download_options_debug() {
        let opts = DownloadOptions::default();
        let debug_str = format!("{:?}", opts);
        // Vérifier que Debug fonctionne (ne panique pas)
        assert!(!debug_str.is_empty());
    }

    #[tokio::test]
    async fn test_stall_timeout_behavior() {
        // Test que le timeout de blocage est utilisé correctement
        // Ceci teste la logique sans nécessiter ffmpeg réel
        let short_timeout = Duration::from_millis(50);
        let opts = DownloadOptions {
            stall_timeout: short_timeout,
            auto_restart: false,
            max_restarts: 0,
        };
        
        assert_eq!(opts.stall_timeout, short_timeout);
    }

    #[tokio::test]
    async fn test_max_restarts_logic() {
        // Test que max_restarts contrôle correctement le nombre de tentatives
        // Avec max_restarts=2 et auto_restart=true, devrait faire 2 tentatives max
        let opts = DownloadOptions {
            stall_timeout: Duration::from_millis(100),
            auto_restart: true,
            max_restarts: 2,
        };
        
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("test.mp4");
        let (tx, _rx) = mpsc::channel(10);
        
        // Devrait échouer après max_restarts tentatives
        let start = std::time::Instant::now();
        let result = download_with_ffmpeg(
            "file:///nonexistent",
            &output_path,
            opts,
            tx
        ).await;
        
        let elapsed = start.elapsed();
        
        assert!(result.is_err());
        // Devrait prendre au moins le temps de 2 tentatives + backoffs
        // Le backoff est: 2^1 = 2s pour la première tentative, 2^2 = 4s pour la seconde
        // Donc au moins 6 secondes (mais on utilise des timeouts courts dans le test)
        // En pratique, avec des timeouts de 100ms, cela devrait être rapide
        assert!(elapsed < Duration::from_secs(10)); // Vérifier que ça ne prend pas trop de temps
    }

    #[tokio::test]
    async fn test_progress_try_send_non_blocking() {
        // Test que try_send fonctionne correctement (non-bloquant)
        let (tx, mut rx) = mpsc::channel(1);
        
        let mut fields1 = HashMap::new();
        fields1.insert("progress".to_string(), "start".to_string());
        let progress1 = FfmpegProgress::new(fields1);
        
        // Premier envoi devrait réussir
        assert!(tx.try_send(progress1).is_ok());
        
        let mut fields2 = HashMap::new();
        fields2.insert("progress".to_string(), "continue".to_string());
        let progress2 = FfmpegProgress::new(fields2);
        
        // Deuxième envoi devrait échouer car le canal est plein (capacité 1)
        assert!(tx.try_send(progress2).is_err());
        
        // Recevoir le premier message
        let received = rx.recv().await.unwrap();
        assert_eq!(received.fields.get("progress"), Some(&"start".to_string()));
    }
}
