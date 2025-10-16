//! Types de base pour décrire une tâche de téléchargement et ses segments.
//!
//! Invariants principaux:
//! - `total_size` représente la taille totale attendue du fichier (en octets).
//! - `chunk_size` (> 0) est la taille cible d'un segment; le dernier peut être plus petit.
//! - Les segments générés couvrent l'intervalle `[0, total_size - 1]` sans chevauchement,
//!   et dans l'ordre croissant.
use std::path::PathBuf;

/// Représente un intervalle (chunk) d'un téléchargement
#[derive(Debug, Clone)]
pub struct Chunk {
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub downloaded: u64, // quantité déjà téléchargée pour ce segment
    pub path: PathBuf, // fichier temporaire associé à ce segment
}


/// Représente une tâche de téléchargement (fichier complet)
#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub url: String,
    pub output: PathBuf,
    pub total_size: u64,
    pub chunk_size: u64,
    pub num_chunks: usize,
}


impl DownloadTask {
    /// Génère les segments à partir de la taille totale et de la taille cible des chunks.
    ///
    /// Contrats:
    /// - Retourne un vecteur vide si `total_size == 0` ou `chunk_size == 0`.
    /// - Les bornes `start`/`end` sont inclusives et continues sans trou ni chevauchement.
    /// - La capacité du vecteur est réservée pour minimiser les réallocations.
    pub fn create_chunks(&self) -> Vec<Chunk> {
        // Garde contre les tailles invalides
        if self.total_size == 0 || self.chunk_size == 0 {
            return Vec::new();
        }

        let estimated_chunks = ((self.total_size + self.chunk_size - 1) / self.chunk_size) as usize;
        let mut chunks = Vec::with_capacity(estimated_chunks);
        let mut start = 0;
        let mut i = 0;

        while start < self.total_size {
            let end = (start + self.chunk_size - 1).min(self.total_size - 1);
            chunks.push(Chunk {
                index: i,
                start,
                end,
                downloaded: 0,
                // Nom de fichier de partie: `<output>.part<index>`
                path: self.output.with_extension(format!("part{}", i))
            });
            i += 1;
            start = end + 1;
        }

        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*; // import structs and impl from the parent module
    use std::path::PathBuf;

    #[test]
    fn test_create_chunks_exact_division() {
        // total_size = 4000 bytes, chunk_size = 1000 → should give 4 chunks of 1000 bytes each
        let task = DownloadTask {
            url: "https://example.com/file.bin".to_string(),
            output: PathBuf::from("file.bin"),
            total_size: 4000,
            chunk_size: 1000,
            num_chunks: 0,
        };

        let chunks = task.create_chunks();
        assert_eq!(chunks.len(), 4);

        // Verify start and end of each chunk
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks[0].end, 999);
        assert_eq!(chunks[3].start, 3000);
        assert_eq!(chunks[3].end, 3999);

        // Verify path naming
        assert_eq!(chunks[0].path, PathBuf::from("file.part0"));
        assert_eq!(chunks[3].path, PathBuf::from("file.part3"));
    }

    #[test]
    fn test_create_chunks_non_divisible() {
        // total_size = 4500, chunk_size = 1000 → should give 5 chunks (last smaller)
        let task = DownloadTask {
            url: "https://example.com/file.bin".to_string(),
            output: PathBuf::from("video.mp4"),
            total_size: 4500,
            chunk_size: 1000,
            num_chunks: 0,
        };

        let chunks = task.create_chunks();
        assert_eq!(chunks.len(), 5);

        // last chunk must end at total_size - 1
        let last = chunks.last().unwrap();
        assert_eq!(last.end, 4499);
        assert_eq!(last.start, 4000);

        // check no overlap and continuous range
        for w in chunks.windows(2) {
            assert_eq!(w[0].end + 1, w[1].start);
        }
    }

    #[test]
    fn test_create_chunks_single_chunk() {
        // file smaller than chunk size → only one chunk
        let task = DownloadTask {
            url: "https://example.com/small.txt".to_string(),
            output: PathBuf::from("small.txt"),
            total_size: 512,
            chunk_size: 1024,
            num_chunks: 0,
        };

        let chunks = task.create_chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks[0].end, 511);
    }

    #[test]
    fn test_create_chunks_zero_total_size() {
        // Edge case: empty file
        let task = DownloadTask {
            url: "https://example.com/empty.txt".to_string(),
            output: PathBuf::from("empty.txt"),
            total_size: 0,
            chunk_size: 1000,
            num_chunks: 0,
        };

        let chunks = task.create_chunks();
        assert!(chunks.is_empty());
    }
}
