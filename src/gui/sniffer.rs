//! Composant UI pour le sniffer réseau.
//!
//! Permet de:
//! - Saisir l'URL à sniffer
//! - Configurer le filtre optionnel
//! - Visualiser les requêtes capturées en temps réel

use egui::{Ui, RichText, Color32, ScrollArea};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tokio::sync::Mutex;
use std::time::Duration;
use crate::sniffers::network_sniffer::{NetworkSniffer, NetworkEntry, open_browser};

/// Onglet du sniffer réseau
pub struct SnifferTab {
    target_url: String,
    filter: String,
    display_filter: String, // Filtre pour afficher les requêtes dans l'UI
    is_sniffing: bool,
    cancel_flag: Arc<AtomicBool>,
    captured_requests: Arc<Mutex<Vec<NetworkEntry>>>,
    error_message: Arc<Mutex<Option<String>>>,
    task_handle: Option<std::thread::JoinHandle<()>>,
}

impl Default for SnifferTab {
    fn default() -> Self {
        Self {
            target_url: String::new(),
            filter: String::new(),
            display_filter: String::new(),
            is_sniffing: false,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            captured_requests: Arc::new(Mutex::new(Vec::new())),
            error_message: Arc::new(Mutex::new(None)),
            task_handle: None,
        }
    }
}

impl SnifferTab {
    pub fn show(&mut self, ui: &mut Ui) {
        // Vérifier si le sniffing est terminé
        self.check_sniffing_status();
        
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
                        
                        // Bouton pour ouvrir l'URL dans le navigateur
                        if ui.add_enabled(
                            !self.target_url.is_empty(),
                            egui::Button::new(RichText::new("🔗 Ouvrir").size(12.0))
                        ).clicked() {
                            if let Err(e) = open_browser(&self.target_url) {
                                eprintln!("Erreur lors de l'ouverture du navigateur: {}", e);
                            }
                        }
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
                            if ui.button(RichText::new("⏹️ Arrêter").size(14.0).color(Color32::from_rgb(255, 100, 100)))
                                .clicked() {
                                self.stop_sniffing();
                            }
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
                    // Utiliser try_lock pour ne pas bloquer le thread UI
                    let requests = match self.captured_requests.try_lock() {
                        Ok(guard) => guard.clone(),
                        Err(_) => Vec::new(), // Si on ne peut pas acquérir le lock, utiliser des données vides
                    };
                    
                    // Afficher les erreurs (non-bloquant)
                    if let Ok(error_guard) = self.error_message.try_lock() {
                        if let Some(ref error) = *error_guard {
                            ui.vertical(|ui| {
                                ui.label(RichText::new("❌ Erreur lors du sniffing")
                                    .color(Color32::from_rgb(255, 100, 100))
                                    .strong()
                                    .size(16.0));
                                ui.add_space(4.0);
                                
                                // Afficher l'erreur avec formatage pour les sauts de ligne
                                let error_lines: Vec<&str> = error.split('\n').collect();
                                for line in error_lines {
                                    if !line.trim().is_empty() {
                                        ui.label(RichText::new(line)
                                            .color(Color32::from_rgb(255, 150, 150))
                                            .small());
                                    }
                                }
                                
                                ui.add_space(8.0);
                                ui.label(RichText::new("💡 Astuce: Assurez-vous que Chrome ou Chromium est installé et accessible")
                                    .color(Color32::YELLOW)
                                    .small());
                            });
                            ui.add_space(8.0);
                        }
                    }
                    
                    if requests.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(RichText::new("📭 Aucune requête capturée").size(18.0).color(Color32::GRAY));
                            ui.label(RichText::new("Les requêtes réseau apparaîtront ici lors du sniffing")
                                .color(Color32::DARK_GRAY));
                        });
                    } else {
                        // Filtre d'affichage
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("🔍 Filtrer l'affichage:").strong());
                            ui.text_edit_singleline(&mut self.display_filter)
                                .on_hover_text("Filtrer les requêtes affichées par URL, méthode, type, etc.");
                            if !self.display_filter.is_empty() {
                                if ui.button("✖️").clicked() {
                                    self.display_filter.clear();
                                }
                            }
                        });
                        ui.add_space(4.0);
                        
                        // Filtrer les requêtes selon le filtre d'affichage
                        let filtered_requests: Vec<_> = if self.display_filter.is_empty() {
                            requests.clone()
                        } else {
                            let filter_lower = self.display_filter.to_lowercase();
                            requests.iter()
                                .filter(|req| {
                                    req.url.to_lowercase().contains(&filter_lower) ||
                                    req.method.as_ref().map(|m| m.to_lowercase().contains(&filter_lower)).unwrap_or(false) ||
                                    req.resource_type.as_ref().map(|t| t.to_lowercase().contains(&filter_lower)).unwrap_or(false)
                                })
                                .cloned()
                                .collect()
                        };
                        
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{} requête(s) affichée(s) / {} total", filtered_requests.len(), requests.len()))
                                .color(Color32::GRAY)
                                .small());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("💾 Exporter JSON").clicked() {
                                    // L'export est déjà fait automatiquement par le sniffer
                                }
                                ui.label(RichText::new("(Exporté automatiquement dans network_output.json)")
                                    .small()
                                    .color(Color32::GRAY));
                            });
                        });
                        ui.add_space(4.0);
                        
                        for (_idx, request) in filtered_requests.iter().enumerate() {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(25, 25, 30))
                                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 50, 60)))
                                .rounding(egui::Rounding::same(6.0))
                                .inner_margin(egui::Margin::same(12.0))
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        // Première ligne: Méthode, Status, Type
                                        ui.horizontal(|ui| {
                                            if let Some(method) = &request.method {
                                                ui.label(RichText::new(method)
                                                    .color(Color32::from_rgb(100, 150, 255))
                                                    .strong()
                                                    .small());
                                            }
                                            
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
                                                    .strong()
                                                    .small());
                                            }
                                            
                                            if let Some(resource_type) = &request.resource_type {
                                                ui.label(RichText::new(format!("[{}]", resource_type))
                                                    .color(Color32::from_rgb(200, 200, 200))
                                                    .small());
                                            }
                                        });
                                        
                                        // URL
                                        ui.label(RichText::new(&request.url)
                                            .small()
                                            .color(Color32::from_rgb(220, 220, 220)));
                                        
                                        // Bouton pour ouvrir l'URL
                                        if ui.button(RichText::new("🔗 Ouvrir").size(10.0)).clicked() {
                                            if let Err(e) = open_browser(&request.url) {
                                                eprintln!("Erreur lors de l'ouverture: {}", e);
                                            }
                                        }
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
        self.cancel_flag.store(false, Ordering::Relaxed);
        
        // Réinitialiser les résultats
        let results = self.captured_requests.clone();
        let error_msg = self.error_message.clone();
        let cancel_flag = self.cancel_flag.clone();
        let target_url = self.target_url.clone();
        let filter = if self.filter.is_empty() { None } else { Some(self.filter.clone()) };
        
        // Lancer le sniffing dans un thread séparé avec mise à jour en temps réel
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
            rt.block_on(async move {
                let sniffer = Arc::new(NetworkSniffer::new(filter));
                let results_ref = results.clone();
                
                // Tâche de mise à jour périodique des résultats (pendant le sniffing)
                let sniffer_update = sniffer.clone();
                let update_task = tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        
                        // Récupérer les résultats actuels depuis le sniffer
                        let captured = sniffer_update.get_results().await;
                        let mut guard = results_ref.lock().await;
                        *guard = captured;
                        
                        // Vérifier si on doit arrêter
                        if cancel_flag.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                });
                
                // Lancer le sniffing directement (pas de spawn car il contient des types non-Send)
                let target_url_clone = target_url.clone();
                let sniff_result = sniffer.sniff(&target_url_clone).await;
                
                // Arrêter la tâche de mise à jour
                update_task.abort();
                
                // Récupérer les résultats finaux
                let captured = sniffer.get_results().await;
                let mut guard = results.lock().await;
                *guard = captured;
                
                // Gérer les erreurs
                if let Err(e) = sniff_result {
                    let mut guard = error_msg.lock().await;
                    *guard = Some(e.to_string());
                }
                
                // Marquer le sniffing comme terminé
                // Note: On ne peut pas mettre à jour is_sniffing directement ici car c'est dans un thread séparé
                // Le flag sera mis à jour via le mécanisme de stop_sniffing ou quand l'utilisateur vérifie l'état
            });
        });
        
        self.task_handle = Some(handle);
    }
    
    fn stop_sniffing(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
        self.is_sniffing = false;
        
        // Note: Le sniffer actuel ne peut pas être arrêté facilement
        // On peut améliorer ça en ajoutant un mécanisme d'annulation dans NetworkSniffer
        if let Some(handle) = self.task_handle.take() {
            // Attendre que le thread se termine (peut prendre un peu de temps)
            // On le fait dans un thread séparé pour ne pas bloquer l'UI
            let cancel_flag = self.cancel_flag.clone();
            std::thread::spawn(move || {
                let _ = handle.join();
                // Une fois terminé, on pourrait mettre à jour un flag, mais pour l'instant
                // on laisse l'utilisateur voir que c'est terminé via l'interface
            });
        }
    }
    
    /// Vérifie si le sniffing est terminé et met à jour le flag
    pub fn check_sniffing_status(&mut self) {
        if self.is_sniffing {
            if let Some(ref handle) = self.task_handle {
                if handle.is_finished() {
                    self.is_sniffing = false;
                }
            }
        }
    }
}

