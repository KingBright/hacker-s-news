use crate::core::nexus::{ItemPayload, NexusClient};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sled::Db;
use std::path::Path;
use std::sync::Arc;

// Retry configuration constants
const MAX_RETRY_COUNT: u8 = 5; // Maximum retry attempts before giving up

#[derive(Debug, Serialize, Deserialize)]
pub enum RetryAction {
    UploadAudio {
        filename: String,
        file_path: String, // Local path where audio is temporarily saved
    },
    PushItem(ItemPayload),
    MarkUrl {
        url: String,
        category: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct RetryEntry {
    action: RetryAction,
    created_at: i64, // Unix timestamp of creation
    retry_count: u8, // Number of retry attempts
    #[serde(default)] // Backward compatible: defaults to 0 for old entries
    last_retry_at: i64, // Unix timestamp of last retry attempt
}

pub struct RetryManager {
    db: Db,
    nexus: Arc<NexusClient>,
    cache_dir: String,
}

impl RetryManager {
    pub fn new(cache_dir: &str, nexus: Arc<NexusClient>) -> Result<Self> {
        let db = sled::open(Path::new(cache_dir).join("retry_db"))?;
        std::fs::create_dir_all(Path::new(cache_dir).join("audio_cache"))?;

        Ok(Self {
            db,
            nexus,
            cache_dir: cache_dir.to_string(),
        })
    }

    pub fn enqueue(&self, action: RetryAction) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let entry = RetryEntry {
            action,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as i64,
            retry_count: 0,
            last_retry_at: 0,
        };
        let val = serde_json::to_vec(&entry)?;
        self.db.insert(id.as_bytes(), val)?;
        self.db.flush()?;
        log::info!("Enqueued retry action: {:?}", id);
        Ok(())
    }

    pub async fn process_queue(&self) -> Result<()> {
        // Iterate over all items in DB
        // sled iter returns Result<(IVec, IVec)>
        for item in self.db.iter() {
            let (key, val) = item?;
            let mut entry: RetryEntry = match serde_json::from_slice(&val) {
                Ok(e) => e,
                Err(_) => {
                    // Legacy format: try to parse as RetryAction directly
                    log::warn!("Found legacy retry entry format, attempting migration");
                    match serde_json::from_slice::<RetryAction>(&val) {
                        Ok(action) => RetryEntry {
                            action,
                            created_at: 0,
                            retry_count: 0,
                            last_retry_at: 0,
                        },
                        Err(e) => {
                            log::error!("Failed to parse retry entry: {}", e);
                            continue;
                        }
                    }
                }
            };

            // Check if max retries exceeded
            if entry.retry_count >= MAX_RETRY_COUNT {
                log::warn!(
                    "Action {:?} exceeded max retries ({}). Removing from queue.",
                    String::from_utf8_lossy(&key),
                    MAX_RETRY_COUNT
                );
                self.db.remove(&key)?;

                // Cleanup local file if it was UploadAudio
                if let RetryAction::UploadAudio { file_path, .. } = &entry.action {
                    let _ = std::fs::remove_file(file_path);
                }
                continue;
            }

            // Implement exponential backoff: skip if retried too recently
            // Backoff: 1min, 5min, 15min, 1hr, 4hr based on retry count
            let backoff_secs = match entry.retry_count {
                0 => 0,
                1 => 60,    // 1 minute
                2 => 300,   // 5 minutes
                3 => 900,   // 15 minutes
                4 => 3600,  // 1 hour
                _ => 14400, // 4 hours
            };

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as i64;
            // Use last_retry_at for backoff, fall back to created_at if never retried
            let reference_time = if entry.last_retry_at > 0 {
                entry.last_retry_at
            } else {
                entry.created_at
            };
            let elapsed_since_last = now - reference_time;

            // Skip if not enough time has passed since last retry attempt
            if entry.retry_count > 0 && elapsed_since_last < backoff_secs as i64 {
                log::debug!(
                    "Skipping action {:?} - backoff not elapsed ({}s < {}s since last attempt)",
                    String::from_utf8_lossy(&key),
                    elapsed_since_last,
                    backoff_secs
                );
                continue;
            }

            log::info!(
                "Retrying action: {:?} (attempt {}/{})",
                String::from_utf8_lossy(&key),
                entry.retry_count + 1,
                MAX_RETRY_COUNT
            );

            match self.execute_action(&entry.action).await {
                Ok(_) => {
                    log::info!("Action succeeded. Removing from queue.");
                    self.db.remove(&key)?;

                    // Cleanup local file if it was UploadAudio
                    if let RetryAction::UploadAudio { file_path, .. } = &entry.action {
                        let _ = std::fs::remove_file(file_path);
                    }
                }
                Err(e) => {
                    // Increment retry count and record last retry time
                    entry.retry_count += 1;
                    entry.last_retry_at = now;
                    let updated_val = serde_json::to_vec(&entry)?;
                    self.db.insert(&key, updated_val)?;

                    log::warn!(
                        "Action failed (attempt {}/{}): {}. Will retry with backoff.",
                        entry.retry_count,
                        MAX_RETRY_COUNT,
                        e
                    );
                }
            }
        }
        self.db.flush()?;
        Ok(())
    }

    /// Prune old retry entries and entries that exceeded max retries
    pub fn prune_old_entries(&self, max_age_secs: u64) -> Result<usize> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        let mut count = 0;

        for item in self.db.iter() {
            let (key, val) = item?;

            let should_remove = if let Ok(entry) = serde_json::from_slice::<RetryEntry>(&val) {
                // Remove if too old OR exceeded max retries
                let is_too_old = now - entry.created_at > max_age_secs as i64;
                let is_max_retries = entry.retry_count >= MAX_RETRY_COUNT;
                is_too_old || is_max_retries
            } else {
                // Legacy entries have no timestamp, treat as very old
                true
            };

            if should_remove {
                // Cleanup local file if it was UploadAudio
                if let Ok(RetryEntry {
                    action: RetryAction::UploadAudio { file_path, .. },
                    ..
                }) = serde_json::from_slice::<RetryEntry>(&val)
                {
                    let _ = std::fs::remove_file(&file_path);
                }
                self.db.remove(&key)?;
                count += 1;
            }
        }

        if count > 0 {
            self.db.flush()?;
        }
        Ok(count)
    }

    async fn execute_action(&self, action: &RetryAction) -> Result<()> {
        match action {
            RetryAction::UploadAudio {
                filename,
                file_path,
            } => {
                let data = tokio::fs::read(file_path).await?;
                self.nexus.upload_audio(data, filename).await?;
            }
            RetryAction::PushItem(payload) => {
                // ItemPayload is Clone now
                self.nexus.push_item(payload.clone()).await?;
            }
            RetryAction::MarkUrl { url, category } => {
                self.nexus.mark_url(url, category).await?;
            }
        }
        Ok(())
    }

    // Helper to save audio to disk for retry
    pub async fn cache_audio(&self, data: &[u8], filename: &str) -> Result<String> {
        let path = Path::new(&self.cache_dir)
            .join("audio_cache")
            .join(filename);
        tokio::fs::write(&path, data).await?;
        Ok(path.to_string_lossy().to_string())
    }

    /// Get queue statistics (entry count and estimated size in bytes)
    pub fn get_queue_stats(&self) -> Result<(usize, u64)> {
        let mut count = 0;
        let mut total_size: u64 = 0;
        for item in self.db.iter() {
            let (key, val) = item?;
            count += 1;
            total_size += key.len() as u64 + val.len() as u64;
        }
        Ok((count, total_size))
    }
}
