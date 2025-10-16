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
use manager::DownloadManager;
use types::DownloadTask;
use std::path::PathBuf;
const DEFAULT_CHUNK_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

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
        output,
        total_size: 0,
        chunk_size: chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE),
        num_chunks: 0,
    };
    let manager = DownloadManager::new();
    manager.start(task).await
}
