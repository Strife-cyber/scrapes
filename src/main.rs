//! Exemple minimal d'utilisation du module `downloader`.
//!
//! Étapes illustrées:
//! 1. Création d'une `DownloadTask` avec l'URL, le chemin de sortie et la taille.
//! 2. Préparation des segments et des fichiers temporaires via `DownloadManager`.
//! 3. Affichage de la liste des segments préparés.
mod downloader;

use downloader::*;

#[tokio::main]
async fn main() {
    let task = DownloadTask {
        url: "https://example.com/file.zip".to_string(),
        output: "file.zip".into(),
        total_size: 50_000_000,
        chunk_size: 10_000_000,
        num_chunks: 0, // non utilisé pour l'instant
    };

    let manager = DownloadManager::new();
    let chunks = manager.prepare(&task).expect("prepare failed");

    println!("Prepared {} chunks:", chunks.len());
    for c in &chunks {
        println!("{:?}", c);
    }

    // Démarrer le téléchargement réel
    if let Err(e) = manager.start(task).await {
        eprintln!("Erreur de téléchargement: {:#}", e);
    }
}