
use anyhow::Result;
use std::sync::Arc;

use cortex::core::config::{load_config, NewsCategory};
use cortex::core::llm::LlmClient;
use cortex::core::tts::TtsClient;
use cortex::core::nexus::{NexusClient, ItemPayload};

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
api_url = "http://localhost:8899"
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
        std::fs::write(&config_path, dummy_config)?;
    }

    let config = load_config(&config_path)?;

    let llm = Arc::new(cortex::core::llm::LlmClient::new(config.llm.clone()));
    let tts = Arc::new(cortex::core::tts::TtsClient::new(config.tts.clone()));
    let nexus = Arc::new(cortex::core::nexus::NexusClient::new(config.nexus.clone()));
    
    // Initialize Retry Manager
    let home_dir = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let cache_dir = format!("{}/.freshloop/cache", home_dir);
    let retry_manager = Arc::new(cortex::core::retry::RetryManager::new(&cache_dir, nexus.clone()).expect("Failed to init RetryManager"));

    // Spawn Retry Background Loop
    let retry_mgr_clone = retry_manager.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300)); // Retry every 5 mins
        loop {
            interval.tick().await;
            if let Err(e) = retry_mgr_clone.process_queue().await {
                log::error!("Error processing retry queue: {}", e);
            }
        }
    });

    log::info!("Starting Cortex service...");

    // Run the main news loop
    cortex::core::news::run_news_loop(config, llm, tts, nexus, retry_manager).await;

    Ok(())
}
