mod scrapers;
mod downloader;
mod ffmpeg;
mod sniffers;
mod gui;

use gui::ScrapesApp;

fn main() -> eframe::Result<()> {
    // Initialiser le logging
    downloader::init_logging();
    
    // Configuration de la fenêtre
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Scrapes - Gestionnaire de Téléchargements"),
        ..Default::default()
    };
    
    // Lancer l'application
    eframe::run_native(
        "Scrapes",
        options,
        Box::new(|_cc| Ok(Box::new(ScrapesApp::default()))),
    )
}