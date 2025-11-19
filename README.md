# scrapes

`scrapes` est une boîte à outils asynchrone (Tokio) axée sur trois besoins complémentaires :
scraper le site FZTV pour retrouver des épisodes, télécharger efficacement de gros fichiers
multi-sources (HTTP Range, ffmpeg) et sniffer du trafic réseau via Chromium pour rétro‑ingénierie.

## Fonctionnalités clés

- **Scrapers FZTV** : exploration des saisons/épisodes, parsing résilient et enrichissement automatique
  des liens via `downloadmp4.php`.
- **Téléchargeur natif** (`src/downloader`) : découpe en chunks pré‑alloués, Range requests parallèles,
  reprise par marqueurs `.done` et fusion tamponnée.
- **Pont ffmpeg** (`src/ffmpeg`) : exécution supervisée de `ffmpeg` avec détection de blocage,
  redémarrage exponentiel, canal de progression et callbacks.
- **Sniffer réseau** (`src/sniffers/network_sniffer.rs`) : lance Chromium, intercepte requêtes/réponses CDP,
  filtre optionnel et export JSON.
- **Observabilité** : configuration `scrapes.toml`, logs via `tracing`/`tracing-subscriber`, sérialisation serde.

## Prérequis

- Rust 1.80+ (Edition 2024) et `cargo`.
- `ffmpeg` présent dans le `PATH`.
- Chrome ou Chromium compatible pour `chromiumoxide`.
- (Windows) PowerShell 7 recommandé pour les scripts; le projet fonctionne aussi sous Linux/macOS.

## Installation & exécution

```powershell
git clone https://github.com/<votre-compte>/scrapes.git
cd scrapes
cargo run
```

### Variables d’environnement utiles

- `RUST_LOG=debug,scrapes::downloader=trace` pour le téléchargeur.
- `SCRAPES_CONFIG=path/to/scrapes.toml` (option à ajouter si vous souhaitez rendre le chemin configurable).

## Configuration (`scrapes.toml`)

```toml
[logging]
filter = "info,scrapes::downloader=debug"

[cleanup]
remove_temp_files = true   # suppression après succès
remove_on_error = false    # suppression si erreur
```

- `logging.filter` : filtre passé à `tracing_subscriber::EnvFilter`. L’environnement `RUST_LOG`
  a priorité.
- `cleanup.remove_temp_files` : efface `*.part*` et marqueurs `.done` après fusion réussie.
- `cleanup.remove_on_error` : nettoie également en cas d’échec (désactivé par défaut pour debug).

## Aperçu des modules

| Module | Fichier | Responsabilités principales |
| --- | --- | --- |
| `downloader` | `src/downloader/*` | Calcul des segments (`DownloadTask`), préallocation disque, Range GET parallèles (`DownloadManager::start`), fusion (`utils::merge_chunks`). |
| `ffmpeg` | `src/ffmpeg/*` | Construction des commandes `ffmpeg`, parsing des sorties `-progress`, détection de blocage, callbacks. |
| `scrapers::fzscrape` | `src/scrapers/fzscrape/fztv_scraper.rs` | Découverte des saisons, scraping robuste des épisodes/qualités, ouverture navigateur pour debug, extraction des URLs finales. |
| `sniffers` | `src/sniffers/network_sniffer.rs` | Instrumentation Chromium CDP, collecte filtrée, export `network_output.json`. |
| `main.rs` | `src/main.rs` | Point d’entrée (actuellement minimal) pour orchestrer les services selon vos besoins. |

## Workflows typiques

### Téléchargement chunké

1. `DownloadManager::start` détecte `content-length`/`accept-ranges` via `HEAD`.
2. Prépare les chunks -> fichiers `output.part<i>` pré‑alloués (`utils::create_empty_file`).
3. Télécharge en parallèle (concurrence 8) avec `Range: bytes=start-end`.
4. Chaque chunk complété crée un marqueur `.done` pour la reprise.
5. Fusion séquentielle dans le fichier final puis nettoyage.

### Téléchargement via ffmpeg

1. `ffmpeg::download_*` construit un canal MPSC pour `FfmpegProgress`.
2. `download_with_ffmpeg` lance `ffmpeg -c copy -progress pipe:1`.
3. Les lignes `clé=valeur` alimentent la progression, un timeout (`stall_timeout`) tue le processus.
4. Redémarrage automatique jusqu’à `max_restarts`, renommage du `.tmp` en sortie lorsque terminé.

### Scraping FZTV

1. `FztvScraper::scrape_seasons` récupère la page principale et collecte les URLs relatives.
2. `scrape_episodes` applique une cascade de sélecteurs (`ul.list`, `div[class*=episode]`, etc.) pour tolérer les variations HTML.
3. `scrape_actual_download_link_fast` suit `episode.php -> downloadmp4.php -> liens textbox/input`.
4. `enrich_with_actual_links` traite en parallèle avec `Semaphore` (10 requêtes simultanées).

### Sniffing réseau

1. `NetworkSniffer::sniff` lance Chromium via `chromiumoxide`.
2. Écoute `EventRequestWillBeSent` et `EventResponseReceived`, applique un filtre optionnel.
3. Attendre la navigation et 5 s supplémentaires, exporter vers `network_output.json`.

## Exemples d’utilisation

### Scraper FZTV et enrichir les liens

```rust
use scrapes::scrapers::fzscrape::fztv_scraper::FztvScraper;

# async fn demo() -> anyhow::Result<()> {
let scraper = FztvScraper::new("https://www.fztvseries.mobi/".into());
let seasons = scraper.scrape_all("https://www.fztvseries.mobi/sermons/series").await?;
let enriched = scraper.enrich_with_actual_links(seasons).await?;
println!("{} saisons enrichies", enriched.len());
# Ok(())
# }
```

### Télécharger un fichier via downloader natif

```rust
use scrapes::downloader::download_to;
use std::path::PathBuf;

# async fn download() -> anyhow::Result<()> {
download_to(
    "https://example.com/file.bin".to_string(),
    PathBuf::from("file.bin"),
).await?;
# Ok(())
# }
```

### Contrôler `ffmpeg` avec un callback

```rust
use scrapes::ffmpeg::{self, DownloadOptions};
use std::time::Duration;

# async fn grab() -> Result<(), scrapes::ffmpeg::params::DownloadError> {
let options = DownloadOptions {
    stall_timeout: Duration::from_secs(30),
    auto_restart: true,
    max_restarts: 5,
};

ffmpeg::download_with_options(
    "https://cdn.example.com/video.m3u8",
    "episode.mp4",
    options,
    Some(|progress| {
        if let Some(ms) = progress.fields.get("out_time_ms") {
            println!("Position courante: {ms} ms");
        }
    }),
).await
# }
```

### Sniffer une page spécifique

```rust
use scrapes::sniffers::network_sniffer::NetworkSniffer;

# async fn sniff() -> anyhow::Result<()> {
let sniffer = NetworkSniffer::new(Some("m3u8".into()));
sniffer.sniff("https://example.com/player").await?;
let captured = sniffer.get_results().await;
println!("{} requêtes capturées", captured.len());
# Ok(())
# }
```

## Tests & qualité

- `cargo fmt` pour le formatage.
- `cargo clippy --all-targets --all-features` pour les lint Rust.
- `cargo test` exécute les tests unitaires (downloader/utils/scraper/ffmpeg/sniffer).
  - Les tests `#[ignore]` dans `network_sniffer.rs` nécessitent un navigateur installé; lancez‑les via
    `cargo test -- --ignored`.
- Les tests `ffmpeg` simulés vérifient surtout la logique (timeouts, options); pour valider
  l’intégration réelle, lancez `ffmpeg` manuellement.

## Développement futur

- Orchestration complète dans `main.rs` (CLI ou service gRPC) pour piloter les trois modules.
- Résilience accrue : reprise des téléchargements `ffmpeg`, stockage persistant du catalogue
  FZTV, UI pour le sniffer.
- Paramétrage du chemin `scrapes.toml` via variable d’environnement ou argument CLI.

## Dépannage

- **`accept-ranges` absent** : le gestionnaire retombe automatiquement sur un téléchargement séquentiel,
  mais il n’y aura pas de reprise ni de parallélisme.
- **`ffmpeg` introuvable** : vérifiez `ffmpeg -version` dans le terminal utilisé par `cargo run`.
- **Sniffer bloqué** : installez une version récente de Chrome/Chromium et assurez-vous que la sandbox
  n’est pas verrouillée (Linux : ajouter `--no-sandbox` via `BrowserConfig` si nécessaire).
- **Pages FZTV changeantes** : ajustez la cascade de sélecteurs dans
  `scrapers/fzscrape/fztv_scraper.rs` et pensez à activer les logs `info`.

---
Ce README couvre l’ensemble des composants actuels. Complétez‑le au fur et à mesure que l’entrée
`main.rs` orchestrera concrètement les scénarios (CLI, service, etc.).
