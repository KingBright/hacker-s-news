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

// --- Daily Rotating Logger ---
// Writes logs to date-stamped files: cortex-YYYY-MM-DD.log
// Auto-cleans logs older than 30 days on startup.
mod rotating_log {
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex;

    pub struct RotatingLogger {
        log_dir: PathBuf,
        current_date: Mutex<String>,
        file: Mutex<Option<std::fs::File>>,
    }

    impl RotatingLogger {
        pub fn new(log_dir: &std::path::Path) -> Self {
            std::fs::create_dir_all(log_dir).ok();
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let file = Self::open_log_file(log_dir, &today);
            Self {
                log_dir: log_dir.to_path_buf(),
                current_date: Mutex::new(today),
                file: Mutex::new(file),
            }
        }

        fn open_log_file(log_dir: &std::path::Path, date: &str) -> Option<std::fs::File> {
            let path = log_dir.join(format!("cortex-{}.log", date));
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()
        }

        /// Delete log files older than `max_age_days`
        pub fn cleanup_old_logs(log_dir: &std::path::Path, max_age_days: i64) {
            let cutoff = chrono::Local::now() - chrono::Duration::days(max_age_days);
            let cutoff_str = cutoff.format("%Y-%m-%d").to_string();

            if let Ok(entries) = std::fs::read_dir(log_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    // Match cortex-YYYY-MM-DD.log pattern (len = 21)
                    if name.starts_with("cortex-") && name.ends_with(".log") && name.len() == 21 {
                        let date_part = &name[7..17]; // extract YYYY-MM-DD
                        if date_part < cutoff_str.as_str() {
                            if std::fs::remove_file(entry.path()).is_ok() {
                                eprintln!("[LOG CLEANUP] Removed old log: {}", name);
                            }
                        }
                    }
                }
            }
        }
    }

    impl log::Log for RotatingLogger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Info
        }

        fn log(&self, record: &log::Record) {
            if !self.enabled(record.metadata()) {
                return;
            }
            let now = chrono::Local::now();
            let today = now.format("%Y-%m-%d").to_string();
            let timestamp = now.format("%Y-%m-%dT%H:%M:%S%:z");
            let msg = format!(
                "[{}] [{}] [{}] {}\n",
                timestamp,
                record.level(),
                record.target(),
                record.args()
            );

            // Check if we need to rotate (new day)
            {
                let mut current = self.current_date.lock().unwrap();
                if *current != today {
                    *current = today.clone();
                    let mut file_guard = self.file.lock().unwrap();
                    *file_guard = Self::open_log_file(&self.log_dir, &today);
                }
            }

            // Write to file
            if let Ok(mut file_guard) = self.file.lock() {
                if let Some(ref mut f) = *file_guard {
                    let _ = f.write_all(msg.as_bytes());
                }
            }

            // Also echo to stdout/stderr for interactive debugging
            if record.level() <= log::Level::Warn {
                eprint!("{}", msg);
            } else {
                print!("{}", msg);
            }
        }

        fn flush(&self) {
            if let Ok(mut file_guard) = self.file.lock() {
                if let Some(ref mut f) = *file_guard {
                    let _ = f.flush();
                }
            }
        }
    }
}

/// Run the main service logic (shared between standalone and service modes)
async fn run_service() -> Result<()> {
    let app_data_dir = get_app_data_dir();

    // Initialize rotating logger
    let log_dir = app_data_dir.join("logs");
    let logger = rotating_log::RotatingLogger::new(&log_dir);
    rotating_log::RotatingLogger::cleanup_old_logs(&log_dir, 30);
    log::set_boxed_logger(Box::new(logger)).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    // Load Config
    let config_path = "config.toml";

    // Create a dummy config if not exists for first run ease
    if !std::path::Path::new(config_path).exists() {
        let dummy_config = r#"
[nexus]
api_url = "http://localhost:8899"
auth_key = "CHANGE_ME_NEXUS_KEY"

[llm]
model = "llama3"
api_url = "http://localhost:11434"

[tts]
engine = "voxcpm"

rss_feeds = ["https://news.ycombinator.com/rss"]

[[categories]]
name = "Tech"
description = "Technology news"

[curated_feed]
enabled = true
schedule_times = ["08:00"]
source_group = "karpathy_hn"
max_items_per_cycle = 20
max_age_days = 2
min_quality_score = 6
article_audio_enabled = true
article_audio_max_items_per_cycle = 3
weekly_digest_enabled = true
weekly_digest_schedule_times = ["18:00"]
weekly_digest_min_items = 3
weekly_digest_max_items = 12

[[curated_feed.feeds]]
name = "Karpathy Blog"
url = "https://karpathy.bearblog.dev/feed/"
kind = "rss"
tags = ["AI", "Programming"]
"#;
        std::fs::write(config_path, dummy_config)?;
    }

    let config = load_config(config_path)?;

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
    cortex::core::news::run_news_loop(config, llm, tts, nexus, retry_manager, cache_dir_str).await;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    run_service().await
}
