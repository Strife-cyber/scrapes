mod downloader;

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    downloader::init_logging();

    let url = "http://uf3qp40g0f.y9a8ua48mhss5ye.cyou/rlink_t/f5b538952b361f2505dc1f114a13ff5b/579e40fe536ebff55b7c673e5a067ecb/a4af1a23ba939683bc363e4c3880b59c/The_Lincoln_Lawyer_-_S01E01_-_Unknown_7da1ae289ae704453cd1f96ddc902538.mp4".to_string();
    let output: PathBuf = "lincoln_lawyer_s1_ep_1.mp4".into();

    if let Err(e) = downloader::download_to(url, output).await {
        eprintln!("Erreur de téléchargement: {:#}", e);
    }
}