//! Exemple minimal d'utilisation du module `downloader`.
//!
//! Étapes illustrées:
//! 1. Télécharger une URL vers un fichier local via l'API publique `download_to`.
mod downloader;

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let url = "https://example.com/file.zip".to_string();
    let output: PathBuf = "file.zip".into();

    if let Err(e) = downloader::download_to(url, output).await {
        eprintln!("Erreur de téléchargement: {:#}", e);
    }
}