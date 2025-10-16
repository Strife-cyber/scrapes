//! Fonctions utilitaires d'E/S pour le téléchargement.
//!
//! Objectifs:
//! - Pré‑allouer des fichiers de parties à une taille donnée pour des écritures efficaces.
//! - Fusionner des parties vers un fichier final en minimisant les appels système
//!   via des tampons de 1 MiB en lecture et écriture.
use std::fs::File;
use std::path::Path;
use std::io::{self, BufReader, BufWriter, Write, Read};

/// Crée ou tronque un fichier à la taille spécifiée.
/// Utilisé pour pré‑allouer les fichiers de parties.
pub fn create_empty_file(path: &Path, size: u64) -> io::Result<File> {
    let file = File::create(path)?;
    file.set_len(size)?; // alloue l'espace sur disque
    Ok(file)
}


pub fn merge_chunks(parts: &[&Path], output: &Path) -> io::Result<()> {
    let out_file = File::create(output)?;
    // Tampon de sortie plus grand pour réduire les appels système
    let mut writer = BufWriter::with_capacity(1 << 20, out_file); // 1 MiB

    let mut buffer = vec![0u8; 1 << 20]; // 1 MiB buffer pour la lecture
    for part in parts {
        let file = File::open(part)?;
        let mut reader = BufReader::with_capacity(1 << 20, file);
        loop {
            let read_count = reader.read(&mut buffer)?;
            if read_count == 0 { break; }
            writer.write_all(&buffer[..read_count])?;
        }
    }

    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::{Read, Write};
    use tempfile::tempdir;

    #[test]
    fn test_create_empty_file_size() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty_file.bin");

        let file_size = 1024 * 1024; // 1 MB
        let file = create_empty_file(&path, file_size).unwrap();

        // Ensure file exists and has the right size
        let metadata = file.metadata().unwrap();
        assert_eq!(metadata.len(), file_size);
    }

    #[test]
    fn test_merge_two_chunks() {
        let dir = tempdir().unwrap();
        let chunk1_path = dir.path().join("chunk1.bin");
        let chunk2_path = dir.path().join("chunk2.bin");
        let output_path = dir.path().join("merged.bin");

        // Write some data to the chunks
        {
            let mut f1 = File::create(&chunk1_path).unwrap();
            f1.write_all(b"Hello ").unwrap();
            let mut f2 = File::create(&chunk2_path).unwrap();
            f2.write_all(b"World!").unwrap();
        }

        // Merge them
        merge_chunks(&[chunk1_path.as_path(), chunk2_path.as_path()], &output_path).unwrap();

        // Verify merged content
        let mut merged = String::new();
        File::open(&output_path).unwrap().read_to_string(&mut merged).unwrap();
        assert_eq!(merged, "Hello World!");
    }

    #[test]
    fn test_merge_large_chunks() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("merged_large.bin");

        // Create 3 chunks of 1 MB each
        let mut paths = vec![];
        for i in 0..3 {
            let path = dir.path().join(format!("chunk_{}.bin", i));
            let mut f = File::create(&path).unwrap();
            let data = vec![i as u8; 1024 * 1024]; // 1 MB filled with same byte
            f.write_all(&data).unwrap();
            paths.push(path);
        }

        // Merge them
        let parts: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
        merge_chunks(&parts, &output_path).unwrap();

        // Validate merged size
        let metadata = fs::metadata(&output_path).unwrap();
        assert_eq!(metadata.len(), 3 * 1024 * 1024);

        // Validate byte pattern continuity
        let mut buf = Vec::new();
        File::open(&output_path).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf[..1024 * 1024], vec![0; 1024 * 1024]);
        assert_eq!(buf[1024 * 1024..2 * 1024 * 1024], vec![1; 1024 * 1024]);
        assert_eq!(buf[2 * 1024 * 1024..], vec![2; 1024 * 1024]);
    }

    #[test]
    fn test_merge_empty_input_list() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("empty_merge.bin");

        let result = merge_chunks(&[], &output_path);
        assert!(result.is_ok(), "Merging empty list should succeed");
        assert!(output_path.exists());
        assert_eq!(fs::metadata(&output_path).unwrap().len(), 0);
    }

    #[test]
    fn test_merge_with_missing_chunk() {
        let dir = tempdir().unwrap();
        let chunk_path = dir.path().join("missing.bin");
        let output_path = dir.path().join("output.bin");

        let result = merge_chunks(&[chunk_path.as_path()], &output_path);
        assert!(result.is_err(), "Should error when chunk is missing");
    }
}
