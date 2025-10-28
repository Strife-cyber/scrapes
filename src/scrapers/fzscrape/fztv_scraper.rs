use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;
use tokio::sync::Semaphore;
use std::sync::Arc;
use futures::stream::{self, StreamExt};

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
    // Semaphore pour limiter les requêtes concurrentes
    semaphore: Arc<Semaphore>,
}

impl FztvScraper {
    /// Crée une nouvelle instance du scraper FZTV
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Impossible de créer le client HTTP");

        // Limite à 10 requêtes concurrentes pour ne pas surcharger le serveur
        let semaphore = Arc::new(Semaphore::new(10));

        Self { client, base_url, semaphore }
    }

    /// Scrape toutes les saisons disponibles sur la page principale
    pub async fn scrape_seasons(&self, main_url: &str) -> Result<Vec<Season>> {
        info!("Début du scraping des saisons FZTV depuis: {}", main_url);
        
        let html = self.fetch_page(main_url).await?;
        let document = Html::parse_document(&html);
        
        // Sélecteur pour les liens de saisons avec itemprop="url"
        let season_selector = Selector::parse("a[itemprop=\"url\"]")
            .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour les saisons: {}", e))?;
        
        // Collecter toutes les infos de saisons d'abord
        let mut season_infos = Vec::new();
        
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
                season_infos.push((season_name, season_url));
            }
        }
        
        // Scraper toutes les saisons en parallèle avec contrôle de concurrence
        let seasons = stream::iter(season_infos)
            .map(|(name, url)| async move {
                let episodes = self.scrape_episodes(&url).await.ok()?;
                Some(Season {
                    name,
                    url,
                    episodes,
                })
            })
            .buffer_unordered(10)  // Traiter jusqu'à 10 saisons en parallèle
            .filter_map(|x| async { x })
            .collect::<Vec<_>>()
            .await;
        
        info!("{} saisons FZTV trouvées", seasons.len());
        Ok(seasons)
    }

    /// Scrape tous les épisodes d'une saison donnée
    async fn scrape_episodes(&self, season_url: &str) -> Result<Vec<Episode>> {
        info!("Scraping des épisodes FZTV pour: {}", season_url);
        
        let html = self.fetch_page(season_url).await?;
        let document = Html::parse_document(&html);
        
        // Debug: Afficher une partie du HTML pour comprendre la structure
        self.debug_html_structure(&document, season_url).await?;
        
        // Essayer différents sélecteurs pour trouver les épisodes
        let mut episodes = Vec::new();
        
        // Sélecteur 1: ul.list (original)
        if let Ok(selector) = Selector::parse("ul.list") {
            episodes.extend(self.scrape_episodes_with_selector(&document, &selector, "ul.list").await?);
        }
        
        // Sélecteur 2: div avec class contenant "episode" ou "list"
        if episodes.is_empty() {
            if let Ok(selector) = Selector::parse("div[class*=\"episode\"], div[class*=\"list\"]") {
                episodes.extend(self.scrape_episodes_with_selector(&document, &selector, "div.episode/list").await?);
            }
        }
        
        // Sélecteur 3: table ou tr pour les épisodes
        if episodes.is_empty() {
            if let Ok(selector) = Selector::parse("table tr, tr") {
                episodes.extend(self.scrape_episodes_with_selector(&document, &selector, "table tr").await?);
            }
        }
        
        // Sélecteur 4: liens avec onclick contenant "episode"
        if episodes.is_empty() {
            if let Ok(selector) = Selector::parse("a[onclick*=\"episode\"]") {
                episodes.extend(self.scrape_episodes_with_selector(&document, &selector, "a[onclick*=\"episode\"]").await?);
            }
        }
        
        info!("{} épisodes FZTV trouvés pour cette saison", episodes.len());
        Ok(episodes)
    }
    
    /// Debug function pour examiner la structure HTML
    async fn debug_html_structure(&self, document: &Html, season_url: &str) -> Result<()> {
        info!("=== DEBUG HTML STRUCTURE pour {} ===", season_url);
        
        // Chercher tous les éléments qui pourraient contenir des épisodes
        let debug_selectors = vec![
            "ul", "div", "table", "tr", "li",
            "a[onclick]", "a[href*=\"episode\"]", "a[href*=\"download\"]",
            "[class*=\"episode\"]", "[class*=\"list\"]", "[class*=\"download\"]"
        ];
        
        for selector_str in debug_selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                let count = document.select(&selector).count();
                if count > 0 {
                    info!("Sélecteur '{}': {} éléments trouvés", selector_str, count);
                    
                    // Afficher les premiers éléments pour comprendre la structure
                    for (i, element) in document.select(&selector).enumerate() {
                        if i >= 3 { break; } // Limiter à 3 exemples
                        let text = element.text().collect::<String>().trim().to_string();
                        let text_preview = if text.len() > 100 { 
                            format!("{}...", &text[..100]) 
                        } else { 
                            text 
                        };
                        info!("  Exemple {}: {}", i + 1, text_preview);
                        
                        // Afficher les attributs importants
                        if let Some(onclick) = element.value().attr("onclick") {
                            info!("    onclick: {}", onclick);
                        }
                        if let Some(href) = element.value().attr("href") {
                            info!("    href: {}", href);
                        }
                        if let Some(class) = element.value().attr("class") {
                            info!("    class: {}", class);
                        }
                    }
                }
            }
        }
        
        info!("=== FIN DEBUG HTML STRUCTURE ===");
        Ok(())
    }
    
    /// Scrape les épisodes avec un sélecteur spécifique
    async fn scrape_episodes_with_selector(&self, document: &Html, selector: &Selector, selector_name: &str) -> Result<Vec<Episode>> {
        let mut episodes = Vec::new();
        
        info!("Tentative de scraping avec le sélecteur: {}", selector_name);
        
        for (episode_index, element) in document.select(selector).enumerate() {
            let mut download_links = Vec::new();
            
            // Essayer d'extraire le nom de l'épisode
            let episode_name = self.extract_episode_name_from_element(&element, episode_index);
            
            // Chercher les liens de téléchargement dans cet élément
            let link_selector = Selector::parse("a[onclick*=\"window.open\"], a[onclick*=\"episode\"], a[href*=\"download\"]")
                .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour les liens: {}", e))?;
            
            for link_element in element.select(&link_selector) {
                if let Some(onclick) = link_element.value().attr("onclick") {
                    // Extraire l'URL de téléchargement, le fileid et le dkey
                    if let Some((download_url, file_id, dkey)) = self.parse_onclick(onclick) {
                        let quality_selector = Selector::parse("small, span, b")
                            .map_err(|e| anyhow::anyhow!("Impossible de créer le sélecteur pour la qualité: {}", e))?;
                        
                        let quality = link_element
                            .select(&quality_selector)
                            .next()
                            .and_then(|elem| elem.text().next())
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
                
                // Essayer aussi avec href direct
                if let Some(href) = link_element.value().attr("href") {
                    if href.contains("episode") || href.contains("download") {
                        let quality = "Direct Link".to_string();
                        let download_link = DownloadLink {
                            quality,
                            url: href.to_string(),
                            file_id: None,
                            dkey: None,
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
        
        info!("Sélecteur '{}': {} épisodes trouvés", selector_name, episodes.len());
        Ok(episodes)
    }
    
    /// Extrait le nom de l'épisode depuis un élément
    fn extract_episode_name_from_element(&self, element: &scraper::ElementRef, episode_index: usize) -> String {
        // Essayer de trouver du texte dans l'élément ou ses enfants
        let text = element.text().collect::<String>().trim().to_string();
        
        if !text.is_empty() && text.len() > 3 {
            // Prendre les premiers mots comme nom d'épisode
            let words: Vec<&str> = text.split_whitespace().take(5).collect();
            if !words.is_empty() {
                return words.join(" ");
            }
        }
        
        // Fallback: chercher dans les éléments enfants
        let name_selectors = vec!["b", "strong", "span", "div", "a"];
        for selector_str in name_selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                if let Some(name_elem) = element.select(&selector).next() {
                    let name_text = name_elem.text().collect::<String>().trim().to_string();
                    if !name_text.is_empty() && name_text.len() > 3 {
                        return name_text;
                    }
                }
            }
        }
        
        // Dernier recours: nom générique
        format!("Épisode {}", episode_index + 1)
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


    /// Récupère le contenu HTML d'une page
    async fn fetch_page(&self, url: &str) -> Result<String> {
        info!("Récupération de la page FZTV: {}", url);
        
        // Acquérir le semaphore pour limiter les requêtes concurrentes
        let _permit = self.semaphore
            .acquire()
            .await
            .map_err(|e| anyhow::anyhow!("Erreur d'acquisition du semaphore: {}", e))?;
        
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
    pub async fn enrich_with_actual_links(&self, seasons: Vec<Season>) -> Result<Vec<Season>> {
        info!("Début de l'enrichissement des liens de téléchargement");
        
        // Créer une liste de toutes les tâches à traiter (season_idx, episode_idx, url, quality)
        let mut tasks = Vec::new();
        
        for (season_idx, season) in seasons.iter().enumerate() {
            for (episode_idx, episode) in season.episodes.iter().enumerate() {
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
                
                if let Some(link_idx) = target_index {
                    let link = &episode.download_links[link_idx];
                    tasks.push((
                        season_idx,
                        episode_idx,
                        link_idx,
                        link.url.clone(),
                        episode.name.clone(),
                    ));
                }
            }
        }
        
        info!("Traitement de {} liens en parallèle", tasks.len());
        
        // Traiter toutes les tâches en parallèle avec limitation de concurrence
        let results: Vec<_> = stream::iter(tasks)
            .map(|(season_idx, episode_idx, link_idx, url, episode_name)| async move {
                info!("Scraping du lien pour l'épisode: {}", episode_name);
                
                match self.scrape_actual_download_link(&url).await {
                    Ok(Some(download_url)) => {
                        info!("Lien trouvé pour {}: {}", episode_name, download_url);
                        Some((season_idx, episode_idx, link_idx, download_url))
                    }
                    Ok(None) => {
                        info!("Aucun lien trouvé pour {}", episode_name);
                        None
                    }
                    Err(e) => {
                        info!("Erreur lors du scraping de {}: {}", episode_name, e);
                        None
                    }
                }
            })
            .buffer_unordered(20)  // Traiter jusqu'à 20 liens en parallèle (le semaphore dans fetch_page limite à 10 requêtes réelles)
            .filter_map(|x| async { x })
            .collect()
            .await;
        
        // Appliquer les résultats aux saisons
        let mut enriched_seasons = seasons;
        for (season_idx, episode_idx, link_idx, download_url) in results {
            if let Some(season) = enriched_seasons.get_mut(season_idx) {
                if let Some(episode) = season.episodes.get_mut(episode_idx) {
                    if let Some(link) = episode.download_links.get_mut(link_idx) {
                        link.actual_download_urls.push(download_url);
                    }
                }
            }
        }
        
        info!("Enrichissement terminé");
        Ok(enriched_seasons)
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
