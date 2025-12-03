//! Module d'interface graphique utilisateur avec egui.
//!
//! Ce module fournit une interface graphique moderne et fluide pour:
//! - Gestion des téléchargements avec progression en temps réel
//! - Scraping FZTV avec visualisation des saisons/épisodes
//! - Sniffing réseau avec affichage des requêtes capturées
//! - Téléchargements FFmpeg avec suivi de progression
//!
//! Architecture:
//! - `app.rs`: État principal de l'application et boucle principale
//! - `downloads.rs`: Composant UI pour les téléchargements
//! - `scraper.rs`: Composant UI pour le scraper FZTV
//! - `sniffer.rs`: Composant UI pour le sniffer réseau
//! - `ffmpeg.rs`: Composant UI pour les téléchargements FFmpeg

mod app;
mod downloads;
mod scraper;
mod sniffer;
mod ffmpeg;

pub use app::ScrapesApp;

