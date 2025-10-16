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
use crate::downloader::{create_empty_file, Chunk, DownloadTask};

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
        let chunks = task.create_chunks();

        for chunk in &chunks {
            // Créer le fichier part si absent, pré‑alloué à la taille réelle du chunk
            if !chunk.path.exists() {
                let part_len = (chunk.end - chunk.start) + 1;
                create_empty_file(&chunk.path, part_len)?;
            }
        }

        Ok(chunks)
    }

    /// Démarre la procédure de téléchargement (placeholder pour l'instant).
    ///
    /// À implémenter:
    /// - Planification des requêtes parallèles avec en‑têtes `Range`.
    /// - Reprise sur erreur et validation des tailles reçues.
    pub fn start(&self, _task: DownloadTask) -> io::Result<()> {
        println!("Starting download for {:?}", _task.url);
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::{Chunk, DownloadTask};
    use tempfile::tempdir;
    use std::fs;

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
}
