//! Composant UI pour les téléchargements FFmpeg.
//!
//! Permet de:
//! - Configurer les téléchargements via FFmpeg
//! - Suivre la progression en temps réel
//! - Gérer les options de redémarrage et timeout
//! - Sélectionner les chemins via un explorateur de fichiers
//! - Historique des chemins utilisés

use egui::{Ui, RichText, Color32, ScrollArea};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tokio::sync::{Mutex, mpsc};
use std::path::PathBuf;
use crate::ffmpeg::{self, DownloadOptions, FfmpegProgress};
use std::time::Duration;
use serde::{Serialize, Deserialize};
use std::fs;

const PATH_HISTORY_FILE: &str = "ffmpeg_paths_history.json";

/// Onglet FFmpeg
pub struct FfmpegTab {
    input_url: String,
    output_path: String,
    path_history: Vec<String>,
    stall_timeout_secs: u64,
    max_restarts: u32,
    auto_restart: bool,
    is_downloading: bool,
    cancel_flag: Arc<AtomicBool>,
    progress: Arc<Mutex<FfmpegProgressUI>>,
    error_message: Arc<Mutex<Option<String>>>,
    task_handle: Option<std::thread::JoinHandle<()>>,
    path_selection_tx: Option<mpsc::UnboundedSender<PathBuf>>,
    path_selection_rx: Option<mpsc::UnboundedReceiver<PathBuf>>,
}

#[derive(Serialize, Deserialize)]
struct PathHistory {
    paths: Vec<String>,
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
        let (tx, rx) = mpsc::unbounded_channel();
        let mut tab = Self {
            input_url: String::new(),
            output_path: String::new(),
            path_history: Vec::new(),
            stall_timeout_secs: 30,
            max_restarts: 3,
            auto_restart: true,
            is_downloading: false,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(FfmpegProgressUI::default())),
            error_message: Arc::new(Mutex::new(None)),
            task_handle: None,
            path_selection_tx: Some(tx),
            path_selection_rx: Some(rx),
        };
        tab.load_path_history();
        tab
    }
}

impl FfmpegTab {
    pub fn show(&mut self, ui: &mut Ui) {
        // Traiter les sélections de chemin depuis le dialogue de fichier
        self.process_path_selections();
        
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
                        
                        // Bouton pour sélectionner un fichier
                        if ui.button("📁 Parcourir...").clicked() {
                            self.browse_for_path();
                        }
                    });
                    
                    // Afficher l'historique des chemins
                    if !self.path_history.is_empty() {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Historique:").small().color(Color32::GRAY));
                            ScrollArea::horizontal().show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let history_clone = self.path_history.clone();
                                    for path in history_clone.iter().take(5) {
                                        if ui.small_button(path).clicked() {
                                            let path_clone = path.clone();
                                            self.output_path = path_clone.clone();
                                            self.save_path_to_history(path_clone);
                                        }
                                    }
                                });
                            });
                        });
                    }
                    
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
                        Err(_) => FfmpegProgressUI::default(),
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
    
    /// Ouvre un dialogue pour sélectionner le fichier de destination
    fn browse_for_path(&mut self) {
        let path_tx = self.path_selection_tx.clone();
        let suggested_path = if !self.output_path.is_empty() {
            PathBuf::from(&self.output_path)
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        };
        
        // Lancer le dialogue dans un thread séparé pour ne pas bloquer l'UI
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new()
                .set_directory(suggested_path.parent().unwrap_or(&PathBuf::from(".")));
            
            let dialog = if let Some(file_name) = suggested_path.file_name().and_then(|n| n.to_str()) {
                dialog.set_file_name(file_name)
            } else {
                dialog
            };
            
            if let Some(path) = dialog.save_file() {
                // Envoyer le chemin sélectionné via le canal
                if let Some(tx) = path_tx {
                    let _ = tx.send(path);
                }
            }
        });
    }
    
    /// Traite les sélections de chemin depuis le dialogue de fichier
    fn process_path_selections(&mut self) {
        let mut paths_to_add = Vec::new();
        if let Some(ref mut rx) = self.path_selection_rx {
            while let Ok(path) = rx.try_recv() {
                let path_str = path.to_string_lossy().to_string();
                self.output_path = path_str.clone();
                paths_to_add.push(path_str);
            }
        }
        // Ajouter les chemins à l'historique après avoir libéré l'emprunt
        for path in paths_to_add {
            self.save_path_to_history(path);
        }
    }
    
    /// Charge l'historique des chemins depuis le fichier
    fn load_path_history(&mut self) {
        if let Ok(content) = fs::read_to_string(PATH_HISTORY_FILE) {
            if let Ok(history) = serde_json::from_str::<PathHistory>(&content) {
                self.path_history = history.paths;
            }
        }
    }
    
    /// Sauvegarde l'historique des chemins dans le fichier
    fn save_path_history(&self) {
        let history = PathHistory {
            paths: self.path_history.clone(),
        };
        
        if let Ok(json) = serde_json::to_string_pretty(&history) {
            let _ = fs::write(PATH_HISTORY_FILE, json);
        }
    }
    
    /// Ajoute un chemin à l'historique (sans doublons, limite à 20)
    fn save_path_to_history(&mut self, path: String) {
        // Retirer le chemin s'il existe déjà
        self.path_history.retain(|p| p != &path);
        
        // Ajouter au début
        self.path_history.insert(0, path);
        
        // Limiter à 20 chemins
        if self.path_history.len() > 20 {
            self.path_history.truncate(20);
        }
        
        // Sauvegarder
        self.save_path_history();
    }
    
    fn start_download(&mut self) {
        if self.input_url.is_empty() || self.output_path.is_empty() {
            return;
        }
        
        // Sauvegarder le chemin dans l'historique
        self.save_path_to_history(self.output_path.clone());
        
        self.is_downloading = true;
        self.cancel_flag.store(false, Ordering::Relaxed);
        
        // Réinitialiser les erreurs (non-bloquant)
        if let Ok(mut guard) = self.error_message.try_lock() {
            *guard = None;
        }
        
        let progress = self.progress.clone();
        let error_msg = self.error_message.clone();
        let cancel_flag = self.cancel_flag.clone();
        let input_url = self.input_url.clone();
        let output_path = PathBuf::from(&self.output_path);
        let stall_timeout = Duration::from_secs(self.stall_timeout_secs);
        let max_restarts = self.max_restarts as usize;
        let auto_restart = self.auto_restart;
        
        // Créer un canal pour les mises à jour de progression
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<FfmpegProgressUI>();
        
        // Lancer le téléchargement dans un thread séparé
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
            rt.block_on(async move {
                // Tâche pour traiter les mises à jour de progression (spawnée dans le runtime)
                let progress_clone = progress.clone();
                let progress_task = tokio::spawn(async move {
                    while let Some(prog_ui) = progress_rx.recv().await {
                        if let Ok(mut guard) = progress_clone.try_lock() {
                            *guard = prog_ui;
                        }
                    }
                });
                
                let options = DownloadOptions {
                    stall_timeout,
                    auto_restart,
                    max_restarts,
                };
                
                let progress_tx_clone = progress_tx.clone();
                let error_msg_clone = error_msg.clone();
                
                let result = ffmpeg::download_with_options(
                    &input_url,
                    &output_path,
                    options,
                    Some(move |prog: &FfmpegProgress| {
                        // Envoyer la progression via le canal au lieu de bloquer
                        let prog_ui = FfmpegProgressUI {
                            out_time_ms: prog.fields.get("out_time_ms").cloned(),
                            bitrate: prog.fields.get("bitrate").cloned(),
                            speed: prog.fields.get("speed").cloned(),
                        };
                        let _ = progress_tx_clone.send(prog_ui);
                    }),
                ).await;
                
                // Fermer le canal pour signaler la fin
                drop(progress_tx);
                
                match result {
                    Ok(_) => {
                        // Succès - réinitialiser la progression (non-bloquant)
                        if let Ok(mut guard) = progress.try_lock() {
                            *guard = FfmpegProgressUI::default();
                        }
                    }
                    Err(e) => {
                        // Erreur (non-bloquant)
                        if let Ok(mut guard) = error_msg_clone.try_lock() {
                            *guard = Some(e.to_string());
                        }
                    }
                }
                
                // Attendre que la tâche de progression se termine
                let _ = progress_task.await;
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
            // Ne pas bloquer - laisser le thread se terminer en arrière-plan
            std::thread::spawn(move || {
                let _ = handle.join();
            });
        }
    }
}
