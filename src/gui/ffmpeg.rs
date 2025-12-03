//! Composant UI pour les téléchargements FFmpeg.
//!
//! Permet de:
//! - Configurer les téléchargements via FFmpeg
//! - Suivre la progression en temps réel
//! - Gérer les options de redémarrage et timeout

use egui::{Ui, RichText, Color32};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Onglet FFmpeg
pub struct FfmpegTab {
    input_url: String,
    output_path: String,
    stall_timeout_secs: u64,
    max_restarts: u32,
    auto_restart: bool,
    is_downloading: bool,
    progress: Arc<Mutex<FfmpegProgress>>,
}

#[derive(Clone, Debug, Default)]
struct FfmpegProgress {
    out_time_ms: Option<String>,
    bitrate: Option<String>,
    speed: Option<String>,
}

impl Default for FfmpegTab {
    fn default() -> Self {
        Self {
            input_url: String::new(),
            output_path: String::new(),
            stall_timeout_secs: 30,
            max_restarts: 3,
            auto_restart: true,
            is_downloading: false,
            progress: Arc::new(Mutex::new(FfmpegProgress::default())),
        }
    }
}

impl FfmpegTab {
    pub fn show(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.heading("🎬 Téléchargement FFmpeg");
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
                        ui.label(RichText::new("URL d'entrée:").strong());
                        ui.text_edit_singleline(&mut self.input_url)
                            .on_hover_text("URL du flux (ex: m3u8, mp4)");
                    });
                    
                    ui.add_space(4.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Chemin de sortie:").strong());
                        ui.text_edit_singleline(&mut self.output_path)
                            .on_hover_text("Fichier de destination");
                    });
                    
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                    
                    ui.heading("🔧 Options");
                    ui.add_space(8.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Timeout de blocage (s):").strong());
                        ui.add(egui::Slider::new(&mut self.stall_timeout_secs, 10..=120)
                            .show_value(true));
                    });
                    
                    ui.add_space(4.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Redémarrages max:").strong());
                        ui.add(egui::Slider::new(&mut self.max_restarts, 0..=10)
                            .show_value(true));
                    });
                    
                    ui.add_space(4.0);
                    
                    ui.checkbox(&mut self.auto_restart, RichText::new("Redémarrage automatique").strong());
                    
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                    
                    ui.horizontal(|ui| {
                        let button_enabled = !self.input_url.is_empty() && !self.output_path.is_empty() && !self.is_downloading;
                        if ui.add_enabled(button_enabled, egui::Button::new(RichText::new("▶️ Démarrer").size(14.0)))
                            .clicked() {
                            self.start_download();
                        }
                        
                        if self.is_downloading {
                            ui.spinner();
                            ui.label(RichText::new("Téléchargement en cours...").color(Color32::YELLOW));
                        }
                    });
                });
            
            ui.add_space(12.0);
            
            // Progression
            ui.heading("📊 Progression");
            ui.add_space(4.0);
            
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(25, 25, 30))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 50, 60)))
                .rounding(egui::Rounding::same(6.0))
                .inner_margin(egui::Margin::same(12.0))
                .show(ui, |ui| {
                    let progress_guard = self.progress.blocking_lock();
                    let progress = progress_guard.clone();
                    drop(progress_guard);
                    
                    if self.is_downloading {
                        if let Some(ref time) = progress.out_time_ms {
                            ui.label(RichText::new(format!("Temps: {}", time)).strong());
                        }
                        if let Some(ref bitrate) = progress.bitrate {
                            ui.label(RichText::new(format!("Débit: {}", bitrate)).small().color(Color32::GRAY));
                        }
                        if let Some(ref speed) = progress.speed {
                            ui.label(RichText::new(format!("Vitesse: {}", speed)).small().color(Color32::GRAY));
                        }
                    } else {
                        ui.label(RichText::new("Les informations de progression apparaîtront ici")
                            .color(Color32::GRAY));
                    }
                });
        });
    }
    
    fn start_download(&mut self) {
        if self.input_url.is_empty() || self.output_path.is_empty() {
            return;
        }
        
        self.is_downloading = true;
        // TODO: Intégrer avec le système FFmpeg réel
    }
}

