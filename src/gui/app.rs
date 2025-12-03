//! État principal de l'application et gestion de la boucle principale egui.
//!
//! Ce module gère:
//! - L'état global de l'application
//! - La navigation entre les différents onglets
//! - L'orchestration des composants UI

use egui::{CentralPanel, TopBottomPanel, Context, Visuals, Color32};
use crate::gui::downloads::DownloadsTab;
use crate::gui::scraper::ScraperTab;
use crate::gui::sniffer::SnifferTab;
use crate::gui::ffmpeg::FfmpegTab;

/// État principal de l'application
pub struct ScrapesApp {
    current_tab: Tab,
    downloads_tab: DownloadsTab,
    scraper_tab: ScraperTab,
    sniffer_tab: SnifferTab,
    ffmpeg_tab: FfmpegTab,
}

/// Onglets disponibles dans l'interface
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Downloads,
    Scraper,
    Sniffer,
    Ffmpeg,
}

impl Tab {
    fn name(&self) -> &'static str {
        match self {
            Tab::Downloads => "📥 Téléchargements",
            Tab::Scraper => "🔍 Scraper FZTV",
            Tab::Sniffer => "🌐 Sniffer Réseau",
            Tab::Ffmpeg => "🎬 FFmpeg",
        }
    }
}

impl Default for ScrapesApp {
    fn default() -> Self {
        Self {
            current_tab: Tab::Downloads,
            downloads_tab: DownloadsTab::default(),
            scraper_tab: ScraperTab::default(),
            sniffer_tab: SnifferTab::default(),
            ffmpeg_tab: FfmpegTab::default(),
        }
    }
}

impl eframe::App for ScrapesApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Configuration du style moderne
        self.configure_style(ctx);
        
        // Définir le contexte pour les mises à jour asynchrones
        self.downloads_tab.set_context(ctx.clone());

        // Barre de navigation supérieure
        TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🎬 Scrapes");
                ui.separator();
                
                // Boutons d'onglets
                ui.selectable_value(&mut self.current_tab, Tab::Downloads, Tab::Downloads.name());
                ui.selectable_value(&mut self.current_tab, Tab::Scraper, Tab::Scraper.name());
                ui.selectable_value(&mut self.current_tab, Tab::Sniffer, Tab::Sniffer.name());
                ui.selectable_value(&mut self.current_tab, Tab::Ffmpeg, Tab::Ffmpeg.name());
            });
        });

        // Contenu principal
        CentralPanel::default().show(ctx, |ui| {
            match self.current_tab {
                Tab::Downloads => self.downloads_tab.show(ui),
                Tab::Scraper => self.scraper_tab.show(ui),
                Tab::Sniffer => self.sniffer_tab.show(ui),
                Tab::Ffmpeg => self.ffmpeg_tab.show(ui),
            }
        });
    }
}

impl ScrapesApp {
    /// Configure le style moderne de l'interface
    fn configure_style(&self, ctx: &Context) {
        let mut style = (*ctx.style()).clone();
        
        // Couleurs modernes avec un thème sombre élégant
        style.visuals = Visuals::dark();
        style.visuals.override_text_color = Some(Color32::from_gray(240));
        style.visuals.window_fill = Color32::from_rgb(20, 20, 25);
        style.visuals.panel_fill = Color32::from_rgb(25, 25, 30);
        style.visuals.faint_bg_color = Color32::from_rgb(30, 30, 35);
        style.visuals.extreme_bg_color = Color32::from_rgb(15, 15, 20);
        
        // Couleurs d'accent modernes
        style.visuals.selection.bg_fill = Color32::from_rgb(100, 150, 255);
        style.visuals.hyperlink_color = Color32::from_rgb(100, 200, 255);
        
        // Espacement amélioré
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(10.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        
        // Polices plus lisses
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(24.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(14.0, egui::FontFamily::Proportional),
        );
        
        ctx.set_style(style);
    }
}

