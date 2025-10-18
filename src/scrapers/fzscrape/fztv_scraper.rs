use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

/// Structure représentant une saison avec ses épisodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Season {
    pub name: String,
    pub url: String,
    pub episodes: Vec<Episode>,
}

/// Structure représentant un épisode avec ses liens de téléchargement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub name: String,
    pub download_links: Vec<DownloadLink>,
}

/// Structure représentant un lien de téléchargement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadLink {
    pub quality: String,
    pub url: String,
    pub file_id: Option<String>,
    pub dkey: Option<String>,
    pub actual_download_urls: Vec<String>,
}

/// Scraper spécialisé pour FZTV Series
pub struct FztvScraper {
    client: Client,
    base_url: String,
}

impl FztvScraper {
    /// Crée une nouvelle instance du scraper FZTV
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Impossible de créer le client HTTP");

        Self { client, base_url }
    }

    /// Scrape toutes les saisons disponibles sur la page principale
    pub async fn scrape_seasons(&self, main_url: &str) -> Result<Vec<Season>> {
        info!("Début du scraping des saisons FZTV depuis: {}", main_url);
        
        let html = self.fetch_page(main_url).await?;
        let document = Html::parse_document(&html);
        
        // Sélecteur pour les liens de saisons avec itemprop="url"
        let season_selector = Selector::parse("a[itemprop=\"url\"]")
            .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour les saisons: {}", e))?;
        
        let mut seasons = Vec::new();
        
        for element in document.select(&season_selector) {
            if let Some(href) = element.value().attr("href") {
                let name_selector = Selector::parse("span[itemprop=\"name\"]")
                    .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour le nom de saison: {}", e))?;
                
                let season_name = element
                    .select(&name_selector)
                    .next()
                    .and_then(|span| span.text().next())
                    .unwrap_or("Saison inconnue")
                    .to_string();
                
                // Construire l'URL complète de la saison
                let season_url = self.resolve_url(href)?;
                
                info!("Saison trouvée: {} -> {}", season_name, season_url);
                
                // Scraper les épisodes de cette saison
                let episodes = self.scrape_episodes(&season_url).await?;
                
                seasons.push(Season {
                    name: season_name,
                    url: season_url,
                    episodes,
                });
            }
        }
        
        info!("{} saisons FZTV trouvées", seasons.len());
        Ok(seasons)
    }

    /// Scrape tous les épisodes d'une saison donnée
    async fn scrape_episodes(&self, season_url: &str) -> Result<Vec<Episode>> {
        info!("Scraping des épisodes FZTV pour: {}", season_url);
        
        let html = self.fetch_page(season_url).await?;
        let document = Html::parse_document(&html);
        
        // Sélecteur pour les blocs d'épisodes (ul avec class contenant "list")
        let episode_block_selector = Selector::parse("ul.list")
            .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour les blocs d'épisodes: {}", e))?;
        
        let mut episodes = Vec::new();
        
        // Pour chaque bloc ul.list, extraire les informations
        for (episode_index, ul_element) in document.select(&episode_block_selector).enumerate() {
            let mut download_links = Vec::new();
            
            // Extraire le nom de l'épisode depuis le <b> tag avant le ul ou depuis le contenu
            let episode_name = self.extract_episode_name_from_block(&ul_element, &document)
                .unwrap_or_else(|| format!("Épisode {}", episode_index + 1));
            
            // Sélecteur pour les liens avec onclick dans ce bloc
            let link_selector = Selector::parse("a[onclick*=\"window.open\"]")
                .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour les liens: {}", e))?;
            
            for element in ul_element.select(&link_selector) {
                if let Some(onclick) = element.value().attr("onclick") {
                    // Extraire l'URL de téléchargement, le fileid et le dkey
                    if let Some((download_url, file_id, dkey)) = self.parse_onclick(onclick) {
                        let quality_selector = Selector::parse("small")
                            .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour la qualité: {}", e))?;
                        
                        let quality = element
                            .select(&quality_selector)
                            .next()
                            .and_then(|small| small.text().next())
                            .unwrap_or("Qualité inconnue")
                            .to_string();
                        
                        let download_link = DownloadLink {
                            quality,
                            url: download_url,
                            file_id: Some(file_id),
                            dkey,
                            actual_download_urls: Vec::new(),
                        };
                        
                        download_links.push(download_link);
                    }
                }
            }
            
            if !download_links.is_empty() {
                episodes.push(Episode {
                    name: episode_name,
                    download_links,
                });
            }
        }
        
        info!("{} épisodes FZTV trouvés pour cette saison", episodes.len());
        Ok(episodes)
    }

    /// Parse le contenu onclick pour extraire l'URL de window.location.href, le fileid et le dkey
    fn parse_onclick(&self, onclick: &str) -> Option<(String, String, Option<String>)> {
        // Rechercher l'URL dans window.location.href (c'est l'URL importante, pas window.open)
        if let Some(start) = onclick.find("window.location.href=\"") {
            let start = start + 22; // Longueur de "window.location.href=\""
            if let Some(end) = onclick[start..].find("\"") {
                let url = &onclick[start..start + end];
                
                // Rechercher le fileid dans l'URL
                if let Some(fileid_start) = onclick.find("fileid=") {
                    let fileid_start = fileid_start + 7; // Longueur de "fileid="
                    if let Some(fileid_end) = onclick[fileid_start..].find("&") {
                        let file_id = &onclick[fileid_start..fileid_start + fileid_end];
                        
                        // Rechercher le dkey
                        let dkey = if let Some(dkey_start) = onclick.find("dkey=") {
                            let dkey_start = dkey_start + 5; // Longueur de "dkey="
                            if let Some(dkey_end) = onclick[dkey_start..].find("\"") {
                                Some(onclick[dkey_start..dkey_start + dkey_end].to_string())
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        
                        return Some((url.to_string(), file_id.to_string(), dkey));
                    }
                }
            }
        }
        None
    }

    /// Scrape les URLs de téléchargement réelles depuis la page de téléchargement
    async fn scrape_download_urls(&self, download_page_url: &str) -> Result<Vec<String>> {
        info!("Scraping des URLs de téléchargement FZTV depuis: {}", download_page_url);
        
        let html = self.fetch_page(download_page_url).await?;
        let document = Html::parse_document(&html);
        
        // Si c'est une page episode.php, chercher le lien "DOWNLOAD THIS EPISODE ON YOUR DEVICE"
        if download_page_url.contains("episode.php") {
            return self.scrape_episode_page(&document).await;
        }
        
        // Si c'est une page downloadmp4.php, chercher directement les liens
        if download_page_url.contains("downloadmp4.php") {
            return self.scrape_download_page(&document).await;
        }
        
        // Sinon, essayer de scraper directement
        self.scrape_download_page(&document).await
    }

    /// Scrape une page episode.php pour trouver le lien de téléchargement
    async fn scrape_episode_page(&self, document: &Html) -> Result<Vec<String>> {
        info!("Recherche du lien dlink2 dans la page episode.php FZTV");
        
        // Chercher le lien "DOWNLOAD THIS EPISODE ON YOUR DEVICE"
        let download_link_selector = Selector::parse("a[id=\"dlink2\"]")
            .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour le lien de téléchargement: {}", e))?;
        
        let mut found_links = 0;
        for element in document.select(&download_link_selector) {
            found_links += 1;
            if let Some(href) = element.value().attr("href") {
                // Construire l'URL complète
                let full_download_url = self.resolve_url(href)?;
                info!("Lien de téléchargement FZTV trouvé: {}", full_download_url);
                
                // Naviguer vers cette page et scraper les URLs réelles
                return self.scrape_download_page_from_url(&full_download_url).await;
            }
        }
        
        info!("Aucun lien dlink2 FZTV trouvé ({} éléments trouvés)", found_links);
        Ok(Vec::new())
    }

    /// Scrape une page download.php pour extraire les URLs de téléchargement
    async fn scrape_download_page(&self, document: &Html) -> Result<Vec<String>> {
        // Sélecteur pour les divs contenant les liens de téléchargement
        let download_links_selector = Selector::parse("div.downloadlinks2")
            .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour les liens de téléchargement: {}", e))?;
        
        let mut download_urls = Vec::new();
        
        for element in document.select(&download_links_selector) {
            // Chercher les inputs avec name="filelink"
            let input_selector = Selector::parse("input[name=\"filelink\"]")
                .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour les inputs: {}", e))?;
            
            for input in element.select(&input_selector) {
                if let Some(value) = input.value().attr("value") {
                    download_urls.push(value.to_string());
                    info!("URL de téléchargement FZTV trouvée: {}", value);
                }
            }
        }
        
        info!("{} URLs de téléchargement FZTV trouvées", download_urls.len());
        Ok(download_urls)
    }

    /// Scrape une page de téléchargement depuis une URL
    async fn scrape_download_page_from_url(&self, url: &str) -> Result<Vec<String>> {
        let html = self.fetch_page(url).await?;
        let document = Html::parse_document(&html);
        self.scrape_download_page(&document).await
    }

    /// Extrait le nom de l'épisode depuis un bloc ul.list
    fn extract_episode_name_from_block(&self, _ul_element: &scraper::ElementRef, _document: &Html) -> Option<String> {
        // Essayer de trouver un élément <b> qui précède le ul ou qui est dans le parent
        // Pour l'instant, on retourne None et utilisera le numéro d'épisode par défaut
        // Cette fonction peut être améliorée selon la structure HTML spécifique
        None
    }

    /// Récupère le contenu HTML d'une page
    async fn fetch_page(&self, url: &str) -> Result<String> {
        info!("Récupération de la page FZTV: {}", url);
        
        let response = self.client
            .get(url)
            .send()
            .await
            .context("Erreur lors de la requête HTTP")?;
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Erreur HTTP: {}", response.status()));
        }
        
        let html = response.text().await
            .context("Impossible de lire le contenu de la réponse")?;
        
        Ok(html)
    }

    /// Résout une URL relative en URL absolue
    fn resolve_url(&self, url: &str) -> Result<String> {
        if url.starts_with("http://") || url.starts_with("https://") {
            Ok(url.to_string())
        } else {
            let base = Url::parse(&self.base_url)
                .context("URL de base invalide")?;
            let resolved = base.join(url)
                .context("Impossible de résoudre l'URL relative")?;
            Ok(resolved.to_string())
        }
    }

    /// Scrape toutes les données (saisons et épisodes) depuis une URL principale
    pub async fn scrape_all(&self, main_url: &str) -> Result<Vec<Season>> {
        info!("Début du scraping complet FZTV depuis: {}", main_url);
        
        let seasons = self.scrape_seasons(main_url).await?;
        
        info!("Scraping FZTV terminé. {} saisons avec un total de {} épisodes trouvés", 
              seasons.len(), 
              seasons.iter().map(|s| s.episodes.len()).sum::<usize>());
        
        Ok(seasons)
    }

    /// Scrape les liens de téléchargement réels depuis une page episode.php
    /// Cette fonction navigue vers la page episode.php et extrait le lien downloadmp4.php
    pub async fn scrape_actual_download_link(&self, episode_url: &str) -> Result<Option<String>> {
        info!("Scraping du lien de téléchargement réel depuis: {}", episode_url);
        
        // Construire l'URL complète
        let full_url = self.resolve_url(episode_url)?;
        
        // Récupérer le contenu de la page
        let html = self.fetch_page(&full_url).await?;
        let document = Html::parse_document(&html);
        
        // Chercher le div avec class="mainbox3" et le lien avec id="dlink2"
        let mainbox_selector = Selector::parse("div.mainbox3")
            .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour mainbox3: {}", e))?;
        
        let link_selector = Selector::parse("a#dlink2")
            .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour dlink2: {}", e))?;
        
        // Chercher dans les divs mainbox3
        for mainbox in document.select(&mainbox_selector) {
            // Chercher le lien dlink2
            for link in mainbox.select(&link_selector) {
                if let Some(onclick) = link.value().attr("onclick") {
                    info!("Onclick trouvé: {}", onclick);
                    
                    // Extraire l'URL de window.location.href
                    if let Some(start) = onclick.find("window.location.href=&quot;") {
                        let start = start + 27; // Longueur de "window.location.href=&quot;"
                        if let Some(end) = onclick[start..].find("&quot;") {
                            let download_url = &onclick[start..start + end];
                            info!("URL de téléchargement trouvée: {}", download_url);
                            return Ok(Some(download_url.to_string()));
                        }
                    }
                    
                    // Essayer aussi sans l'encodage HTML
                    if let Some(start) = onclick.find("window.location.href=\"") {
                        let start = start + 22; // Longueur de "window.location.href=\""
                        if let Some(end) = onclick[start..].find("\"") {
                            let download_url = &onclick[start..start + end];
                            info!("URL de téléchargement trouvée: {}", download_url);
                            return Ok(Some(download_url.to_string()));
                        }
                    }
                }
                
                // Si pas de onclick, essayer de récupérer le href directement
                if let Some(href) = link.value().attr("href") {
                    info!("Href trouvé: {}", href);
                    if href.contains("downloadmp4.php") {
                        return Ok(Some(href.to_string()));
                    }
                }
            }
        }
        
        info!("Aucun lien de téléchargement trouvé dans la page");
        Ok(None)
    }

    /// Enrichit les saisons existantes avec les liens de téléchargement réels
    /// Ne traite que le premier lien "High MP4" ou le premier lien disponible
    pub async fn enrich_with_actual_links(&self, mut seasons: Vec<Season>) -> Result<Vec<Season>> {
        info!("Début de l'enrichissement des liens de téléchargement");
        
        for season in &mut seasons {
            info!("Traitement de la saison: {}", season.name);
            
            for episode in &mut season.episodes {
                info!("Traitement de l'épisode: {}", episode.name);
                
                // Trouver l'index du premier lien "High MP4" ou prendre le premier
                let target_index = episode.download_links.iter()
                    .position(|link| link.quality.contains("High MP4"))
                    .or_else(|| {
                        if episode.download_links.is_empty() {
                            None
                        } else {
                            Some(0)
                        }
                    });
                
                if let Some(index) = target_index {
                    let link = &mut episode.download_links[index];
                    info!("Scraping du lien: {} ({})", link.url, link.quality);
                    
                    let url_to_scrape = link.url.clone();
                    
                    match self.scrape_actual_download_link(&url_to_scrape).await {
                        Ok(Some(download_url)) => {
                            info!("Lien trouvé: {}", download_url);
                            link.actual_download_urls.push(download_url);
                        }
                        Ok(None) => {
                            info!("Aucun lien trouvé pour cet épisode");
                        }
                        Err(e) => {
                            info!("Erreur lors du scraping: {}", e);
                        }
                    }
                    
                    // Attendre un peu pour ne pas surcharger le serveur
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                } else {
                    info!("Aucun lien disponible pour cet épisode");
                }
            }
        }
        
        info!("Enrichissement terminé");
        Ok(seasons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_onclick() {
        let scraper = FztvScraper::new("http://example.com".to_string());
        let onclick = r#"window.open("https://ev.notemandimpled.com/idY6kqCkfWs/kvvRE"); window.location.href="downloadmp4.php?fileid=154326&dkey=d7bf5ed1208135eee507edac13ac6d54"; return false;"#;
        
        let result = scraper.parse_onclick(onclick);
        assert!(result.is_some());
        
        let (url, file_id, dkey) = result.unwrap();
        assert_eq!(url, "downloadmp4.php?fileid=154326&dkey=d7bf5ed1208135eee507edac13ac6d54");
        assert_eq!(file_id, "154326");
        assert_eq!(dkey, Some("d7bf5ed1208135eee507edac13ac6d54".to_string()));
    }
}
