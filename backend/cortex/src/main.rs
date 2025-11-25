use std::time::Duration;
use tokio::time;
use anyhow::Result;
use rss::Channel;
use chrono::DateTime;

mod core;

use core::config::load_config;
use core::llm::LlmClient;
use core::tts::TtsClient;
use core::nexus::{NexusClient, ItemPayload};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // Load Config
    // In a real app, path might be an argument
    let config_path = "config.toml";

    // Create a dummy config if not exists for first run ease
    if !std::path::Path::new(config_path).exists() {
        let dummy_config = r#"
[nexus]
api_url = "http://localhost:8080"
auth_key = "my-secret-key-123"

[llm]
model = "llama3"
api_url = "http://localhost:11434"

[tts]
model_path = "./zh_CN-huayan-medium.onnx"

[[sources]]
name = "Hacker News"
url = "https://news.ycombinator.com/rss"
interval_min = 60
tags = ["Tech", "Global"]
"#;
        std::fs::write(config_path, dummy_config)?;
    }

    let config = load_config(config_path)?;

    let llm = LlmClient::new(config.llm.clone());
    let tts = TtsClient::new(config.tts.clone());
    let nexus = NexusClient::new(config.nexus.clone());

    log::info!("Cortex started. Sources: {}", config.sources.len());

    let mut handles = vec![];

    for source in config.sources {
        let llm = LlmClient::new(config.llm.clone()); // simplistic clone
        let tts = TtsClient::new(config.tts.clone()); // simplistic clone
        let nexus = NexusClient::new(config.nexus.clone()); // simplistic clone

        let handle = tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(source.interval_min * 60));
            loop {
                interval.tick().await;
                log::info!("Fetching source: {}", source.name);

                match process_source(&source, &llm, &tts, &nexus).await {
                    Ok(_) => log::info!("Finished processing {}", source.name),
                    Err(e) => log::error!("Error processing {}: {}", source.name, e),
                }
            }
        });
        handles.push(handle);
    }

    futures::future::join_all(handles).await;

    Ok(())
}

async fn process_source(
    source: &core::config::SourceConfig,
    llm: &LlmClient,
    tts: &TtsClient,
    nexus: &NexusClient
) -> Result<()> {
    // 1. Fetch RSS
    let content = reqwest::get(&source.url).await?.bytes().await?;
    let channel = Channel::read_from(&content[..])?;

    for item in channel.items().iter().take(3) { // Limit to 3 latest items for now
        let title = item.title().unwrap_or("No Title").to_string();
        let link = item.link().unwrap_or("").to_string();
        let description = item.description().unwrap_or("").to_string();

        // Skip if link is empty or maybe check if already exists in Nexus?
        // Nexus API doesn't have "check exists" yet, we might want to add deduplication later.
        // For now, we process.

        log::info!("Processing item: {}", title);

        // 2. Summarize
        // Use description or content if available
        let text_to_summarize = if description.len() > 50 { description } else { title.clone() };
        let summary = llm.summarize(&text_to_summarize).await?;

        // 3. TTS
        let audio_data = tts.speak(&summary).await?;
        let audio_url = if !audio_data.is_empty() {
            let filename = format!("{}.mp3", uuid::Uuid::new_v4());
            match nexus.upload_audio(audio_data, &filename).await {
                Ok(url) => Some(url),
                Err(e) => {
                    log::warn!("Failed to upload audio: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // 4. Push to Nexus
        let publish_time = item.pub_date().and_then(|d| DateTime::parse_from_rfc2822(d).ok()).map(|dt| dt.timestamp());

        let payload = ItemPayload {
            title,
            summary: Some(summary),
            original_url: Some(link),
            cover_image_url: None, // RSS usually doesn't give easy cover image, skipping for now
            audio_url,
            publish_time,
        };

        nexus.push_item(payload).await?;
    }

    Ok(())
}
