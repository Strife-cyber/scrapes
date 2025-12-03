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
    #[serde(skip)]
    pub status: DownloadStatus,
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

/// Onglet des téléchargements
pub struct DownloadsTab {
    downloads: Arc<Mutex<HashMap<DownloadId, DownloadItem>>>,
    new_url: String,
    new_path: String,
    next_id: Arc<Mutex<DownloadId>>,
    progress_rx: Option<mpsc::UnboundedReceiver<DownloadProgress>>,
    progress_tx: Option<mpsc::UnboundedSender<DownloadProgress>>,
    ctx: Option<Context>,
}

impl Default for DownloadsTab {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut tab = Self {
            downloads: Arc::new(Mutex::new(HashMap::new())),
            new_url: String::new(),
            new_path: String::new(),
            next_id: Arc::new(Mutex::new(0)),
            progress_rx: Some(rx),
            progress_tx: Some(tx),
            ctx: None,
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
    
    /// Traite les messages de progression reçus
    fn process_progress_updates(&mut self) {
        if let Some(ref mut rx) = self.progress_rx {
            while let Ok(progress) = rx.try_recv() {
                let mut downloads = self.downloads.blocking_lock();
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
                        DownloadProgress::Completed { .. } => {
                            download.status = DownloadStatus::Completed;
                            download.progress = 1.0;
                            download.speed = None;
                        }
                        DownloadProgress::Error { error, .. } => {
                            download.status = DownloadStatus::Error(error.clone());
                            download.error_message = Some(error);
                        }
                        DownloadProgress::Paused { .. } => {
                            download.status = DownloadStatus::Paused;
                        }
                        DownloadProgress::Cancelled { .. } => {
                            download.status = DownloadStatus::Cancelled;
                        }
                    }
                }
            }
        }
        
        // Sauvegarder périodiquement
        self.save_history();
        
        // Demander un repaint si nécessaire
        if let Some(ref ctx) = self.ctx {
            ctx.request_repaint();
        }
    }
    
    pub fn show(&mut self, ui: &mut Ui) {
        // Traiter les mises à jour de progression
        self.process_progress_updates();
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
                        ui.text_edit_singleline(&mut self.new_url)
                            .on_hover_text("URL du fichier à télécharger");
                    });
                    
                    ui.add_space(4.0);
                    
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Chemin:").strong());
                        ui.text_edit_singleline(&mut self.new_path)
                            .on_hover_text("Chemin de destination (ex: D:\\Videos\\file.mp4)");
                    });
                    
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
                        let downloads = self.downloads.blocking_lock();
                        downloads.values()
                            .filter(|d| matches!(d.status, DownloadStatus::Queued))
                            .count()
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
            
            // Liste des téléchargements avec scroll
            ui.heading("📋 Téléchargements");
            ui.add_space(4.0);
            
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let downloads_guard = self.downloads.blocking_lock();
                    let mut downloads: Vec<_> = downloads_guard.values().cloned().collect();
                    drop(downloads_guard);
                    
                    // Trier par ID (ordre d'ajout)
                    downloads.sort_by_key(|d| d.id);
                    
                    if downloads.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(RichText::new("📭 Aucun téléchargement").size(18.0).color(Color32::GRAY));
                            ui.label(RichText::new("Ajoutez un téléchargement ci-dessus pour commencer").color(Color32::DARK_GRAY));
                        });
                    } else {
                        for download in &downloads {
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
                                if ui.small_button("🔄").clicked() {
                                    self.restart_download(download.id);
                                }
                            }
                            _ => {}
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
        let downloads = self.downloads.blocking_lock();
        let active = downloads.values()
            .filter(|d| matches!(d.status, DownloadStatus::Downloading | DownloadStatus::Merging | DownloadStatus::Queued))
            .count();
        let completed = downloads.values()
            .filter(|d| matches!(d.status, DownloadStatus::Completed))
            .count();
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
        
        let mut downloads = self.downloads.blocking_lock();
        downloads.insert(id, item);
        drop(downloads);
        
        // Sauvegarder l'historique
        self.save_history();
        
        // Réinitialiser le formulaire
        self.new_url.clear();
        self.new_path.clear();
    }
    
    /// Charge l'historique depuis le fichier JSON
    fn load_history(&mut self) {
        if let Ok(content) = fs::read_to_string(HISTORY_FILE) {
            if let Ok(items) = serde_json::from_str::<Vec<DownloadItem>>(&content) {
                let mut downloads = self.downloads.blocking_lock();
                let mut max_id = 0;
                for mut item in items {
                    // Réinitialiser les champs non-sérialisables
                    item.cancel_flag = Arc::new(AtomicBool::new(false));
                    item.task_handle = Some(Arc::new(Mutex::new(None)));
                    
                    // Si le téléchargement était en cours, le remettre en file
                    if matches!(item.status, DownloadStatus::Downloading | DownloadStatus::Merging) {
                        item.status = DownloadStatus::Queued;
                    }
                    
                    max_id = max_id.max(item.id);
                    downloads.insert(item.id, item);
                }
                drop(downloads);
                
                // Mettre à jour le prochain ID
                let mut next_id = self.next_id.blocking_lock();
                *next_id = max_id + 1;
            }
        }
    }
    
    /// Sauvegarde l'historique dans le fichier JSON
    fn save_history(&self) {
        let downloads = self.downloads.blocking_lock();
        let items: Vec<_> = downloads.values()
            .filter(|d| !matches!(d.status, DownloadStatus::Cancelled))
            .cloned()
            .collect();
        drop(downloads);
        
        if let Ok(json) = serde_json::to_string_pretty(&items) {
            let _ = fs::write(HISTORY_FILE, json);
        }
    }
    
    /// Met en pause un téléchargement
    fn pause_download(&mut self, id: DownloadId) {
        let mut downloads = self.downloads.blocking_lock();
        if let Some(download) = downloads.get_mut(&id) {
            download.cancel_flag.store(true, Ordering::Relaxed);
            download.status = DownloadStatus::Paused;
        }
        drop(downloads);
        self.save_history();
        
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(DownloadProgress::Paused { id });
        }
    }
    
    /// Annule un téléchargement
    fn cancel_download(&mut self, id: DownloadId) {
        let mut downloads = self.downloads.blocking_lock();
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
        drop(downloads);
        self.save_history();
        
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(DownloadProgress::Cancelled { id });
        }
    }
    
    /// Reprend un téléchargement en pause
    fn resume_download(&mut self, id: DownloadId) {
        let mut downloads = self.downloads.blocking_lock();
        let download = downloads.get(&id).cloned();
        drop(downloads);
        
        if let Some(download) = download {
            if matches!(download.status, DownloadStatus::Paused | DownloadStatus::Queued) {
                let url = download.url.clone();
                let output = download.output_path.clone();
                let tx = self.progress_tx.clone().expect("Progress channel should exist");
                
                // Mettre à jour le statut
        {
            let downloads = self.downloads.blocking_lock();
            if let Some(d) = downloads.get(&id) {
                if !matches!(d.status, DownloadStatus::Paused | DownloadStatus::Queued) {
                    return;
                }
            } else {
                return;
            }
        }
        
        let mut downloads = self.downloads.blocking_lock();
        if let Some(d) = downloads.get_mut(&id) {
            d.status = DownloadStatus::Queued;
            d.cancel_flag.store(false, Ordering::Relaxed);
        }
        drop(downloads);
                
                // Relancer le téléchargement
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                    rt.block_on(async move {
                        let result = Self::run_download(id, url, output, tx.clone()).await;
                        if let Err(e) = result {
                            let _ = tx.send(DownloadProgress::Error {
                                id,
                                error: e.to_string(),
                            });
                        }
                    });
                });
            }
        }
    }
    
    /// Redémarre un téléchargement (après erreur ou annulation)
    fn restart_download(&mut self, id: DownloadId) {
        let mut downloads = self.downloads.blocking_lock();
        let download = downloads.get(&id).cloned();
        drop(downloads);
        
        if let Some(mut download) = download {
            // Réinitialiser l'état
            download.status = DownloadStatus::Queued;
            download.progress = 0.0;
            download.downloaded = 0;
            download.error_message = None;
            download.cancel_flag = Arc::new(AtomicBool::new(false));
            download.task_handle = Some(Arc::new(Mutex::new(None)));
            
            // Supprimer les fichiers part existants pour recommencer
            let output_dir = download.output_path.parent().unwrap_or(std::path::Path::new("."));
            let output_stem = download.output_path.file_stem().unwrap_or_else(|| std::ffi::OsStr::new("file"));
            
            if let Ok(entries) = std::fs::read_dir(output_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with(&format!("{}.part", output_stem.to_string_lossy())) {
                            let _ = std::fs::remove_file(&path);
                        }
                        if name.ends_with(".done") && name.starts_with(&format!("{}.part", output_stem.to_string_lossy())) {
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
            
        // Remettre dans la liste
        let mut downloads = self.downloads.blocking_lock();
        downloads.insert(id, download);
        drop(downloads);
            
            // Démarrer le téléchargement
            self.resume_download(id);
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
            
            // Mettre à jour le statut
            let mut downloads = self.downloads.blocking_lock();
            if let Some(d) = downloads.get_mut(&id) {
                d.status = DownloadStatus::Downloading;
            }
            drop(downloads);
            
            // Utiliser le runtime tokio global ou créer une nouvelle tâche
            // Pour egui, on utilise std::thread pour lancer le runtime
            let url_clone = url.clone();
            let output_clone = output.clone();
            let handle = std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                rt.block_on(async move {
                    let result = Self::run_download(id, url_clone, output_clone, tx.clone()).await;
                    if let Err(e) = result {
                        let _ = tx.send(DownloadProgress::Error {
                            id,
                            error: e.to_string(),
                        });
                    }
                });
            });
            
            // Stocker le handle pour pouvoir l'arrêter
            let mut downloads = self.downloads.blocking_lock();
            if let Some(d) = downloads.get_mut(&id) {
                if let Some(handle_arc) = &d.task_handle {
                    if let Ok(mut handle_opt) = handle_arc.try_lock() {
                        *handle_opt = Some(handle);
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

