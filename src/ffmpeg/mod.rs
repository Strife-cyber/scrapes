pub mod params;
pub mod downloader;

pub use params::{DownloadError, DownloadOptions, FfmpegProgress};

use std::path::Path;
use tokio::sync::mpsc;
use crate::ffmpeg::downloader::download_with_ffmpeg;

/// Télécharge une URL vers un fichier de sortie avec les options par défaut.
/// 
/// Cette fonction est la plus simple à utiliser. Elle utilise – Timeout de blocage : 20 secondes
/// - Redémarrage automatique : activé (max trois tentatives).
/// 
/// # Exemple
/// ```no_run
/// use scrapes::ffmpeg;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// ffmpeg::download("https://example.com/video.mp4", "output.mp4").await?;
/// # Ok(())
/// # }
/// ```
pub async fn download(
    input_url: impl AsRef<str>,
    output_path: impl AsRef<Path>,
) -> Result<(), DownloadError> {
    download_with_options(input_url, output_path, DownloadOptions::default(), None::<fn(&FfmpegProgress)>).await
}

/// Télécharge une URL avec un callback pour suivre la progression.
/// 
/// Le callback est appelé à chaque mise à jour de progression de ffmpeg.
/// 
/// # Exemple
/// ```no_run
/// use scrapes::ffmpeg;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// ffmpeg::download_with_progress(
///     "https://example.com/video.mp4",
///     "output.mp4",
///     |progress| {
///         if let Some(time) = progress.fields.get("out_time_ms") {
///             println!("Temps: {} ms", time);
///         }
///     }
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn download_with_progress<F>(
    input_url: impl AsRef<str>,
    output_path: impl AsRef<Path>,
    on_progress: F,
) -> Result<(), DownloadError>
where
    F: Fn(&FfmpegProgress) + Send + Sync + 'static,
{
    download_with_options(input_url, output_path, DownloadOptions::default(), Some(on_progress)).await
}

/// Télécharge une URL avec des options personnalisées et un callback optionnel de progression.
/// 
/// Cette fonction offre le contrôle maximal sur le processus de téléchargement.
/// 
/// # Exemple
/// ```no_run
/// use scrapes::ffmpeg::{self, DownloadOptions};
/// use std::time::Duration;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let options = DownloadOptions {
///     stall_timeout: Duration::from_secs(30),
///     auto_restart: true,
///     max_restarts: 5,
/// };
/// 
/// ffmpeg::download_with_options(
///     "https://example.com/video.mp4",
///     "output.mp4",
///     options,
///     Some(|progress| {
///         println!("Progression: {:?}", progress.fields);
///     })
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn download_with_options<F>(
    input_url: impl AsRef<str>,
    output_path: impl AsRef<Path>,
    options: DownloadOptions,
    on_progress: Option<F>,
) -> Result<(), DownloadError>
where
    F: Fn(&FfmpegProgress) + Send + Sync + 'static,
{
    let input_url = input_url.as_ref();
    let (progress_tx, mut progress_rx) = mpsc::channel(100);

    // Spawner une tâche pour gérer les callbacks de progression
    let callback_task = if let Some(callback) = on_progress {
        Some(tokio::spawn(async move {
            while let Some(progress) = progress_rx.recv().await {
                callback(&progress);
            }
        }))
    } else {
        // Si pas de callback, on consomme juste les messages pour éviter de bloquer
        Some(tokio::spawn(async move {
            while let Some(_) = progress_rx.recv().await {}
        }))
    };

    // Lancer le téléchargement
    // Le canal se ferme automatiquement quand progress_tx est drop (à la fin de download_with_ffmpeg)
    let result = download_with_ffmpeg(input_url, output_path, options, progress_tx).await;

    // Attendre que le callback ait fini de traiter tous les messages
    // Le canal se ferme quand progress_tx est drop, ce qui fait que progress_rx.recv() retourne None
    if let Some(task) = callback_task {
        // Attendre que la tâche se termine naturellement (quand le canal est fermé)
        let _ = task.await;
    }

    result
}
