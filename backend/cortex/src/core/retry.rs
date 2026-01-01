use anyhow::Result;
use sled::Db;
use serde::{Serialize, Deserialize};
use std::path::Path;
use crate::core::nexus::{NexusClient, ItemPayload};
use std::sync::Arc;

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
        let val = serde_json::to_vec(&action)?;
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
            let action: RetryAction = serde_json::from_slice(&val)?;
            
            log::info!("Retrying action: {:?}", String::from_utf8_lossy(&key));

            match self.execute_action(&action).await {
                Ok(_) => {
                    log::info!("Action succeeded. Removing from queue.");
                    self.db.remove(&key)?;
                    
                    // Cleanup local file if it was UploadAudio
                    if let RetryAction::UploadAudio { file_path, .. } = action {
                        let _ = std::fs::remove_file(file_path);
                    }
                },
                Err(e) => {
                    log::warn!("Action failed again: {}. Keeping in queue.", e);
                    // Continue to next item? Or stop? 
                    // Continue, as some might succeed (e.g. different endpoints)
                }
            }
        }
        self.db.flush()?;
        Ok(())
    }

    async fn execute_action(&self, action: &RetryAction) -> Result<()> {
        match action {
            RetryAction::UploadAudio { filename, file_path } => {
                let data = tokio::fs::read(file_path).await?;
                self.nexus.upload_audio(data, filename).await?;
            },
            RetryAction::PushItem(payload) => {
                // ItemPayload is Clone now
                self.nexus.push_item(payload.clone()).await?;
            },
            RetryAction::MarkUrl { url, category } => {
                self.nexus.mark_url(url, category).await?;
            }
        }
        Ok(())
    }

    // Helper to save audio to disk for retry
    pub async fn cache_audio(&self, data: &[u8], filename: &str) -> Result<String> {
        let path = Path::new(&self.cache_dir).join("audio_cache").join(filename);
        tokio::fs::write(&path, data).await?;
        Ok(path.to_string_lossy().to_string())
    }
}
