use std::collections::HashMap;

mod scrapers;
mod downloader;
mod ffmpeg;
mod sniffers;

use tokio::task;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}