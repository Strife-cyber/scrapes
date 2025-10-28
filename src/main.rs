mod scrapers;
mod downloader;

use std::path::PathBuf;
use scrapers::fzscrape::fztv_scraper::FztvScraper;
use tracing::{info, error};

#[tokio::main]
async fn main() {
    downloader::init_logging();

    // ===== CODE DE SCRAPING INITIAL =====
    // Scraper les saisons et épisodes depuis le site
    let main_url = "https://fztvseries.live/files-Supernatural--16385.htm";
    let base_url = "https://fztvseries.live";
    
    let scraper = FztvScraper::new(base_url.to_string());
    
    match scraper.scrape_all(main_url).await {
        Ok(seasons) => {
            info!("Scraping réussi ! {} saisons trouvées", seasons.len());
            
            for season in &seasons {
                println!("\n=== {} ===", season.name);
                println!("URL: {}", season.url);
                println!("Épisodes: {}", season.episodes.len());
                
                for episode in &season.episodes {
                    println!("  - {}", episode.name);
                    println!("    Liens de téléchargement: {}", episode.download_links.len());
                }
            }
            
            // Sauvegarder les résultats en JSON
            if let Ok(json) = serde_json::to_string_pretty(&seasons) {
                if let Err(e) = tokio::fs::write("scraped_data.json", json).await {
                    error!("Erreur lors de la sauvegarde: {}", e);
                } else {
                    info!("Données sauvegardées dans scraped_data.json");
                }
            }
        }
        Err(e) => {
            error!("Erreur lors du scraping: {:#}", e);
        }
    }
    
    
    // ===== CODE D'ENRICHISSEMENT DES LIENS DE TÉLÉCHARGEMENT (COMMENTÉ) =====
    // On charge le JSON existant et on enrichit avec les liens de téléchargement réels
    
    let base_url = "https://fztvseries.live";
    let scraper = FztvScraper::new(base_url.to_string());
    
    info!("Chargement du fichier scraped_data.json...");
    
    match tokio::fs::read_to_string("scraped_data.json").await {
        Ok(json_content) => {
            match serde_json::from_str(&json_content) {
                Ok(seasons) => {
                    info!("Fichier chargé avec succès");
                    
                    // Enrichir les saisons avec les liens de téléchargement réels
                    match scraper.enrich_with_actual_links(seasons).await {
                        Ok(enriched_seasons) => {
                            info!("Enrichissement terminé !");
                            
                            // Afficher les résultats
                            for season in &enriched_seasons {
                                println!("\n=== {} ===", season.name);
                                
                                for episode in &season.episodes {
                                    println!("  - {}", episode.name);
                                    for link in &episode.download_links {
                                        if !link.actual_download_urls.is_empty() {
                                            println!("    {}: {}", link.quality, link.url);
                                            println!("      URLs de téléchargement trouvées:");
                                            for (i, download_url) in link.actual_download_urls.iter().enumerate() {
                                                println!("        {}: {}", i + 1, download_url);
                                            }
                                        }
                                    }
                                }
                            }
                            
                            // Sauvegarder les résultats enrichis
                            if let Ok(json) = serde_json::to_string_pretty(&enriched_seasons) {
                                if let Err(e) = tokio::fs::write("scraped_data_enriched.json", json).await {
                                    error!("Erreur lors de la sauvegarde: {}", e);
                                } else {
                                    info!("Données enrichies sauvegardées dans scraped_data_enriched.json");
                                }
                            }
                        }
                        Err(e) => {
                            error!("Erreur lors de l'enrichissement: {:#}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Erreur lors du parsing du JSON: {:#}", e);
                }
            }
        }
        Err(e) => {
            error!("Erreur lors de la lecture du fichier: {:#}", e);
        }
    }
    
    // Exemple de téléchargement d'un fichier (code original conservé)
    /*let url = "http://uf3qp40g0f.y9a8ua48mhss5ye.cyou/rlink_t/432ca7c011673093e49aa5bfc7f33306/064ae025056c5ed2c33110c7e55c6440/a4af1a23ba939683bc363e4c3880b59c/Supernatural_-_S01E07_-_Hook_Man_35e16d46f3186a38e3ce876d4435e61b.mp4".to_string();
    let output: PathBuf = "D:\\Shows\\Supernatural\\supernatural_s1_ep_7.mp4".into();

    if let Err(e) = downloader::download_to(url, output).await {
        eprintln!("Erreur de téléchargement: {:#}", e);
    }*/
}