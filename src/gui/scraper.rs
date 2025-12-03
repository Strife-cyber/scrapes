//! Composant UI pour le scraper FZTV.
//!
//! Permet de:
//! - Saisir l'URL de base et l'URL de la série
//! - Lancer le scraping des saisons/épisodes
//! - Visualiser les résultats avec les liens de téléchargement

use egui::{Ui, RichText, Color32};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Onglet du scraper FZTV
pub struct ScraperTab {
    base_url: String,
    series_url: String,
    is_scraping: bool,
    results: Arc<Mutex<Vec<String>>>, // Pour l'instant, juste des messages
}

impl Default for ScraperTab {
    fn default() -> Self {
        Self {
            base_url: "https://www.fztvseries.mobi/".to_string(),
            series_url: String::new(),
            is_scraping: false,
            results: Arc::new(Mutex::new(Vec::new())),
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
                    let results_guard = self.results.blocking_lock();
                    let results = results_guard.clone();
                    drop(results_guard);
                    
                    if results.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(RichText::new("📭 Aucun résultat").size(18.0).color(Color32::GRAY));
                            ui.label(RichText::new("Les saisons et épisodes avec leurs liens de téléchargement apparaîtront ici")
                                .color(Color32::DARK_GRAY));
                        });
                    } else {
                        for result in results {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(25, 25, 30))
                                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 50, 60)))
                                .rounding(egui::Rounding::same(6.0))
                                .inner_margin(egui::Margin::same(12.0))
                                .show(ui, |ui| {
                                    ui.label(result);
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
        // TODO: Intégrer avec le scraper réel
    }
}

