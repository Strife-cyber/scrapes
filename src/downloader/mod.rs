//! Gestionnaire de téléchargement modulaire.
//!
//! Ce module regroupe:
//! - **types**: structures de données (`DownloadTask`, `Chunk`) et leurs invariants.
//! - **utils**: fonctions d'E/S (préallocation/merge) optimisées pour limiter les appels système.
//! - **manager**: logique de préparation et orchestration du téléchargement.
//!
//! Conception et performances:
//! - Les fichiers de parties sont pré‑alloués à la taille exacte du segment pour éviter les
//!   réallocations et garantir des écritures positionnées constantes.
//! - La fusion s'appuie sur des tampons de 1 MiB (lecture/écriture) afin de réduire le nombre
//!   d'appels système lors de la concaténation.
//! - `create_chunks` réserve la capacité du vecteur à l'avance et protège contre les tailles
//!   invalides (`total_size == 0` ou `chunk_size == 0`).
//!
//! Extension future:
//! - Ajout du téléchargement HTTP parallèle (plages `Range`) et reprise.
//! - Progression par chunk et agrégation vers un indicateur global.
//! - Vérification d'intégrité (hash) post‑merge.
mod types;
mod utils;
mod manager;

pub use manager::DownloadManager;
pub use types::DownloadTask;
use std::path::PathBuf;
use std::fs;
use serde::Deserialize;

const DEFAULT_CHUNK_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub logging: Option<LoggingConfig>,
    pub cleanup: Option<CleanupConfig>,
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    pub filter: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct CleanupConfig {
    /// Supprimer les fichiers temporaires après téléchargement réussi
    pub remove_temp_files: Option<bool>,
    /// Supprimer les fichiers temporaires en cas d'erreur
    pub remove_on_error: Option<bool>,
}

/// Charge la configuration depuis scrapes.toml
pub fn load_config() -> AppConfig {
    fs::read_to_string("scrapes.toml")
        .ok()
        .and_then(|s| toml::from_str::<AppConfig>(&s).ok())
        .unwrap_or_default()
}

/// Initialise le logging basé sur la configuration
pub fn init_logging() {
    let config = load_config();
    let file_filter = config.logging.and_then(|l| l.filter);
    let env = std::env::var("RUST_LOG").ok();
    let effective = file_filter.or(env).unwrap_or_else(|| "info".to_string());

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(effective))
        .with_target(false)
        .compact()
        .init();
}

/// Nettoie les fichiers temporaires en cas d'erreur
pub fn cleanup_temp_files_on_error(output: &PathBuf) {
    let output_dir = output.parent().unwrap_or(std::path::Path::new("."));
    let output_stem = output.file_stem().unwrap_or_else(|| std::ffi::OsStr::new("file"));
    
    // Chercher tous les fichiers .part* et .done
    if let Ok(entries) = fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with(&format!("{}.part", output_stem.to_string_lossy())) {
                    if let Err(e) = fs::remove_file(&path) {
                        tracing::warn!(?path, error = %e, "Impossible de supprimer le fichier part");
                    } else {
                        tracing::debug!(?path, "Fichier part supprimé après erreur");
                    }
                }
                if name.ends_with(".done") && name.starts_with(&format!("{}.part", output_stem.to_string_lossy())) {
                    if let Err(e) = fs::remove_file(&path) {
                        tracing::warn!(?path, error = %e, "Impossible de supprimer le marqueur .done");
                    } else {
                        tracing::debug!(?path, "Marqueur .done supprimé après erreur");
                    }
                }
            }
        }
    }
}

/// API publique minimale: télécharge une ressource `url` vers `output`.
/// Cache l'ensemble des détails d'orchestration.
pub async fn download_to(url: String, output: PathBuf) -> anyhow::Result<()> {
    download_to_with_chunk_size(url, output, None).await
}

/// Variante avec paramètre optionnel pour la taille des chunks.
/// Si `chunk_size` est `None`, une valeur par défaut performante est utilisée.
pub async fn download_to_with_chunk_size(
    url: String,
    output: PathBuf,
    chunk_size: Option<u64>,
) -> anyhow::Result<()> {
    let task = DownloadTask {
        url,
        output: output.clone(),
        total_size: 0,
        chunk_size: chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE),
        num_chunks: 0,
    };
    let manager = DownloadManager::new();
    
    match manager.start(task).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // Nettoyage en cas d'erreur si configuré
            let config = load_config();
            if config.cleanup.and_then(|c| c.remove_on_error).unwrap_or(false) {
                tracing::info!("Nettoyage des fichiers temporaires après erreur");
                cleanup_temp_files_on_error(&output);
            }
            Err(e)
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            logging: None,
            cleanup: None,
        }
    }
}
