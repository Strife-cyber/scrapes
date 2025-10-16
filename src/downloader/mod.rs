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
pub mod types;
pub mod utils;
pub mod manager;


pub use types::*;
pub use utils::*;
pub use manager::*;
