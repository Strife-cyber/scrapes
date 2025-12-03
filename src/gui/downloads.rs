//! Composant UI pour la gestion des téléchargements.
//!
//! Affiche:
//! - Liste des téléchargements actifs avec progression
//! - Formulaire pour ajouter de nouveaux téléchargements
//! - Statistiques globales

use egui::{Ui, ProgressBar, RichText, Color32, ScrollArea, Frame, Stroke, Rounding, Context};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::{Serialize, Deserialize};
use std::fs;
use crate::downloader::{DownloadTask, DownloadManager};

/// ID unique pour chaque téléchargement
pub type DownloadId = u64;

/// État d'un téléchargement individuel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadItem {
    pub id: DownloadId,
    pub url: String,
    #[serde(with = "pathbuf_serde")]
    pub output_path: PathBuf,
    pub status: DownloadStatus, // SÉRIALISÉ pour sauvegarder le statut dans le JSON
    pub progress: f32, // 0.0 à 1.0
    pub speed: Option<u64>, // bytes/s
    pub total_size: Option<u64>, // bytes
    pub downloaded: u64, // bytes téléchargés
    pub error_message: Option<String>,
    #[serde(skip)]
    pub cancel_flag: Arc<AtomicBool>,
    #[serde(skip)]
    pub task_handle: Option<Arc<Mutex<Option<std::thread::JoinHandle<()>>>>>,
}

// Helper pour sérialiser PathBuf
mod pathbuf_serde {
    use serde::{Serializer, Deserializer, Deserialize};
    use std::path::PathBuf;
    
    pub fn serialize<S>(path: &PathBuf, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&path.to_string_lossy())
    }
    
    pub fn deserialize<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(PathBuf::from(s))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Paused,
    Merging,
    Completed,
    Error(String),
    Cancelled,
}

impl Default for DownloadStatus {
    fn default() -> Self {
        DownloadStatus::Queued
    }
}

impl DownloadStatus {
    fn color(&self) -> Color32 {
        match self {
            DownloadStatus::Queued => Color32::from_gray(150),
            DownloadStatus::Downloading => Color32::from_rgb(100, 200, 255),
            DownloadStatus::Paused => Color32::from_rgb(255, 200, 100),
            DownloadStatus::Merging => Color32::from_rgb(255, 200, 100),
            DownloadStatus::Completed => Color32::from_rgb(100, 255, 100),
            DownloadStatus::Error(_) => Color32::from_rgb(255, 100, 100),
            DownloadStatus::Cancelled => Color32::from_gray(100),
        }
    }
    
    fn text(&self) -> &'static str {
        match self {
            DownloadStatus::Queued => "⏳ En attente",
            DownloadStatus::Downloading => "⬇️ Téléchargement",
            DownloadStatus::Paused => "⏸️ En pause",
            DownloadStatus::Merging => "🔗 Fusion",
            DownloadStatus::Completed => "✅ Terminé",
            DownloadStatus::Error(_) => "❌ Erreur",
            DownloadStatus::Cancelled => "🚫 Annulé",
        }
    }
}

/// Message de progression pour un téléchargement
#[derive(Clone, Debug)]
pub enum DownloadProgress {
    Started { id: DownloadId, total_size: u64 },
    Progress { id: DownloadId, downloaded: u64, speed: Option<u64> },
    Merging { id: DownloadId },
    Completed { id: DownloadId },
    Error { id: DownloadId, error: String },
    Paused { id: DownloadId },
    Cancelled { id: DownloadId },
}

impl DownloadProgress {
    fn id(&self) -> DownloadId {
        match self {
            DownloadProgress::Started { id, .. } => *id,
            DownloadProgress::Progress { id, .. } => *id,
            DownloadProgress::Merging { id } => *id,
            DownloadProgress::Completed { id } => *id,
            DownloadProgress::Error { id, .. } => *id,
            DownloadProgress::Paused { id } => *id,
            DownloadProgress::Cancelled { id } => *id,
        }
    }
}

const HISTORY_FILE: &str = "downloads_history.json";

/// Filtre pour afficher les téléchargements
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DownloadFilter {
    Active,      // En cours, en file, en pause
    Completed,   // Terminés
    All,         // Tous
}

/// Onglet des téléchargements
pub struct DownloadsTab {
    downloads: Arc<Mutex<HashMap<DownloadId, DownloadItem>>>,
    history: Arc<Mutex<HashMap<DownloadId, DownloadItem>>>, // Téléchargements terminés
    new_url: String,
    new_path: String,
    default_download_dir: PathBuf, // Dossier par défaut pour les téléchargements
    next_id: Arc<Mutex<DownloadId>>,
    progress_rx: Option<mpsc::UnboundedReceiver<DownloadProgress>>,
    progress_tx: Option<mpsc::UnboundedSender<DownloadProgress>>,
    ctx: Option<Context>,
    filter: DownloadFilter,
    path_selection_rx: Option<mpsc::UnboundedReceiver<PathBuf>>, // Canal pour recevoir les sélections de chemin
    path_selection_tx: Option<mpsc::UnboundedSender<PathBuf>>, // Canal pour envoyer les sélections de chemin
}

impl Default for DownloadsTab {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (path_tx, path_rx) = mpsc::unbounded_channel();
        
        // Déterminer le dossier de téléchargement par défaut
        let default_dir = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .map(|home| PathBuf::from(home).join("Downloads"))
            .unwrap_or_else(|_| PathBuf::from("."));
        
        let mut tab = Self {
            downloads: Arc::new(Mutex::new(HashMap::new())),
            history: Arc::new(Mutex::new(HashMap::new())),
            new_url: String::new(),
            new_path: String::new(),
            default_download_dir: default_dir,
            next_id: Arc::new(Mutex::new(0)),
            progress_rx: Some(rx),
            progress_tx: Some(tx),
            ctx: None,
            filter: DownloadFilter::Active,
            path_selection_rx: Some(path_rx),
            path_selection_tx: Some(path_tx),
        };
        
        // Charger l'historique au démarrage
        tab.load_history();
        
        tab
    }
}

impl DownloadsTab {
    /// Définit le contexte egui pour les mises à jour
    pub fn set_context(&mut self, ctx: Context) {
        self.ctx = Some(ctx);
    }
    
    /// Suggère un nom de fichier basé sur l'URL
    fn suggest_filename_from_url(&mut self) {
        if let Ok(url) = url::Url::parse(&self.new_url) {
            // Essayer d'extraire le nom de fichier de l'URL
            if let Some(segments) = url.path_segments() {
                let segments: Vec<_> = segments.collect();
                if let Some(last_segment) = segments.last() {
                    // Nettoyer le segment (enlever les paramètres de requête)
                    let clean_segment = last_segment.split('?').next().unwrap_or(last_segment);
                    if !clean_segment.is_empty() && clean_segment.contains('.') {
                        // C'est probablement un nom de fichier
                        let suggested_path = self.default_download_dir.join(clean_segment);
                        self.new_path = suggested_path.to_string_lossy().to_string();
                        return;
                    }
                }
            }
            
            // Si pas de nom de fichier dans l'URL, essayer d'extraire depuis les paramètres
            // ou utiliser le domaine + timestamp
            if let Some(domain) = url.domain() {
                // Essayer de trouver une extension dans le path
                let path = url.path();
                let extension = if path.contains('.') {
                    path.rsplit('.').next().unwrap_or("bin")
                } else {
                    // Essayer de deviner l'extension depuis le Content-Type ou utiliser "bin"
                    "bin"
                };
                
                // Utiliser le domaine (nettoyé) + extension
                let clean_domain = domain.replace('.', "_").replace('-', "_");
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let filename = format!("{}_{}.{}", clean_domain, timestamp, extension);
                let suggested_path = self.default_download_dir.join(filename);
                self.new_path = suggested_path.to_string_lossy().to_string();
            }
        }
    }
    
    /// Ouvre un dialogue pour sélectionner le fichier de destination
    fn browse_for_path(&mut self) {
        let path_tx = self.path_selection_tx.clone();
        let default_dir = self.default_download_dir.clone();
        let suggested_path = if !self.new_path.is_empty() {
            PathBuf::from(&self.new_path)
        } else {
            default_dir.clone()
        };
        
        // Lancer le dialogue dans un thread séparé pour ne pas bloquer l'UI
        std::thread::spawn(move || {
            // Extraire le nom de fichier suggéré si disponible
            let file_name = suggested_path.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string());
            
            let dialog = rfd::FileDialog::new()
                .set_directory(&default_dir);
            
            let dialog = if let Some(name) = file_name {
                dialog.set_file_name(&name)
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
        if let Some(ref mut rx) = self.path_selection_rx {
            while let Ok(path) = rx.try_recv() {
                self.new_path = path.to_string_lossy().to_string();
            }
        }
    }
    
    /// Traite les messages de progression reçus (non-bloquant pour le thread UI)
    fn process_progress_updates(&mut self) {
        if let Some(ref mut rx) = self.progress_rx {
            let mut needs_save = false;
            
            // Traiter tous les messages disponibles sans bloquer
            while let Ok(progress) = rx.try_recv() {
                // Utiliser try_lock pour ne pas bloquer le thread UI
                if let Ok(mut downloads) = self.downloads.try_lock() {
                    if let Some(download) = downloads.get_mut(&progress.id()) {
                        match progress {
                            DownloadProgress::Started { total_size, .. } => {
                                download.status = DownloadStatus::Downloading;
                                download.total_size = Some(total_size);
                                download.progress = 0.0;
                            }
                            DownloadProgress::Progress { downloaded, speed, .. } => {
                                download.downloaded = downloaded;
                                download.speed = speed;
                                if let Some(total) = download.total_size {
                                    download.progress = downloaded as f32 / total as f32;
                                }
                            }
                            DownloadProgress::Merging { .. } => {
                                download.status = DownloadStatus::Merging;
                            }
                            DownloadProgress::Completed { id } => {
                                download.status = DownloadStatus::Completed;
                                download.progress = 1.0;
                                download.speed = None;
                                
                                // Déplacer vers l'historique (non-bloquant)
                                drop(downloads);
                                if let (Ok(mut downloads), Ok(mut history)) = (
                                    self.downloads.try_lock(),
                                    self.history.try_lock(),
                                ) {
                                    if let Some(mut completed) = downloads.remove(&id) {
                                        // S'assurer que le statut est bien Completed
                                        completed.status = DownloadStatus::Completed;
                                        completed.progress = 1.0;
                                        history.insert(id, completed);
                                        needs_save = true;
                                    }
                                }
                                continue; // On a déjà drop downloads, pas besoin de continuer
                            }
                            DownloadProgress::Error { error, .. } => {
                                download.status = DownloadStatus::Error(error.clone());
                                download.error_message = Some(error);
                                needs_save = true;
                            }
                            DownloadProgress::Paused { .. } => {
                                download.status = DownloadStatus::Paused;
                            }
                            DownloadProgress::Cancelled { .. } => {
                                download.status = DownloadStatus::Cancelled;
                            }
                        }
                        needs_save = true;
                    }
                } else {
                    // Si on ne peut pas acquérir le lock, on skip ce message
                    // Il sera traité au prochain frame
                    break;
                }
            }
            
            // Sauvegarder dans un thread séparé pour ne pas bloquer l'UI
            if needs_save {
                self.save_history_async();
            }
        }
        
        // Demander un repaint si nécessaire
        if let Some(ref ctx) = self.ctx {
            ctx.request_repaint();
        }
    }
    
    pub fn show(&mut self, ui: &mut Ui) {
        // Traiter les mises à jour de progression
        self.process_progress_updates();
        // Traiter les sélections de chemin depuis le dialogue de fichier
        self.process_path_selections();
        ui.vertical(|ui| {
            // En-tête avec statistiques
            ui.horizontal(|ui| {
                ui.heading("📥 Gestionnaire de Téléchargements");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let stats = self.get_stats();
                    ui.label(RichText::new(format!("Actifs: {} | Terminés: {}", stats.active, stats.completed))
                        .color(Color32::GRAY)
                        .small());
                });
            });
            ui.separator();
            
            // Formulaire d'ajout avec style amélioré
            Frame::group(ui.style())
                .fill(Color32::from_rgb(30, 30, 35))
                .stroke(Stroke::new(1.0, Color32::from_rgb(60, 60, 70)))
                .rounding(Rounding::same(8.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.heading("➕ Nouveau Téléchargement");
                    ui.add_space(8.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("URL:").strong());
                        let url_edit = ui.text_edit_singleline(&mut self.new_url)
                            .on_hover_text("URL du fichier à télécharger");
                        
                        // Si l'URL change, suggérer automatiquement le nom de fichier
                        if url_edit.changed() && !self.new_url.is_empty() {
                            self.suggest_filename_from_url();
                        }
                    });
                    
                    ui.add_space(4.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Destination:").strong());
                        ui.text_edit_singleline(&mut self.new_path)
                            .on_hover_text("Chemin complet du fichier de destination");
                        
                        // Bouton pour sélectionner un fichier/dossier
                        if ui.button("📁 Parcourir...").clicked() {
                            self.browse_for_path();
                        }
                    });
                    
                    // Aide contextuelle
                    if self.new_path.is_empty() && !self.new_url.is_empty() {
                        ui.label(RichText::new("💡 Astuce: Le nom de fichier sera suggéré automatiquement depuis l'URL")
                            .small()
                            .color(Color32::GRAY));
                    }
                    
                    ui.add_space(8.0);
                    
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("➕ Ajouter à la file").size(14.0)).clicked() {
                            self.add_download();
                        }
                        if ui.button(RichText::new("🗑️ Effacer").size(14.0)).clicked() {
                            self.new_url.clear();
                            self.new_path.clear();
                        }
                    });
                    
                    ui.add_space(8.0);
                    
                    // Bouton pour démarrer les téléchargements en file
                    let queued_count = {
                        match self.downloads.try_lock() {
                            Ok(downloads) => downloads.values()
                                .filter(|d| matches!(d.status, DownloadStatus::Queued))
                                .count(),
                            Err(_) => 0, // Si on ne peut pas acquérir le lock, skip
                        }
                    };
                    
                    if queued_count > 0 {
                        ui.horizontal(|ui| {
                            if ui.button(RichText::new(format!("▶️ Démarrer {} téléchargement(s)", queued_count)).size(14.0).color(Color32::from_rgb(100, 255, 100)))
                                .clicked() {
                                self.start_downloads();
                            }
                        });
                    }
                });
            
            ui.add_space(12.0);
            
            // Filtres et en-tête
            ui.horizontal(|ui| {
                ui.heading("📋 Téléchargements");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.selectable_value(&mut self.filter, DownloadFilter::All, "Tous");
                    ui.selectable_value(&mut self.filter, DownloadFilter::Completed, "Historique");
                    ui.selectable_value(&mut self.filter, DownloadFilter::Active, "Actifs");
                });
            });
            ui.add_space(4.0);
            
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    // Utiliser try_lock pour ne pas bloquer le thread UI
                    let (active_downloads, history_downloads) = {
                        match (self.downloads.try_lock(), self.history.try_lock()) {
                            (Ok(downloads_guard), Ok(history_guard)) => {
                                let active: Vec<_> = downloads_guard.values().cloned().collect();
                                let history: Vec<_> = history_guard.values().cloned().collect();
                                (active, history)
                            }
                            _ => {
                                // Si on ne peut pas acquérir les locks, utiliser des données vides
                                // Les données seront disponibles au prochain frame
                                (Vec::new(), Vec::new())
                            }
                        }
                    };
                    
                    // Filtrer selon le filtre sélectionné
                    let mut to_display = Vec::new();
                    match self.filter {
                        DownloadFilter::Active => {
                            to_display = active_downloads;
                        }
                        DownloadFilter::Completed => {
                            to_display = history_downloads;
                        }
                        DownloadFilter::All => {
                            to_display = active_downloads;
                            to_display.extend(history_downloads);
                        }
                    }
                    
                    // Trier par ID (ordre d'ajout)
                    to_display.sort_by_key(|d| d.id);
                    
                    if to_display.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            let message = match self.filter {
                                DownloadFilter::Active => "Aucun téléchargement actif",
                                DownloadFilter::Completed => "Aucun téléchargement dans l'historique",
                                DownloadFilter::All => "Aucun téléchargement",
                            };
                            ui.label(RichText::new(format!("📭 {}", message)).size(18.0).color(Color32::GRAY));
                            if self.filter == DownloadFilter::Active {
                                ui.label(RichText::new("Ajoutez un téléchargement ci-dessus pour commencer").color(Color32::DARK_GRAY));
                            }
                        });
                    } else {
                        for download in &to_display {
                            self.render_download_item(ui, download);
                            ui.add_space(8.0);
                        }
                    }
                });
        });
    }
    
    fn render_download_item(&mut self, ui: &mut Ui, download: &DownloadItem) {
        Frame::group(ui.style())
            .fill(Color32::from_rgb(25, 25, 30))
            .stroke(Stroke::new(1.0, Color32::from_rgb(50, 50, 60)))
            .rounding(Rounding::same(6.0))
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                // En-tête avec statut
                ui.horizontal(|ui| {
                    ui.label(RichText::new(download.status.text())
                        .color(download.status.color())
                        .strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match download.status {
                            DownloadStatus::Downloading | DownloadStatus::Merging => {
                                if ui.small_button("⏸️").clicked() {
                                    self.pause_download(download.id);
                                }
                                if ui.small_button("❌").clicked() {
                                    self.cancel_download(download.id);
                                }
                            }
                            DownloadStatus::Paused | DownloadStatus::Queued => {
                                if ui.small_button("▶️").clicked() {
                                    self.resume_download(download.id);
                                }
                                if ui.small_button("❌").clicked() {
                                    self.cancel_download(download.id);
                                }
                            }
                            DownloadStatus::Error(_) | DownloadStatus::Cancelled => {
                                // Seulement pour les téléchargements actifs, pas l'historique
                                if matches!(self.filter, DownloadFilter::Active | DownloadFilter::All) {
                                    if ui.small_button("🔄").clicked() {
                                        self.restart_download(download.id);
                                    }
                                }
                            }
                            _ => {}
                        }
                        
                        // Bouton pour nettoyer les fichiers part (toujours disponible)
                        if ui.small_button("🗑️").on_hover_text("Nettoyer les fichiers part").clicked() {
                            self.cleanup_part_files(download.id);
                        }
                    });
                });
                
                ui.add_space(4.0);
                
                // Nom du fichier
                let filename = download.output_path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Fichier inconnu");
                ui.label(RichText::new(filename).strong());
                
                // URL (tronquée)
                let url_display = if download.url.len() > 80 {
                    format!("{}...", &download.url[..80])
                } else {
                    download.url.clone()
                };
                ui.label(RichText::new(url_display).small().color(Color32::GRAY));
                
                ui.add_space(8.0);
                
                // Barre de progression
                if download.status == DownloadStatus::Downloading || download.status == DownloadStatus::Merging {
                    let progress_bar = ProgressBar::new(download.progress)
                        .fill(Color32::from_rgb(100, 200, 255))
                        .show_percentage();
                    ui.add(progress_bar);
                    
                    ui.add_space(4.0);
                    
                    // Informations de progression
                    ui.horizontal(|ui| {
                        if let Some(total) = download.total_size {
                            let downloaded_mb = download.downloaded as f64 / 1_048_576.0;
                            let total_mb = total as f64 / 1_048_576.0;
                            ui.label(RichText::new(format!("{:.2} MB / {:.2} MB", downloaded_mb, total_mb))
                                .small()
                                .color(Color32::GRAY));
                        }
                        
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(speed) = download.speed {
                                let speed_mb = speed as f64 / 1_048_576.0;
                                ui.label(RichText::new(format!("{:.2} MB/s", speed_mb))
                                    .small()
                                    .color(Color32::GRAY));
                            }
                        });
                    });
                } else if let DownloadStatus::Error(ref err) = download.status {
                    ui.label(RichText::new(format!("Erreur: {}", err))
                        .color(Color32::from_rgb(255, 100, 100))
                        .small());
                } else if download.status == DownloadStatus::Completed {
                    ui.label(RichText::new("✅ Téléchargement terminé")
                        .color(Color32::from_rgb(100, 255, 100))
                        .small());
                }
            });
    }
    
    fn get_stats(&self) -> DownloadStats {
        // Utiliser try_lock pour ne pas bloquer le thread UI
        let downloads = match self.downloads.try_lock() {
            Ok(guard) => guard,
            Err(_) => return DownloadStats { active: 0, completed: 0 },
        };
        let history = match self.history.try_lock() {
            Ok(guard) => guard,
            Err(_) => return DownloadStats { active: 0, completed: 0 },
        };
        
        let active = downloads.values()
            .filter(|d| matches!(d.status, DownloadStatus::Downloading | DownloadStatus::Merging | DownloadStatus::Queued))
            .count();
        let completed = history.len();
        
        DownloadStats { active, completed }
    }
    
    fn add_download(&mut self) {
        if self.new_url.is_empty() || self.new_path.is_empty() {
            return;
        }
        
        let output_path = PathBuf::from(&self.new_path);
        let id = {
            let mut next_id = self.next_id.blocking_lock();
            *next_id += 1;
            *next_id
        };
        
        let item = DownloadItem {
            id,
            url: self.new_url.clone(),
            output_path: output_path.clone(),
            status: DownloadStatus::Queued,
            progress: 0.0,
            speed: None,
            total_size: None,
            downloaded: 0,
            error_message: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            task_handle: Some(Arc::new(Mutex::new(None))),
        };
        
        // Pour l'insertion, utiliser try_lock avec retry si nécessaire
        let mut retries = 0;
        loop {
            match self.downloads.try_lock() {
                Ok(mut downloads) => {
                    downloads.insert(id, item);
                    break;
                }
                Err(_) => {
                    retries += 1;
                    if retries > 10 {
                        // Si on ne peut pas acquérir le lock après 10 tentatives, skip
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
        
        // Sauvegarder l'historique de manière asynchrone
        self.save_history_async();
        
        // Réinitialiser le formulaire
        self.new_url.clear();
        self.new_path.clear();
    }
    
    /// Charge l'historique depuis le fichier JSON (appelé une seule fois au démarrage)
    fn load_history(&mut self) {
        // Charger dans un thread séparé pour ne pas bloquer l'UI au démarrage
        let downloads = self.downloads.clone();
        let history = self.history.clone();
        let next_id = self.next_id.clone();
        
        std::thread::spawn(move || {
            if let Ok(content) = fs::read_to_string(HISTORY_FILE) {
                if let Ok(items) = serde_json::from_str::<Vec<DownloadItem>>(&content) {
                    let mut downloads_guard = downloads.blocking_lock();
                    let mut history_guard = history.blocking_lock();
                    let mut max_id = 0;
                    
                    for mut item in items {
                        // Réinitialiser les champs non-sérialisables
                        item.cancel_flag = Arc::new(AtomicBool::new(false));
                        item.task_handle = Some(Arc::new(Mutex::new(None)));
                        
                        max_id = max_id.max(item.id);
                        
                        // Séparer les téléchargements actifs de l'historique
                        if matches!(item.status, DownloadStatus::Completed) {
                            // Téléchargements terminés -> historique
                            history_guard.insert(item.id, item);
                        } else if matches!(item.status, DownloadStatus::Downloading | DownloadStatus::Merging) {
                            // Téléchargements en cours -> remettre en file
                            item.status = DownloadStatus::Queued;
                            downloads_guard.insert(item.id, item);
                        } else {
                            // Autres (Queued, Paused, Error, Cancelled) -> actifs
                            downloads_guard.insert(item.id, item);
                        }
                    }
                    drop(downloads_guard);
                    drop(history_guard);
                    
                    // Mettre à jour le prochain ID
                    let mut next_id_guard = next_id.blocking_lock();
                    *next_id_guard = max_id + 1;
                }
            }
        });
    }
    
    /// Sauvegarde l'historique dans le fichier JSON (version synchrone - à éviter dans le thread UI)
    fn save_history(&self) {
        // Utiliser try_lock pour ne pas bloquer
        let downloads = match self.downloads.try_lock() {
            Ok(guard) => guard,
            Err(_) => return, // Si on ne peut pas acquérir le lock, skip
        };
        let history = match self.history.try_lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        
        // Combiner actifs et historique, exclure les annulés
        let mut items: Vec<_> = downloads.values()
            .filter(|d| !matches!(d.status, DownloadStatus::Cancelled))
            .cloned()
            .collect();
        items.extend(history.values().cloned());
        
        drop(downloads);
        drop(history);
        
        // Écrire dans un thread séparé pour ne pas bloquer l'UI
        let json = match serde_json::to_string_pretty(&items) {
            Ok(j) => j,
            Err(_) => return,
        };
        
        // Lancer l'écriture dans un thread séparé
        std::thread::spawn(move || {
            let _ = fs::write(HISTORY_FILE, json);
        });
    }
    
    /// Sauvegarde asynchrone de l'historique (non-bloquant)
    fn save_history_async(&self) {
        // Cloner les données nécessaires
        let downloads = match self.downloads.try_lock() {
            Ok(guard) => guard.values().cloned().collect::<Vec<_>>(),
            Err(_) => return,
        };
        let history = match self.history.try_lock() {
            Ok(guard) => guard.values().cloned().collect::<Vec<_>>(),
            Err(_) => return,
        };
        
        // Combiner et filtrer - s'assurer que les téléchargements complétés sont inclus
        let mut items: Vec<_> = downloads.into_iter()
            .filter(|d| !matches!(d.status, DownloadStatus::Cancelled))
            .collect();
        // Ajouter tous les éléments de l'historique (qui incluent les complétés)
        items.extend(history);
        
        // Écrire dans un thread séparé
        let json = match serde_json::to_string_pretty(&items) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Erreur lors de la sérialisation de l'historique: {}", e);
                return;
            }
        };
        
        std::thread::spawn(move || {
            if let Err(e) = fs::write(HISTORY_FILE, json) {
                tracing::warn!("Erreur lors de l'écriture de l'historique: {}", e);
            } else {
                tracing::debug!("Historique sauvegardé avec succès");
            }
        });
    }
    
    /// Met en pause un téléchargement (non-bloquant)
    fn pause_download(&mut self, id: DownloadId) {
        // Utiliser try_lock pour ne pas bloquer le thread UI
        if let Ok(mut downloads) = self.downloads.try_lock() {
            if let Some(download) = downloads.get_mut(&id) {
                download.cancel_flag.store(true, Ordering::Relaxed);
                download.status = DownloadStatus::Paused;
            }
        }
        
        // Sauvegarder de manière asynchrone
        self.save_history_async();
        
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(DownloadProgress::Paused { id });
        }
    }
    
    /// Annule un téléchargement (non-bloquant)
    fn cancel_download(&mut self, id: DownloadId) {
        // Utiliser try_lock pour ne pas bloquer le thread UI
        if let Ok(mut downloads) = self.downloads.try_lock() {
            if let Some(download) = downloads.get_mut(&id) {
                download.cancel_flag.store(true, Ordering::Relaxed);
                download.status = DownloadStatus::Cancelled;
                
                // Arrêter la tâche si elle existe
                if let Some(handle_arc) = &download.task_handle {
                    if let Ok(mut handle_opt) = handle_arc.try_lock() {
                        if let Some(handle) = handle_opt.take() {
                            // Note: On ne peut pas vraiment arrêter un thread, mais on peut marquer comme annulé
                            drop(handle);
                        }
                    }
                }
            }
        }
        
        // Sauvegarder de manière asynchrone
        self.save_history_async();
        
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(DownloadProgress::Cancelled { id });
        }
    }
    
    /// Reprend un téléchargement en pause (non-bloquant)
    fn resume_download(&mut self, id: DownloadId) {
        // Vérifier l'état avec try_lock
        let can_resume = {
            match self.downloads.try_lock() {
                Ok(downloads) => {
                    downloads.get(&id)
                        .map(|d| matches!(d.status, DownloadStatus::Paused | DownloadStatus::Queued))
                        .unwrap_or(false)
                }
                Err(_) => false, // Si on ne peut pas acquérir le lock, skip
            }
        };
        
        if !can_resume {
            return;
        }
        
        // Cloner les données nécessaires
        let (url, output) = {
            match self.downloads.try_lock() {
                Ok(downloads) => {
                    if let Some(d) = downloads.get(&id) {
                        (Some(d.url.clone()), Some(d.output_path.clone()))
                    } else {
                        (None, None)
                    }
                }
                Err(_) => (None, None),
            }
        };
        
        if let (Some(url), Some(output)) = (url, output) {
            let tx = self.progress_tx.clone().expect("Progress channel should exist");
            
            // Mettre à jour le statut (non-bloquant)
            if let Ok(mut downloads) = self.downloads.try_lock() {
                if let Some(d) = downloads.get_mut(&id) {
                    d.status = DownloadStatus::Queued;
                    d.cancel_flag.store(false, Ordering::Relaxed);
                }
            }
            
            // Relancer le téléchargement avec runtime multi-thread
            std::thread::Builder::new()
                .name(format!("download-{}", id))
                .spawn(move || {
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(4)
                        .enable_all()
                        .build()
                        .expect("Failed to create runtime");
                    rt.block_on(async move {
                        let result = Self::run_download(id, url, output, tx.clone()).await;
                        if let Err(e) = result {
                            let _ = tx.send(DownloadProgress::Error {
                                id,
                                error: e.to_string(),
                            });
                        }
                    });
                })
                .expect("Failed to spawn download thread");
        }
    }
    
    /// Redémarre un téléchargement (après erreur ou annulation)
    fn restart_download(&mut self, id: DownloadId) {
        // Chercher dans les téléchargements actifs d'abord
        let mut downloads = self.downloads.blocking_lock();
        let download = downloads.get(&id).cloned();
        drop(downloads);
        
        // Si pas trouvé dans actifs, chercher dans l'historique
        let mut download = if let Some(d) = download {
            Some(d)
        } else {
            let mut history = self.history.blocking_lock();
            history.get(&id).cloned()
        };
        
        if let Some(mut download) = download {
            // Réinitialiser l'état
            download.status = DownloadStatus::Queued;
            download.progress = 0.0;
            download.downloaded = 0;
            download.error_message = None;
            download.cancel_flag = Arc::new(AtomicBool::new(false));
            download.task_handle = Some(Arc::new(Mutex::new(None)));
            
            // NE PAS supprimer les fichiers part - ils seront réutilisés pour la reprise
            
            // Retirer de l'historique si présent
            let mut history = self.history.blocking_lock();
            history.remove(&id);
            drop(history);
            
            // Remettre dans la liste active
            let mut downloads = self.downloads.blocking_lock();
            downloads.insert(id, download);
            drop(downloads);
            
            // Démarrer le téléchargement
            self.resume_download(id);
        }
    }
    
    /// Nettoie manuellement les fichiers part d'un téléchargement (non-bloquant)
    fn cleanup_part_files(&mut self, id: DownloadId) {
        // Chercher dans les téléchargements actifs d'abord (non-bloquant)
        let download = match self.downloads.try_lock() {
            Ok(downloads) => downloads.get(&id).cloned(),
            Err(_) => None,
        };
        
        // Si pas trouvé dans actifs, chercher dans l'historique (non-bloquant)
        let download = if let Some(d) = download {
            Some(d)
        } else {
            match self.history.try_lock() {
                Ok(history) => history.get(&id).cloned(),
                Err(_) => None,
            }
        };
        
        if let Some(download) = download {
            let output_dir = download.output_path.parent().unwrap_or(std::path::Path::new("."));
            let output_stem = download.output_path.file_stem().unwrap_or_else(|| std::ffi::OsStr::new("file"));
            
            // Effectuer le nettoyage dans un thread séparé pour ne pas bloquer l'UI
            let output_dir = output_dir.to_path_buf();
            let output_stem = output_stem.to_string_lossy().to_string();
            std::thread::spawn(move || {
                let mut removed_count = 0;
                if let Ok(entries) = std::fs::read_dir(&output_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            // Supprimer les fichiers part
                            if name.starts_with(&format!("{}.part", output_stem)) && !name.ends_with(".done") {
                                if std::fs::remove_file(&path).is_ok() {
                                    removed_count += 1;
                                }
                            }
                            // Supprimer les marqueurs .done
                            if name.ends_with(".done") && name.starts_with(&format!("{}.part", output_stem)) {
                                if std::fs::remove_file(&path).is_ok() {
                                    removed_count += 1;
                                }
                            }
                        }
                    }
                }
                tracing::info!("Nettoyé {} fichier(s) part pour le téléchargement {}", removed_count, id);
            });
        }
    }
    
    /// Démarre tous les téléchargements en file d'attente
    fn start_downloads(&mut self) {
        let downloads = self.downloads.blocking_lock();
        let queued: Vec<_> = downloads.values()
            .filter(|d| matches!(d.status, DownloadStatus::Queued | DownloadStatus::Paused))
            .cloned()
            .collect();
        drop(downloads);
        
        if queued.is_empty() {
            return;
        }
        
        let progress_tx = self.progress_tx.clone().expect("Progress channel should exist");
        
        // Démarrer chaque téléchargement dans une tâche tokio séparée
        for download in queued {
            let id = download.id;
            let url = download.url.clone();
            let output = download.output_path.clone();
            let tx = progress_tx.clone();
            
            // Mettre à jour le statut (non-bloquant)
            if let Ok(mut downloads) = self.downloads.try_lock() {
                if let Some(d) = downloads.get_mut(&id) {
                    d.status = DownloadStatus::Downloading;
                }
            }
            
            // Lancer chaque téléchargement dans son propre thread avec son propre runtime tokio
            // Cela permet un parallélisme illimité - chaque téléchargement est complètement indépendant
            let url_clone = url.clone();
            let output_clone = output.clone();
            let handle = std::thread::Builder::new()
                .name(format!("download-{}", id))
                .spawn(move || {
                    // Créer un runtime tokio multi-thread pour chaque téléchargement
                    // Cela permet un vrai parallélisme - chaque téléchargement peut utiliser plusieurs threads
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(4) // 4 threads par téléchargement pour le parallélisme interne
                        .enable_all()
                        .build()
                        .expect("Failed to create runtime");
                    rt.block_on(async move {
                        let result = Self::run_download(id, url_clone, output_clone, tx.clone()).await;
                        if let Err(e) = result {
                            let _ = tx.send(DownloadProgress::Error {
                                id,
                                error: e.to_string(),
                            });
                        }
                    });
                })
                .expect("Failed to spawn download thread");
            
            // Stocker le handle pour pouvoir l'arrêter (non-bloquant)
            if let Ok(mut downloads) = self.downloads.try_lock() {
                if let Some(d) = downloads.get_mut(&id) {
                    if let Some(handle_arc) = &d.task_handle {
                        if let Ok(mut handle_opt) = handle_arc.try_lock() {
                            *handle_opt = Some(handle);
                        }
                    }
                }
            }
        }
    }
    
    /// Exécute un téléchargement et envoie les mises à jour de progression
    async fn run_download(
        id: DownloadId,
        url: String,
        output: PathBuf,
        progress_tx: mpsc::UnboundedSender<DownloadProgress>,
    ) -> anyhow::Result<()> {
        use std::time::{Instant, Duration};
        use tokio::time::sleep;
        
        // Détecter la taille totale d'abord
        let client = reqwest::Client::builder().build()?;
        let resp = client.head(&url).send().await?;
        resp.error_for_status_ref()?;
        
        let total_size = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        
        let _ = progress_tx.send(DownloadProgress::Started { id, total_size });
        
        // Démarrer le téléchargement dans une tâche séparée pour suivre la progression
        let manager = DownloadManager::new();
        let task = DownloadTask {
            url: url.clone(),
            output: output.clone(),
            total_size: 0,
            chunk_size: 8 * 1024 * 1024, // 8 MiB
            num_chunks: 0,
        };
        
        let start_time = Instant::now();
        let progress_tx_clone = progress_tx.clone();
        
        // Tâche de suivi de progression (compte les chunks complétés)
        let progress_task = tokio::spawn(async move {
            let mut last_downloaded = 0u64;
            let chunk_size = 8 * 1024 * 1024; // 8 MiB
            let output_dir = output.parent().unwrap_or(std::path::Path::new("."));
            let output_stem = output.file_stem().unwrap_or_else(|| std::ffi::OsStr::new("file"));
            
            loop {
                sleep(Duration::from_millis(500)).await;
                
                // Compter les chunks complétés (présence de fichiers .done)
                let mut completed_chunks = 0u64;
                let mut total_chunks = 0u64;
                
                if let Ok(entries) = std::fs::read_dir(&output_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            // Compter les fichiers part
                            if name.starts_with(&format!("{}.part", output_stem.to_string_lossy())) && !name.ends_with(".done") {
                                total_chunks += 1;
                            }
                            // Compter les chunks complétés
                            if name.ends_with(".done") && name.starts_with(&format!("{}.part", output_stem.to_string_lossy())) {
                                completed_chunks += 1;
                                total_chunks += 1;
                            }
                        }
                    }
                }
                
                // Calculer les bytes téléchargés basés sur les chunks complétés
                let current_downloaded = if total_size > 0 && total_chunks > 0 {
                    // Estimer basé sur les chunks complétés
                    let chunks_expected = (total_size + chunk_size - 1) / chunk_size;
                    let bytes_per_chunk = if chunks_expected > 0 { total_size / chunks_expected } else { chunk_size };
                    completed_chunks * bytes_per_chunk
                } else {
                    // Fallback: vérifier la taille réelle des fichiers part
                    let mut actual_size = 0u64;
                    if let Ok(entries) = std::fs::read_dir(&output_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                if name.starts_with(&format!("{}.part", output_stem.to_string_lossy())) && !name.ends_with(".done") {
                                    if let Ok(meta) = std::fs::metadata(&path) {
                                        actual_size += meta.len();
                                    }
                                }
                            }
                        }
                    }
                    actual_size
                };
                
                // Limiter à la taille totale
                let current_downloaded = current_downloaded.min(total_size);
                
                if current_downloaded > last_downloaded || current_downloaded == 0 {
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let speed = if elapsed > 0.0 && current_downloaded > 0 {
                        Some((current_downloaded as f64 / elapsed) as u64)
                    } else {
                        None
                    };
                    
                    let _ = progress_tx_clone.send(DownloadProgress::Progress {
                        id,
                        downloaded: current_downloaded,
                        speed,
                    });
                    
                    last_downloaded = current_downloaded;
                    
                    // Si on a atteint la taille totale, arrêter le suivi
                    if total_size > 0 && current_downloaded >= total_size {
                        break;
                    }
                }
            }
        });
        
        // Exécuter le téléchargement
        let download_result = manager.start(task).await;
        
        // Arrêter le suivi de progression
        progress_task.abort();
        
        let _ = progress_tx.send(DownloadProgress::Merging { id });
        
        match download_result {
            Ok(_) => {
                let _ = progress_tx.send(DownloadProgress::Completed { id });
                Ok(())
            }
            Err(e) => {
                let _ = progress_tx.send(DownloadProgress::Error {
                    id,
                    error: e.to_string(),
                });
                Err(e)
            }
        }
    }
}

struct DownloadStats {
    active: usize,
    completed: usize,
}

