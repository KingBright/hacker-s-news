use crate::core::config::NexusConfig;
use anyhow::{anyhow, Result};
use reqwest::multipart;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Connection health status
#[derive(Debug, Clone)]
pub struct ConnectionHealth {
    pub is_healthy: bool,
    pub latency_ms: u64,
    pub last_check: std::time::SystemTime,
    pub error_count: u32,
    pub last_error: Option<String>,
}

pub struct NexusClient {
    client: Arc<RwLock<Client>>, // Use RwLock to allow client recreation for DNS refresh
    config: NexusConfig,
    health: Arc<RwLock<ConnectionHealth>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ItemPayload {
    pub id: Option<String>, // Added for fetching
    pub title: String,
    pub summary: Option<String>,
    pub original_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub audio_url: Option<String>,
    pub publish_time: Option<i64>,
    pub duration_sec: Option<i64>,
    pub sources: Option<Vec<SourceInfo>>,
    pub category: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct FeedItemContentPayload {
    pub original_html: Option<String>,
    pub reader_markdown: Option<String>,
    pub plain_text: Option<String>,
    pub compressed_markdown: Option<String>,
    pub audio_script: Option<String>,
    pub key_points_json: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FeedItemPayload {
    pub id: Option<String>,
    pub product_line: Option<String>,
    pub item_type: String,
    pub primary_mode: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub source_name: Option<String>,
    pub source_url: Option<String>,
    pub original_url: Option<String>,
    pub canonical_url: Option<String>,
    pub content_hash: Option<String>,
    pub publish_time: Option<i64>,
    pub has_audio: Option<bool>,
    pub audio_url: Option<String>,
    pub clear_audio: Option<bool>,
    pub duration_sec: Option<i64>,
    pub reading_time_min: Option<i64>,
    pub quality_score: Option<i32>,
    pub tags: Option<String>,
    pub status: Option<String>,
    pub content: Option<FeedItemContentPayload>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WeeklyDigestPayload {
    pub id: Option<String>,
    pub feed_item_id: Option<String>,
    pub week_start: i64,
    pub week_end: i64,
    pub title: String,
    pub digest_markdown: Option<String>,
    pub audio_script: Option<String>,
    pub audio_url: Option<String>,
    pub duration_sec: Option<i64>,
    pub included_item_ids_json: Option<String>,
    pub themes_json: Option<String>,
    pub status: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FeedItemPushResult {
    pub id: String,
    pub status: String,
}

impl NexusClient {
    pub fn new(config: NexusConfig) -> Self {
        let client = Self::build_client();
        let health = ConnectionHealth {
            is_healthy: true,
            latency_ms: 0,
            last_check: std::time::SystemTime::now(),
            error_count: 0,
            last_error: None,
        };

        Self {
            client: Arc::new(RwLock::new(client)),
            config,
            health: Arc::new(RwLock::new(health)),
        }
    }

    fn build_client() -> Client {
        Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // 5 minutes for large uploads
            .connect_timeout(std::time::Duration::from_secs(10)) // Fast fail on connection to allow retry
            .pool_idle_timeout(Some(std::time::Duration::from_secs(30))) // Close idle connections quickly
            .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    /// Refresh the HTTP client to bypass DNS cache
    pub async fn refresh_client(&self) {
        log::info!("[NexusClient] Refreshing HTTP client to bypass DNS cache");
        let new_client = Self::build_client();
        *self.client.write().await = new_client;
    }

    /// Health check with latency measurement
    pub async fn health_check(&self) -> Result<ConnectionHealth> {
        let start = std::time::Instant::now();
        let url = format!("{}/api/health", self.config.api_url);

        // Try to get a fresh client for health check (bypass DNS cache)
        let client = self.client.read().await.clone();

        match client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(res) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let is_healthy = res.status().is_success();

                let health = ConnectionHealth {
                    is_healthy,
                    latency_ms,
                    last_check: std::time::SystemTime::now(),
                    error_count: if is_healthy { 0 } else { 1 },
                    last_error: if is_healthy {
                        None
                    } else {
                        Some(format!("HTTP {}", res.status()))
                    },
                };

                *self.health.write().await = health.clone();
                Ok(health)
            }
            Err(e) => {
                let health = ConnectionHealth {
                    is_healthy: false,
                    latency_ms: start.elapsed().as_millis() as u64,
                    last_check: std::time::SystemTime::now(),
                    error_count: 1,
                    last_error: Some(e.to_string()),
                };

                *self.health.write().await = health.clone();
                Err(anyhow!("Health check failed: {}", e))
            }
        }
    }

    /// Get current health status
    pub async fn get_health(&self) -> ConnectionHealth {
        self.health.read().await.clone()
    }

    /// Execute request with retry and exponential backoff
    async fn request_with_retry<F, Fut>(&self, operation: F) -> Result<reqwest::Response>
    where
        F: Fn(Client) -> Fut,
        Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    {
        let max_retries = 3;
        let mut last_error = None;

        for attempt in 0..max_retries {
            // Get fresh client (this may use DNS cache, but we retry with fresh client each time)
            let client = self.client.read().await.clone();

            match operation(client).await {
                Ok(res) => {
                    if res.status().is_server_error() && attempt < max_retries - 1 {
                        log::warn!(
                            "[NexusClient] Server error on attempt {}, retrying...",
                            attempt + 1
                        );
                        last_error = Some(format!("HTTP {}", res.status()));
                    } else {
                        return Ok(res);
                    }
                }
                Err(e) => {
                    let is_connect = e.is_connect();
                    last_error = Some(e.to_string());
                    log::warn!(
                        "[NexusClient] Request failed on attempt {}: {}",
                        attempt + 1,
                        e
                    );

                    if attempt < max_retries - 1 {
                        // Exponential backoff: 1s, 2s, 4s
                        let delay = std::time::Duration::from_secs(2_u64.pow(attempt as u32));
                        log::info!("[NexusClient] Retrying in {:?}...", delay);
                        tokio::time::sleep(delay).await;

                        // On connection error, refresh client to bypass DNS cache
                        if is_connect {
                            self.refresh_client().await;
                        }
                    }
                }
            }
        }

        Err(anyhow!(
            "Request failed after {} attempts: {:?}",
            max_retries,
            last_error
        ))
    }

    pub async fn upload_file(&self, data: Vec<u8>, filename: &str, mime: &str) -> Result<String> {
        let url = format!("{}/api/internal/upload", self.config.api_url);
        let auth_key = self.config.auth_key.clone();
        let filename = filename.to_string();
        let mime = mime.to_string();

        // Validate mime string before entering retry loop to fail fast
        let _ = multipart::Part::bytes(vec![0u8])
            .mime_str(&mime)
            .map_err(|e| anyhow!("Invalid MIME type '{}': {}", mime, e))?;

        let res = self
            .request_with_retry(move |client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                let filename = filename.clone();
                let mime = mime.clone();
                let data = data.clone();
                async move {
                    let part = multipart::Part::bytes(data)
                        .file_name(filename)
                        .mime_str(&mime)
                        .expect("MIME already validated");
                    let form = multipart::Form::new().part("file", part);
                    client
                        .post(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .multipart(form)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to upload file: {} - {}", status, text));
        }

        let json: serde_json::Value = res.json().await?;
        let url = json["url"]
            .as_str()
            .ok_or_else(|| anyhow!("Invalid response"))?
            .to_string();
        Ok(url)
    }

    pub async fn upload_audio(&self, audio_data: Vec<u8>, filename: &str) -> Result<String> {
        self.upload_file(audio_data, filename, "audio/mpeg").await
    }

    pub async fn push_item(&self, item: ItemPayload) -> Result<String> {
        let url = format!("{}/api/internal/items", self.config.api_url);
        let auth_key = self.config.auth_key.clone();
        let item_json = serde_json::to_value(&item)?;

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                let item_json = item_json.clone();
                async move {
                    client
                        .post(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .json(&item_json)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!("Failed to push item: {}", res.status()));
        }

        let json: serde_json::Value = res.json().await.unwrap_or(serde_json::json!({}));
        let item_id = json["id"].as_str().unwrap_or("unknown").to_string();
        Ok(item_id)
    }

    pub async fn push_item_multipart(
        &self,
        item: ItemPayload,
        audio_data: Option<Vec<u8>>,
    ) -> Result<String> {
        let url = format!("{}/api/internal/items/multipart", self.config.api_url);
        let auth_key = self.config.auth_key.clone();
        let item_json = serde_json::to_string(&item)?;

        let res = self
            .request_with_retry(move |client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                let item_json = item_json.clone();
                let audio_data = audio_data.clone();
                async move {
                    let mut form = multipart::Form::new().text("payload", item_json);

                    if let Some(audio) = audio_data {
                        let part = multipart::Part::bytes(audio)
                            .file_name("audio.mp3")
                            .mime_str("audio/mpeg")
                            .expect("audio/mpeg is a valid MIME type");
                        form = form.part("file", part);
                    }

                    client
                        .post(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .multipart(form)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Failed to push multipart item: {} - {}",
                status,
                text
            ));
        }

        let json: serde_json::Value = res.json().await.unwrap_or(serde_json::json!({}));
        let item_id = json["id"].as_str().unwrap_or("unknown").to_string();
        Ok(item_id)
    }

    pub async fn push_feed_item(&self, item: FeedItemPayload) -> Result<FeedItemPushResult> {
        let url = format!("{}/api/internal/feed/items", self.config.api_url);
        let auth_key = self.config.auth_key.clone();
        let item_json = serde_json::to_value(&item)?;

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                let item_json = item_json.clone();
                async move {
                    client
                        .post(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .json(&item_json)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to push feed item: {} - {}", status, text));
        }

        let json: serde_json::Value = res.json().await.unwrap_or(serde_json::json!({}));
        Ok(FeedItemPushResult {
            id: json["id"].as_str().unwrap_or("unknown").to_string(),
            status: json["status"].as_str().unwrap_or("unknown").to_string(),
        })
    }

    pub async fn push_weekly_digest(&self, digest: WeeklyDigestPayload) -> Result<String> {
        let url = format!("{}/api/internal/feed/weeklies", self.config.api_url);
        let auth_key = self.config.auth_key.clone();
        let digest_json = serde_json::to_value(&digest)?;

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                let digest_json = digest_json.clone();
                async move {
                    client
                        .post(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .json(&digest_json)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Failed to push weekly digest: {} - {}",
                status,
                text
            ));
        }

        let json: serde_json::Value = res.json().await.unwrap_or(serde_json::json!({}));
        Ok(json["id"].as_str().unwrap_or("unknown").to_string())
    }

    pub async fn fetch_feed_items(
        &self,
        product_line: &str,
        item_type: &str,
        limit: u32,
    ) -> Result<Vec<FeedItemPayload>> {
        let limit = limit.clamp(1, 100);
        let url = format!(
            "{}/api/feed/items?product_line={}&item_type={}&limit={}",
            self.config.api_url, product_line, item_type, limit
        );

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                async move { client.get(&url).send().await }
            })
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!("Failed to fetch feed items: {}", res.status()));
        }

        Ok(res.json().await?)
    }

    pub async fn fetch_feed_item_content(
        &self,
        item_id: &str,
    ) -> Result<Option<FeedItemContentPayload>> {
        let url = format!("{}/api/feed/items/{}/content", self.config.api_url, item_id);

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                async move { client.get(&url).send().await }
            })
            .await?;

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(anyhow!(
                "Failed to fetch feed item content: {}",
                res.status()
            ));
        }

        Ok(Some(res.json().await?))
    }

    pub async fn fetch_weekly_digests(&self) -> Result<Vec<WeeklyDigestPayload>> {
        let url = format!("{}/api/feed/weeklies", self.config.api_url);

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                async move { client.get(&url).send().await }
            })
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!("Failed to fetch weekly digests: {}", res.status()));
        }

        Ok(res.json().await?)
    }

    pub async fn check_urls(&self, urls: Vec<String>) -> Result<Vec<String>> {
        let url = format!("{}/api/internal/dedup/check", self.config.api_url);
        let payload = serde_json::json!({ "urls": urls });
        let auth_key = self.config.auth_key.clone();

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                let payload = payload.clone();
                async move {
                    client
                        .post(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .json(&payload)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!("Failed to check urls: {}", res.status()));
        }

        let json: serde_json::Value = res.json().await?;
        let existing = serde_json::from_value(json["existing_urls"].clone())?;
        Ok(existing)
    }

    pub async fn mark_url(&self, url_str: &str, category: &str) -> Result<()> {
        let url = format!("{}/api/internal/dedup/mark", self.config.api_url);
        let payload = serde_json::json!({ "url": url_str, "category": category });
        let auth_key = self.config.auth_key.clone();

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                let payload = payload.clone();
                async move {
                    client
                        .post(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .json(&payload)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!("Failed to mark url: {}", res.status()));
        }
        Ok(())
    }

    pub async fn fetch_pending_jobs(&self) -> Result<Vec<ItemPayload>> {
        let url = format!("{}/api/internal/items/pending", self.config.api_url);
        let auth_key = self.config.auth_key.clone();

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                async move {
                    client
                        .get(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!("Failed to fetch pending jobs: {}", res.status()));
        }

        let items: Vec<serde_json::Value> = res.json().await?;
        let payloads = items
            .into_iter()
            .map(|v| ItemPayload {
                id: v["id"].as_str().map(|s| s.to_string()),
                title: v["title"].as_str().unwrap_or_default().to_string(),
                summary: v["summary"].as_str().map(|s| s.to_string()),
                original_url: v["original_url"].as_str().map(|s| s.to_string()),
                cover_image_url: v["cover_image_url"].as_str().map(|s| s.to_string()),
                audio_url: v["audio_url"].as_str().map(|s| s.to_string()),
                publish_time: v["publish_time"].as_i64(),
                duration_sec: v["duration_sec"].as_i64(),
                sources: None,
                category: v["category"].as_str().map(|s| s.to_string()),
            })
            .collect();

        Ok(payloads)
    }

    pub async fn fetch_recent_items(&self, limit: u32) -> Result<Vec<ItemPayload>> {
        let url = format!("{}/api/items?limit={}", self.config.api_url, limit);

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                async move { client.get(&url).send().await }
            })
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!("Failed to fetch recent items: {}", res.status()));
        }

        let items: Vec<serde_json::Value> = res.json().await?;
        let payloads = items
            .into_iter()
            .map(|v| ItemPayload {
                id: v["id"].as_str().map(|s| s.to_string()),
                title: v["title"].as_str().unwrap_or_default().to_string(),
                summary: v["summary"].as_str().map(|s| s.to_string()),
                original_url: v["original_url"].as_str().map(|s| s.to_string()),
                cover_image_url: v["cover_image_url"].as_str().map(|s| s.to_string()),
                audio_url: v["audio_url"].as_str().map(|s| s.to_string()),
                publish_time: v["publish_time"].as_i64(),
                duration_sec: v["duration_sec"].as_i64(),
                sources: None,
                category: v["category"].as_str().map(|s| s.to_string()),
            })
            .collect();
        Ok(payloads)
    }

    pub async fn complete_job(
        &self,
        id: &str,
        audio_url_str: &str,
        summary: &str,
        duration_sec: Option<i64>,
    ) -> Result<()> {
        let url = format!("{}/api/internal/items/{}/complete", self.config.api_url, id);
        let payload = serde_json::json!({
            "audio_url": audio_url_str,
            "summary": summary,
            "duration_sec": duration_sec,
            "publish_time": chrono::Utc::now().timestamp()
        });
        let auth_key = self.config.auth_key.clone();

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                let payload = payload.clone();
                async move {
                    client
                        .post(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .json(&payload)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!("Failed to complete job: {}", res.status()));
        }
        Ok(())
    }

    /// Push source articles for an item
    pub async fn push_sources(&self, item_id: &str, sources: Vec<SourceInfo>) -> Result<()> {
        let url = format!(
            "{}/api/internal/items/{}/sources",
            self.config.api_url, item_id
        );
        let payload = serde_json::json!({
            "sources": sources
        });
        let auth_key = self.config.auth_key.clone();

        let res = self
            .request_with_retry(|client| {
                let url = url.clone();
                let auth_key = auth_key.clone();
                let payload = payload.clone();
                async move {
                    client
                        .post(&url)
                        .header("X-NEXUS-KEY", &auth_key)
                        .json(&payload)
                        .send()
                        .await
                }
            })
            .await?;

        if !res.status().is_success() {
            log::warn!("Failed to push sources: {}", res.status());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceInfo {
    pub url: String,
    pub title: String,
    pub summary: String,
}
