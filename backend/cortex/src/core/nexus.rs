use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::Serialize;
use crate::core::config::NexusConfig;
use reqwest::multipart;

pub struct NexusClient {
    client: Client,
    config: NexusConfig,
}

#[derive(Serialize)]
pub struct ItemPayload {
    pub title: String,
    pub summary: Option<String>,
    pub original_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub audio_url: Option<String>,
    pub publish_time: Option<i64>,
}

impl NexusClient {
    pub fn new(config: NexusConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    pub async fn upload_audio(&self, audio_data: Vec<u8>, filename: &str) -> Result<String> {
        let part = multipart::Part::bytes(audio_data)
            .file_name(filename.to_string())
            .mime_str("audio/mpeg")?;

        let form = multipart::Form::new().part("file", part);

        let url = format!("{}/api/internal/upload", self.config.api_url);
        let res = self.client.post(&url)
            .multipart(form)
            .send()
            .await?;

        if !res.status().is_success() {
             return Err(anyhow!("Failed to upload audio: {}", res.status()));
        }

        let json: serde_json::Value = res.json().await?;
        let url = json["url"].as_str().ok_or_else(|| anyhow!("Invalid response"))?.to_string();
        Ok(url)
    }

    pub async fn push_item(&self, item: ItemPayload) -> Result<()> {
        let url = format!("{}/api/internal/items", self.config.api_url);
        let res = self.client.post(&url)
            .header("X-NEXUS-KEY", &self.config.auth_key)
            .json(&item)
            .send()
            .await?;

        if !res.status().is_success() {
             return Err(anyhow!("Failed to push item: {}", res.status()));
        }

        Ok(())
    }
}
