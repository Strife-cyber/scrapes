//! Composant UI pour le scraper FZTV.
//!
//! Permet de:
//! - Saisir l'URL de base et l'URL de la série
//! - Lancer le scraping des saisons/épisodes
//! - Visualiser les résultats avec les liens de téléchargement

use egui::{Ui, RichText, Color32};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tokio::sync::Mutex;
use crate::scrapers::{FztvScraper, Season};

/// Onglet du scraper FZTV
pub struct ScraperTab {
    base_url: String,
    series_url: String,
    is_scraping: bool,
    cancel_flag: Arc<AtomicBool>,
    results: Arc<Mutex<Vec<Season>>>,
    error_message: Arc<Mutex<Option<String>>>,
    task_handle: Option<std::thread::JoinHandle<()>>,
}

impl Default for ScraperTab {
    fn default() -> Self {
        Self {
            base_url: "https://www.fztvseries.mobi/".to_string(),
            series_url: String::new(),
            is_scraping: false,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            results: Arc::new(Mutex::new(Vec::new())),
            error_message: Arc::new(Mutex::new(None)),
            task_handle: None,
        }
    }
}

impl ScraperTab {
    pub fn show(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.heading("🔍 Scraper FZTV");
            ui.separator();
            
            // Configuration avec style amélioré
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(30, 30, 35))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(60, 60, 70)))
                .rounding(egui::Rounding::same(8.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.heading("⚙️ Configuration");
                    ui.add_space(8.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("URL de base:").strong());
                        ui.text_edit_singleline(&mut self.base_url);
                    });
                    
                    ui.add_space(4.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("URL de la série:").strong());
                        ui.text_edit_singleline(&mut self.series_url)
                            .on_hover_text("URL complète de la page de la série");
                    });
                    
                    ui.add_space(12.0);
                    
                    ui.horizontal(|ui| {
                        let button_enabled = !self.series_url.is_empty() && !self.is_scraping;
                        if ui.add_enabled(button_enabled, egui::Button::new(RichText::new("🔍 Lancer le scraping").size(14.0)))
                            .clicked() {
                            self.start_scraping();
                        }
                        
                        if self.is_scraping {
                            if ui.button(RichText::new("⏹️ Arrêter").size(14.0).color(Color32::from_rgb(255, 100, 100)))
                                .clicked() {
                                self.stop_scraping();
                            }
                            ui.spinner();
                            ui.label(RichText::new("Scraping en cours...").color(Color32::YELLOW));
                        }
                    });
                });
            
            ui.add_space(12.0);
            
            // Résultats avec scroll
            ui.heading("📋 Résultats");
            ui.add_space(4.0);
            
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    // Utiliser try_lock pour ne pas bloquer le thread UI
                    let results = match self.results.try_lock() {
                        Ok(guard) => guard.clone(),
                        Err(_) => Vec::new(), // Si on ne peut pas acquérir le lock, utiliser des données vides
                    };
                    
                    // Afficher les erreurs (non-bloquant)
                    if let Ok(error_guard) = self.error_message.try_lock() {
                        if let Some(ref error) = *error_guard {
                            ui.label(RichText::new(format!("❌ Erreur: {}", error))
                                .color(Color32::from_rgb(255, 100, 100)));
                            ui.add_space(8.0);
                        }
                    }
                    
                    if results.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(RichText::new("📭 Aucun résultat").size(18.0).color(Color32::GRAY));
                            ui.label(RichText::new("Les saisons et épisodes avec leurs liens de téléchargement apparaîtront ici")
                                .color(Color32::DARK_GRAY));
                        });
                    } else {
                        ui.label(RichText::new(format!("{} saison(s) trouvée(s)", results.len()))
                            .color(Color32::GRAY)
                            .small());
                        ui.add_space(4.0);
                        
                        for season in results {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(25, 25, 30))
                                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 50, 60)))
                                .rounding(egui::Rounding::same(6.0))
                                .inner_margin(egui::Margin::same(12.0))
                                .show(ui, |ui| {
                                    ui.label(RichText::new(&season.name).strong());
                                    ui.label(RichText::new(format!("{} épisode(s)", season.episodes.len()))
                                        .small()
                                        .color(Color32::GRAY));
                                    
                                    if !season.episodes.is_empty() {
                                        ui.collapsing("Épisodes", |ui| {
                                            for episode in &season.episodes {
                                                ui.label(RichText::new(&episode.name).small());
                                                if !episode.download_links.is_empty() {
                                                    ui.indent("links", |ui| {
                                                        for link in &episode.download_links {
                                                            ui.label(RichText::new(format!("{}: {}", link.quality, link.url))
                                                                .small()
                                                                .color(Color32::from_rgb(100, 200, 255)));
                                                        }
                                                    });
                                                }
                                            }
                                        });
                                    }
                                });
                            ui.add_space(4.0);
                        }
                    }
                });
        });
    }
    
    fn start_scraping(&mut self) {
        if self.series_url.is_empty() {
            return;
        }
        
        self.is_scraping = true;
        self.cancel_flag.store(false, Ordering::Relaxed);
        
        // Réinitialiser les résultats
        let results = self.results.clone();
        let error_msg = self.error_message.clone();
        let cancel_flag = self.cancel_flag.clone();
        let base_url = self.base_url.clone();
        let series_url = self.series_url.clone();
        
        // Lancer le scraping dans un thread séparé
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
            rt.block_on(async move {
                let scraper = FztvScraper::new(base_url);
                
                // Vérifier le flag d'annulation périodiquement
                let result = if cancel_flag.load(Ordering::Relaxed) {
                    Err(anyhow::anyhow!("Annulé par l'utilisateur"))
                } else {
                    scraper.scrape_all(&series_url).await
                };
                
                match result {
                    Ok(seasons) => {
                        let mut guard = results.blocking_lock();
                        *guard = seasons;
                        drop(guard);
                    }
                    Err(e) => {
                        let mut guard = error_msg.blocking_lock();
                        *guard = Some(e.to_string());
                    }
                }
            });
        });
        
        self.task_handle = Some(handle);
    }
    
    fn stop_scraping(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
        self.is_scraping = false;
        
        // Attendre que le thread se termine
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.join();
        }
    }
}

