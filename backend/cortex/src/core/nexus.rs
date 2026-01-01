use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::{Serialize, Deserialize};
use crate::core::config::NexusConfig;
use reqwest::multipart;

pub struct NexusClient {
    client: Client,
    config: NexusConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
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
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300)) // 5 minutes for large uploads
                .connect_timeout(std::time::Duration::from_secs(10)) // Fast fail on connection to allow retry
                .pool_idle_timeout(Some(std::time::Duration::from_secs(30))) // Close idle connections quickly
                .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
                .build()
                .unwrap_or_else(|_| Client::new()),
            config,
        }
    }

    pub async fn upload_file(&self, data: Vec<u8>, filename: &str, mime: &str) -> Result<String> {
        let part = multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str(mime)?;

        let form = multipart::Form::new().part("file", part);

        let url = format!("{}/api/internal/upload", self.config.api_url);
        let res = self.client.post(&url)
            .header("X-NEXUS-KEY", &self.config.auth_key)
            .multipart(form)
            .send()
            .await?;

        if !res.status().is_success() {
             let status = res.status();
             let text = res.text().await.unwrap_or_default();
             return Err(anyhow!("Failed to upload file: {} - {}", status, text));
        }

        let json: serde_json::Value = res.json().await?;
        let url = json["url"].as_str().ok_or_else(|| anyhow!("Invalid response"))?.to_string();
        Ok(url)
    }

    pub async fn upload_audio(&self, audio_data: Vec<u8>, filename: &str) -> Result<String> {
        self.upload_file(audio_data, filename, "audio/mpeg").await
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

    pub async fn check_urls(&self, urls: Vec<String>) -> Result<Vec<String>> {
        let url = format!("{}/api/internal/dedup/check", self.config.api_url);
        let res = self.client.post(&url)
            .header("X-NEXUS-KEY", &self.config.auth_key)
            .json(&serde_json::json!({ "urls": urls }))
            .send()
            .await?;
        
        if !res.status().is_success() {
            return Err(anyhow!("Failed to check urls: {}", res.status()));
        }
        
        let json: serde_json::Value = res.json().await?;
        let existing = serde_json::from_value(json["existing_urls"].clone())?;
        Ok(existing)
    }

    pub async fn mark_url(&self, url: &str, category: &str) -> Result<()> {
        let endpoint = format!("{}/api/internal/dedup/mark", self.config.api_url);
        let res = self.client.post(&endpoint)
            .header("X-NEXUS-KEY", &self.config.auth_key)
            .json(&serde_json::json!({ "url": url, "category": category }))
            .send()
            .await?;
            
        if !res.status().is_success() {
            return Err(anyhow!("Failed to mark url: {}", res.status()));
        }
        Ok(())
    }
}
