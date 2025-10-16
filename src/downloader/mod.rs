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

/// API publique minimale: télécharge une ressource `url` vers `output`.
/// Cache l'ensemble des détails d'orchestration.
pub async fn download_to(url: String, output: PathBuf) -> anyhow::Result<()> {
    let task = DownloadTask {
        url,
        output,
        total_size: 0,
        chunk_size: 4 * 1024 * 1024, // 4 MiB par défaut
        num_chunks: 0,
    };
    let manager = DownloadManager::new();
    manager.start(task).await
}
