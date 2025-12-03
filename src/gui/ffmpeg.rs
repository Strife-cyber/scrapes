//! Composant UI pour les téléchargements FFmpeg.
//!
//! Permet de:
//! - Configurer les téléchargements via FFmpeg
//! - Suivre la progression en temps réel
//! - Gérer les options de redémarrage et timeout

use egui::{Ui, RichText, Color32, ProgressBar};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tokio::sync::Mutex;
use std::path::PathBuf;
use crate::ffmpeg::{self, DownloadOptions, FfmpegProgress};
use std::time::Duration;

/// Onglet FFmpeg
pub struct FfmpegTab {
    input_url: String,
    output_path: String,
    stall_timeout_secs: u64,
    max_restarts: u32,
    auto_restart: bool,
    is_downloading: bool,
    cancel_flag: Arc<AtomicBool>,
    progress: Arc<Mutex<FfmpegProgressUI>>,
    error_message: Arc<Mutex<Option<String>>>,
    task_handle: Option<std::thread::JoinHandle<()>>,
}

// Utiliser le type FfmpegProgress du module ffmpeg mais avec des champs simplifiés pour l'UI
#[derive(Clone, Debug, Default)]
struct FfmpegProgressUI {
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
            cancel_flag: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(FfmpegProgressUI::default())),
            error_message: Arc::new(Mutex::new(None)),
            task_handle: None,
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
                            if ui.button(RichText::new("⏹️ Arrêter").size(14.0).color(Color32::from_rgb(255, 100, 100)))
                                .clicked() {
                                self.stop_download();
                            }
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
                    // Afficher les erreurs (non-bloquant)
                    if let Ok(error_guard) = self.error_message.try_lock() {
                        if let Some(ref error) = *error_guard {
                            ui.label(RichText::new(format!("❌ Erreur: {}", error))
                                .color(Color32::from_rgb(255, 100, 100)));
                            ui.add_space(8.0);
                        }
                    }
                    
                    // Lire la progression (non-bloquant)
                    let progress = match self.progress.try_lock() {
                        Ok(guard) => guard.clone(),
                        Err(_) => FfmpegProgressUI::default(), // Si on ne peut pas acquérir le lock, utiliser des valeurs par défaut
                    };
                    
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
        self.cancel_flag.store(false, Ordering::Relaxed);
        
        // Réinitialiser les erreurs
        let error_msg = self.error_message.clone();
        {
            let mut guard = error_msg.blocking_lock();
            *guard = None;
        }
        
        let progress = self.progress.clone();
        let cancel_flag = self.cancel_flag.clone();
        let input_url = self.input_url.clone();
        let output_path = PathBuf::from(&self.output_path);
        let stall_timeout = Duration::from_secs(self.stall_timeout_secs);
        let max_restarts = self.max_restarts as usize;
        let auto_restart = self.auto_restart;
        
        // Lancer le téléchargement dans un thread séparé
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
            rt.block_on(async move {
                let options = DownloadOptions {
                    stall_timeout,
                    auto_restart,
                    max_restarts,
                };
                
                let progress_clone = progress.clone();
                let error_msg_clone = error_msg.clone();
                
                let result = ffmpeg::download_with_options(
                    &input_url,
                    &output_path,
                    options,
                    Some(move |prog: &FfmpegProgress| {
                        let mut guard = progress_clone.blocking_lock();
                        guard.out_time_ms = prog.fields.get("out_time_ms").cloned();
                        guard.bitrate = prog.fields.get("bitrate").cloned();
                        guard.speed = prog.fields.get("speed").cloned();
                    }),
                ).await;
                
                match result {
                    Ok(_) => {
                        // Succès - réinitialiser la progression
                        let mut guard = progress.blocking_lock();
                        *guard = FfmpegProgressUI::default();
                    }
                    Err(e) => {
                        let mut guard = error_msg_clone.blocking_lock();
                        *guard = Some(e.to_string());
                    }
                }
            });
        });
        
        self.task_handle = Some(handle);
    }
    
    fn stop_download(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
        self.is_downloading = false;
        
        // Note: FFmpeg ne peut pas être arrêté facilement une fois lancé
        // On peut améliorer ça en ajoutant un mécanisme d'annulation dans le downloader FFmpeg
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.join();
        }
    }
}

