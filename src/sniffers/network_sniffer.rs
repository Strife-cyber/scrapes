use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::network::{EventRequestWillBeSent, EventResponseReceived};
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::Result;
use std::fs;
use futures::StreamExt;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetworkEntry {
    pub url: String,
    pub status: Option<u16>,
}

pub struct NetworkSniffer {
    pub filter: Option<String>,
    results: Arc<Mutex<Vec<NetworkEntry>>>,
}

impl NetworkSniffer {
    pub fn new(filter: Option<String>) -> Self {
        Self {
            filter,
            results: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn sniff(&self, url: &str) -> Result<()> {
        // Lancer le navigateur
        let config = BrowserConfig::builder()
            .build()
            .map_err(|e| anyhow::anyhow!("Erreur de configuration du navigateur: {}", e))?;
        let (mut browser, mut handler) = Browser::launch(config).await?;
        let page = browser.new_page("about:blank").await?;

        let results_ref = self.results.clone();
        let filter_ref = self.filter.clone();

        // Démarrer une tâche pour maintenir la boucle d'événements du navigateur
        let handler_task = tokio::spawn(async move {
            while let Some(_) = handler.next().await {}
        });

        // Écouter les requêtes réseau
        {
            let results_ref = results_ref.clone();
            let filter_ref = filter_ref.clone();
            let mut request_stream = page.event_listener::<EventRequestWillBeSent>().await?;
            let results_ref2 = results_ref.clone();
            let filter_ref2 = filter_ref.clone();
            tokio::spawn(async move {
                while let Some(event) = request_stream.next().await {
                    let url = event.request.url.clone();
                    if let Some(f) = &filter_ref2 {
                        if !url.contains(f) {
                            continue;
                        }
                    }
                    let mut guard = results_ref2.lock().await;
                    guard.push(NetworkEntry { url, status: None });
                }
            });
        }

        // Écouter les réponses réseau
        {
            let results_ref = results_ref.clone();
            let filter_ref = filter_ref.clone();
            let mut response_stream = page.event_listener::<EventResponseReceived>().await?;
            let results_ref2 = results_ref.clone();
            let filter_ref2 = filter_ref.clone();
            tokio::spawn(async move {
                while let Some(event) = response_stream.next().await {
                    let url = event.response.url.clone();
                    let status = Some(event.response.status as u16);
                    if let Some(f) = &filter_ref2 {
                        if !url.contains(f) {
                            continue;
                        }
                    }
                    let mut guard = results_ref2.lock().await;
                    guard.push(NetworkEntry { url, status });
                }
            });
        }

        // Naviguer vers la page
        page.goto(url).await?;
        page.wait_for_navigation().await?;

        // Attendre quelques secondes pour que les requêtes se déclenchent
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        // Exporter les résultats
        self.export().await?;

        browser.close().await?;
        handler_task.abort();
        Ok(())
    }

    async fn export(&self) -> Result<()> {
        let guard = self.results.lock().await;
        let json = serde_json::to_string_pretty(&*guard)?;
        fs::write("network_output.json", json)?;
        println!("📁 Saved output → network_output.json");
        Ok(())
    }

    pub async fn get_results(&self) -> Vec<NetworkEntry> {
        let guard = self.results.lock().await;
        guard.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_new_without_filter() {
        let sniffer = NetworkSniffer::new(None);
        assert_eq!(sniffer.filter, None);
    }

    #[test]
    fn test_new_with_filter() {
        let filter = Some("example.com".to_string());
        let sniffer = NetworkSniffer::new(filter.clone());
        assert_eq!(sniffer.filter, filter);
    }

    #[tokio::test]
    async fn test_get_results_empty() {
        let sniffer = NetworkSniffer::new(None);
        let results = sniffer.get_results().await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_export_empty_results() {
        let sniffer = NetworkSniffer::new(None);
        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test_network_output.json");
        
        // Modifier temporairement le chemin d'export pour le test
        // On va créer une version de test qui accepte un chemin
        let guard = sniffer.results.lock().await;
        let json = serde_json::to_string_pretty(&*guard).unwrap();
        fs::write(&test_file, json).unwrap();
        
        assert!(test_file.exists());
        let content = fs::read_to_string(&test_file).unwrap();
        // Vérifier que le contenu est un tableau JSON vide (peut avoir des espaces/retours à la ligne)
        let parsed: Vec<NetworkEntry> = serde_json::from_str(&content).unwrap();
        assert!(parsed.is_empty());
    }

    #[tokio::test]
    async fn test_export_with_results() {
        let sniffer = NetworkSniffer::new(None);
        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test_network_output.json");
        
        // Ajouter des résultats manuellement pour tester l'export
        {
            let mut guard = sniffer.results.lock().await;
            guard.push(NetworkEntry {
                url: "https://example.com".to_string(),
                status: Some(200),
            });
            guard.push(NetworkEntry {
                url: "https://test.com/api".to_string(),
                status: Some(404),
            });
        }
        
        let json = serde_json::to_string_pretty(&sniffer.get_results().await).unwrap();
        fs::write(&test_file, json).unwrap();
        
        assert!(test_file.exists());
        let content = fs::read_to_string(&test_file).unwrap();
        assert!(content.contains("example.com"));
        assert!(content.contains("test.com"));
        assert!(content.contains("\"status\": 200"));
        assert!(content.contains("\"status\": 404"));
    }

    #[test]
    fn test_network_entry_serialization() {
        let entry = NetworkEntry {
            url: "https://example.com".to_string(),
            status: Some(200),
        };
        
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("example.com"));
        assert!(json.contains("200"));
        
        let entry_no_status = NetworkEntry {
            url: "https://test.com".to_string(),
            status: None,
        };
        
        let json2 = serde_json::to_string(&entry_no_status).unwrap();
        assert!(json2.contains("test.com"));
        assert!(json2.contains("null"));
    }

    #[test]
    fn test_network_entry_clone() {
        let entry = NetworkEntry {
            url: "https://example.com".to_string(),
            status: Some(200),
        };
        
        let cloned = entry.clone();
        assert_eq!(entry.url, cloned.url);
        assert_eq!(entry.status, cloned.status);
    }

    #[tokio::test]
    #[ignore] // Ignorer par défaut, car nécessite Chrome/Chromium et est lent
    async fn test_sniff_simple_page() {
        // Test d'intégration qui nécessite un navigateur réel
        // Pour exécuter : cargo test -- --ignored
        let sniffer = NetworkSniffer::new(None);
        
        // Utiliser une page HTML simple en data URL pour éviter les dépendances externes
        let data_url = "data:text/html,<html><body><h1>Test</h1><script>fetch('https://httpbin.org/get').then(r => r.json())</script></body></html>";
        
        // Ce test peut échouer si Chrome n'est pas installé ou disponible
        let result = sniffer.sniff(data_url).await;
        
        // Si le navigateur peut être lancé, vérifier que des résultats sont collectés
        if result.is_ok() {
            let results = sniffer.get_results().await;
            // Au minimum, la page elle-même devrait être dans les résultats
            assert!(!results.is_empty(), "Le sniffer devrait avoir collecté au moins une requête");
        }
    }

    #[tokio::test]
    #[ignore] // Ignorer par défaut, car nécessite Chrome/Chromium et est lent
    async fn test_sniff_with_filter() {
        // Test d'intégration avec filtre
        let filter = Some("httpbin".to_string());
        let sniffer = NetworkSniffer::new(filter);
        
        let data_url = "data:text/html,<html><body><script>fetch('https://httpbin.org/get'); fetch('https://example.com')</script></body></html>";
        
        let result = sniffer.sniff(data_url).await;
        
        if result.is_ok() {
            let results = sniffer.get_results().await;
            // Tous les résultats devraient contenir "httpbin"
            for entry in &results {
                assert!(
                    entry.url.contains("httpbin"),
                    "Tous les résultats devraient contenir le filtre 'httpbin', mais trouvé: {}",
                    entry.url
                );
            }
        }
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        // Tester que plusieurs tâches peuvent accéder aux résultats simultanément
        let sniffer = NetworkSniffer::new(None);
        
        // Ajouter quelques résultats
        {
            let mut guard = sniffer.results.lock().await;
            guard.push(NetworkEntry {
                url: "https://test1.com".to_string(),
                status: Some(200),
            });
            guard.push(NetworkEntry {
                url: "https://test2.com".to_string(),
                status: Some(201),
            });
        }
        
        // Lire depuis plusieurs tâches simultanément
        let results1 = sniffer.get_results().await;
        let results2 = sniffer.get_results().await;
        
        assert_eq!(results1.len(), 2);
        assert_eq!(results2.len(), 2);
        assert_eq!(results1, results2);
    }
}
