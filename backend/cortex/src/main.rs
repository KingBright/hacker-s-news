use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use cortex::core::config::load_config;

/// Get the application data directory (cross-platform)
fn get_app_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".freshloop")
}

/// Run the main service logic (shared between standalone and service modes)
async fn run_service() -> Result<()> {
    // Custom Logger to split stdout/stderr
    struct SplitLogger;

    impl log::Log for SplitLogger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Info
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                let timestamp = chrono::Local::now().format("%Y-%m-%dT%H:%M:%SZ");
                let msg = format!(
                    "[{}] [{}] [{}] {}",
                    timestamp,
                    record.level(),
                    record.target(),
                    record.args()
                );

                // Error and Warn go to stderr (cortex.err.log)
                // Info and below go to stdout (cortex.out.log)
                if record.level() <= log::Level::Warn {
                    eprintln!("{}", msg);
                } else {
                    println!("{}", msg);
                }
            }
        }

        fn flush(&self) {}
    }

    log::set_boxed_logger(Box::new(SplitLogger)).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    // Load Config
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
        std::fs::write(config_path, dummy_config)?;
    }

    let config = load_config(config_path)?;

    let app_data_dir = get_app_data_dir();

    // LLM Audit Log & Cache Path
    let llm_log_path = app_data_dir.join("logs").join("llm_audit.log");
    let llm_cache_path = app_data_dir.join("cache").join("llm_cache");

    // Ensure log dir exists
    if let Some(parent) = llm_log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Ensure cache dir exists
    if let Some(parent) = llm_cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let llm = Arc::new(cortex::core::llm::LlmClient::new(
        config.llm.clone(),
        Some(llm_log_path),
        Some(llm_cache_path),
    ));
    let tts = Arc::new(cortex::core::tts::TtsClient::new(config.tts.clone()));
    let nexus = Arc::new(cortex::core::nexus::NexusClient::new(config.nexus.clone()));

    // Initialize Retry Manager
    let cache_dir = app_data_dir.join("cache");
    let cache_dir_str = cache_dir.to_string_lossy().to_string();
    let retry_manager = Arc::new(
        cortex::core::retry::RetryManager::new(&cache_dir_str, nexus.clone())
            .expect("Failed to init RetryManager"),
    );

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

    // Spawn Retry Prune Background Loop (weekly)
    let retry_mgr_prune = retry_manager.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(7 * 24 * 3600)); // Weekly
        interval.tick().await; // Skip first tick
        loop {
            interval.tick().await;
            match retry_mgr_prune.prune_old_entries(7 * 24 * 3600) {
                // 7 days TTL
                Ok(n) => {
                    if n > 0 {
                        log::info!("Retry Prune: Removed {} old entries", n);
                    }
                }
                Err(e) => log::warn!("Retry Prune failed: {}", e),
            }
        }
    });

    log::info!("Starting Cortex service...");

    // Run the main news loop
    cortex::core::news::run_news_loop(
        config,
        llm,
        tts,
        nexus,
        retry_manager,
        cache_dir_str,
    )
    .await;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    run_service().await
}
