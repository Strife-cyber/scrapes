use std::time::Duration;
use std::collections::HashMap;

/// Événement de progression émis depuis `-progress pipe:1` de ffmpeg
#[derive(Debug, Clone)]
pub struct FfmpegProgress {
    pub fields: HashMap<String, String>
}

impl FfmpegProgress {
    /// Crée un nouveau FfmpegProgress avec les champs donnés
    #[inline]
    pub fn new(fields: HashMap<String, String>) -> Self {
        Self { fields }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadError {
    #[error("ffmpeg s'est terminé avec un statut non-zéro: {0}")]
    FfmpegExit(i32),
    #[error("erreur io: {0}")]
    Io(#[from] std::io::Error),
    #[error("autre: {0}")]
    Other(String),
}

/// Options contrôlant le comportement
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// nombre maximum de secondes sans progression avant de considérer qu'il y a blocage
    pub stall_timeout: Duration,
    /// s'il faut tenter des redémarrages automatiques en cas de blocage
    pub auto_restart: bool,
    /// nombre maximum de tentatives de redémarrage
    pub max_restarts: usize,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            stall_timeout: Duration::from_secs(20),
            auto_restart: true,
            max_restarts: 3,
        }
    }
}
