//! Module pour capturer les requêtes réseau d'une page web.
//!
//! Utilise chromiumoxide pour lancer un navigateur Chromium et capturer
//! toutes les requêtes réseau effectuées par la page.

use anyhow::Result;
use chromiumoxide::{Browser, BrowserConfig};
use chromiumoxide_cdp::cdp::browser_protocol::network::{
    EventRequestWillBeSent, EventResponseReceived,
};
use chromiumoxide_cdp::cdp::browser_protocol::page::NavigateParams;
use futures::StreamExt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use serde::Serialize;

/// Structure représentant une entrée réseau capturée
#[derive(Clone, Debug, Serialize)]
pub struct NetworkEntry {
    pub url: String,
    pub method: Option<String>,
    pub status: Option<u16>,
    pub resource_type: Option<String>,
    pub headers: Option<String>,
    pub timestamp: f64,
}

/// Sniffer réseau qui capture toutes les requêtes d'une page
pub struct NetworkSniffer {
    filter: Option<String>,
    captured_requests: Arc<Mutex<Vec<NetworkEntry>>>,
}

impl NetworkSniffer {
    /// Crée un nouveau sniffer réseau
    pub fn new(filter: Option<String>) -> Self {
        Self {
            filter,
            captured_requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Lance le navigateur, navigue vers l'URL et capture toutes les requêtes réseau
    pub async fn sniff(&self, url: &str) -> Result<()> {
        // Réinitialiser les résultats
        {
            let mut requests = self.captured_requests.lock().unwrap();
            requests.clear();
        }

        // Configuration du navigateur
        let config = BrowserConfig::builder()
            .with_head()
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build browser config: {}", e))?;

        let (mut browser, mut handler) = Browser::launch(config).await?;

        // Gérer les événements du navigateur dans une tâche séparée
        let handler_task = tokio::spawn(async move {
            while let Some(h) = handler.next().await {
                if h.is_err() {
                    break;
                }
            }
        });

        // Obtenir une page
        let page = browser.new_page("about:blank").await?;

        // Activer le domaine Network pour capturer les requêtes
        let enable_params = chromiumoxide_cdp::cdp::browser_protocol::network::EnableParams::default();
        page.execute(enable_params).await?;

        // Cloner les références pour les handlers
        let requests_clone = self.captured_requests.clone();
        let filter_clone = self.filter.clone();

        // Naviguer vers l'URL
        let nav_params = NavigateParams::new(url);
        page.goto(nav_params).await?;

        // Attendre que la page se charge
        page.wait_for_navigation().await?;

        // Écouter les requêtes envoyées et les réponses pendant 5 secondes
        let requests_sent = requests_clone.clone();
        let filter_sent = filter_clone.clone();
        let mut request_stream = page.event_listener::<EventRequestWillBeSent>().await?;

        let requests_resp = requests_clone.clone();
        let filter_resp = filter_clone.clone();
        let mut response_stream = page.event_listener::<EventResponseReceived>().await?;

        // Écouter les événements pendant 5 secondes
        let timeout = sleep(Duration::from_secs(5));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    break;
                }
                Some(event) = request_stream.next() => {
                    let request = &event.request;
                    let url = request.url.clone();
                    
                    // Appliquer le filtre si fourni
                    if let Some(ref filter_str) = filter_sent {
                        if !url.contains(filter_str) {
                            continue;
                        }
                    }
                    
                    let entry = NetworkEntry {
                        url: url.clone(),
                        method: Some(request.method.clone()),
                        status: None,
                        resource_type: Some(format!("{:?}", event.r#type)),
                        headers: Some(format!("{:?}", request.headers)),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs_f64(),
                    };
                    
                    let mut requests_guard = requests_sent.lock().unwrap();
                    requests_guard.push(entry);
                }
                Some(event) = response_stream.next() => {
                    let response = &event.response;
                    let url = response.url.clone();
                    
                    // Appliquer le filtre si fourni
                    if let Some(ref filter_str) = filter_resp {
                        if !url.contains(filter_str) {
                            continue;
                        }
                    }
                    
                    // Mettre à jour l'entrée existante ou créer une nouvelle
                    let mut requests_guard = requests_resp.lock().unwrap();
                    
                    // Chercher une entrée existante avec cette URL
                    if let Some(entry) = requests_guard.iter_mut().find(|e| e.url == url) {
                        entry.status = Some(response.status as u16);
                    } else {
                        // Créer une nouvelle entrée si elle n'existe pas
                        let entry = NetworkEntry {
                            url: url.clone(),
                            method: None,
                            status: Some(response.status as u16),
                            resource_type: Some(format!("{:?}", event.r#type)),
                            headers: Some(format!("{:?}", response.headers)),
                            timestamp: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs_f64(),
                        };
                        requests_guard.push(entry);
                    }
                }
            }
        }

        // Exporter vers JSON
        self.export_to_json("network_output.json").await?;

        // Fermer le navigateur
        browser.close().await?;
        handler_task.abort();

        Ok(())
    }

    /// Récupère les résultats capturés
    pub async fn get_results(&self) -> Vec<NetworkEntry> {
        let requests = self.captured_requests.lock().unwrap();
        requests.clone()
    }

    /// Exporte les résultats vers un fichier JSON
    async fn export_to_json(&self, filename: &str) -> Result<()> {
        let requests = self.captured_requests.lock().unwrap();
        let json = serde_json::to_string_pretty(&*requests)?;
        tokio::fs::write(filename, json).await?;
        Ok(())
    }
}

/// Ouvre une URL dans le navigateur par défaut de l'utilisateur
///
/// # Arguments
/// * `url` - L'URL à ouvrir dans le navigateur
///
/// # Exemples
/// ```
/// use crate::sniffers::network_sniffer::open_browser;
/// open_browser("https://example.com").unwrap();
/// ```
pub fn open_browser(url: &str) -> Result<()> {
    webbrowser::open(url)?;
    Ok(())
}
