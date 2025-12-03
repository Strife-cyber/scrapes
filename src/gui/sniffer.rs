//! Composant UI pour le sniffer réseau.
//!
//! Permet de:
//! - Saisir l'URL à sniffer
//! - Configurer le filtre optionnel
//! - Visualiser les requêtes capturées en temps réel

use egui::{Ui, RichText, Color32, ScrollArea};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Onglet du sniffer réseau
pub struct SnifferTab {
    target_url: String,
    filter: String,
    is_sniffing: bool,
    captured_requests: Arc<Mutex<Vec<NetworkRequest>>>,
}

#[derive(Clone, Debug)]
struct NetworkRequest {
    url: String,
    status: Option<u16>,
}

impl Default for SnifferTab {
    fn default() -> Self {
        Self {
            target_url: String::new(),
            filter: String::new(),
            is_sniffing: false,
            captured_requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl SnifferTab {
    pub fn show(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.heading("🌐 Sniffer Réseau");
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
                        ui.label(RichText::new("URL à sniffer:").strong());
                        ui.text_edit_singleline(&mut self.target_url)
                            .on_hover_text("URL de la page à analyser");
                    });
                    
                    ui.add_space(4.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Filtre (optionnel):").strong());
                        ui.text_edit_singleline(&mut self.filter)
                            .on_hover_text("Filtrer les requêtes (ex: 'm3u8', 'mp4')");
                    });
                    
                    ui.add_space(12.0);
                    
                    ui.horizontal(|ui| {
                        let button_enabled = !self.target_url.is_empty() && !self.is_sniffing;
                        if ui.add_enabled(button_enabled, egui::Button::new(RichText::new("🌐 Démarrer le sniffing").size(14.0)))
                            .clicked() {
                            self.start_sniffing();
                        }
                        
                        if self.is_sniffing {
                            ui.spinner();
                            ui.label(RichText::new("Sniffing en cours...").color(Color32::YELLOW));
                        }
                    });
                });
            
            ui.add_space(12.0);
            
            // Requêtes capturées
            ui.heading("📋 Requêtes Capturées");
            ui.add_space(4.0);
            
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let requests_guard = self.captured_requests.blocking_lock();
                    let requests = requests_guard.clone();
                    drop(requests_guard);
                    
                    if requests.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(RichText::new("📭 Aucune requête capturée").size(18.0).color(Color32::GRAY));
                            ui.label(RichText::new("Les requêtes réseau apparaîtront ici lors du sniffing")
                                .color(Color32::DARK_GRAY));
                        });
                    } else {
                        ui.label(RichText::new(format!("{} requête(s) capturée(s)", requests.len()))
                            .color(Color32::GRAY)
                            .small());
                        ui.add_space(4.0);
                        
                        for (_idx, request) in requests.iter().enumerate() {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(25, 25, 30))
                                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 50, 60)))
                                .rounding(egui::Rounding::same(6.0))
                                .inner_margin(egui::Margin::same(12.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        if let Some(status) = request.status {
                                            let status_color = if status >= 200 && status < 300 {
                                                Color32::from_rgb(100, 255, 100)
                                            } else if status >= 300 && status < 400 {
                                                Color32::from_rgb(255, 200, 100)
                                            } else {
                                                Color32::from_rgb(255, 100, 100)
                                            };
                                            ui.label(RichText::new(format!("[{}]", status))
                                                .color(status_color)
                                                .strong());
                                        }
                                        ui.label(RichText::new(&request.url).small());
                                    });
                                });
                            ui.add_space(4.0);
                        }
                    }
                });
        });
    }
    
    fn start_sniffing(&mut self) {
        if self.target_url.is_empty() {
            return;
        }
        
        self.is_sniffing = true;
        // TODO: Intégrer avec le sniffer réel
    }
}

