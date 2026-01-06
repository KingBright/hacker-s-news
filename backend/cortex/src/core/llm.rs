use anyhow::Result;
use reqwest::Client;
use serde_json::json;
use crate::core::config::LlmConfig;
use std::path::PathBuf;
use std::fs::OpenOptions;
use std::io::Write;
use chrono::Local;

use sha2::{Sha256, Digest};
use serde::{Serialize, Deserialize};
use std::sync::Arc;

const CACHE_TTL_SECS: i64 = 7 * 24 * 3600; // 7 days

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    created_at: i64,
    content: String,
}

pub struct LlmClient {
    client: Client,
    config: LlmConfig,
    audit_log_path: Option<PathBuf>,
    cache: Option<sled::Db>,
}

impl LlmClient {
    pub fn new(config: LlmConfig, audit_log_path: Option<PathBuf>, cache_path: Option<PathBuf>) -> Self {
        let cache = cache_path.and_then(|path| {
            sled::open(path).ok()
        });
        
        // Spawn Background GC
        if let Some(db) = &cache {
            let db_clone = db.clone();
            tokio::spawn(async move {
                log::info!("LLM Cache GC started.");
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await; // Check every hour
                    let now = Local::now().timestamp();
                    let mut count = 0;
                    
                    for item in db_clone.iter() {
                        if let Ok((key, value)) = item {
                             if let Ok(entry) = serde_json::from_slice::<CacheEntry>(&value) {
                                 if now - entry.created_at > CACHE_TTL_SECS {
                                     let _ = db_clone.remove(key);
                                     count += 1;
                                 }
                             }
                        }
                    }
                    if count > 0 {
                        log::info!("LLM Cache GC: Removed {} expired entries.", count);
                        let _ = db_clone.flush();
                    }
                }
            });
        }
        
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .unwrap_or_else(|_| Client::new()),
            config,
            audit_log_path,
            cache,
        }
    }

    fn log_audit(&self, stage: &str, content: &str) {
        if let Some(path) = &self.audit_log_path {
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
            let log_entry = format!("--------------------------------------------------\n[{}] [{}]\n{}\n", timestamp, stage, content);
            
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{}", log_entry);
            }
        }
    }

    pub async fn chat(&self, prompt: &str, skip_cache: bool) -> Result<String> {
        // 1. Check Cache
        let cache_key = if !skip_cache {
            let mut hasher = Sha256::new();
            hasher.update(prompt);
            Some(hex::encode(hasher.finalize()))
        } else {
            None
        };

        if let Some(key) = &cache_key {
            if let Some(db) = &self.cache {
                if let Ok(Some(cached_bytes)) = db.get(key) {
                    if let Ok(entry) = serde_json::from_slice::<CacheEntry>(&cached_bytes) {
                        let now = Local::now().timestamp();
                        if now - entry.created_at < CACHE_TTL_SECS {
                            log::info!("LLM Cache Hit! Key: {}", key);
                            self.log_audit("CACHE HIT", &entry.content);
                            return Ok(entry.content);
                        } else {
                            log::info!("LLM Cache Key {} expired.", key);
                            // Lazy delete? GC handles it, but we shouldn't use it.
                        }
                    }
                }
            }
        }

        let body = json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "stream": false,
            "max_tokens": 8192
        });

        // Assume api_url is like "http://localhost:1234/v1"
        let url = format!("{}/chat/completions", self.config.api_url.trim_end_matches('/'));

        log::info!("Sending LLM request to {} (Prompt Length: {} chars)", url, prompt.len());
        self.log_audit("INPUT", prompt);

        let res = match self.client.post(&url)
            .json(&body)
            .send()
            .await {
                Ok(response) => response,
                Err(e) => {
                     log::warn!("Failed to connect to LLM at {}: {}", url, e);
                     return Err(anyhow::anyhow!("LLM Connection Failed: {}", e));
                }
            };

        if !res.status().is_success() {
             let status = res.status();
             let error_text = res.text().await.unwrap_or_default();
             log::error!("LLM Error {}: {}", status, error_text);
             self.log_audit("ERROR", &format!("Status: {}, Body: {}", status, error_text));
             return Err(anyhow::anyhow!("LLM API Error {}: {}", status, error_text));
        }

        let response_json: serde_json::Value = res.json().await?;
        log::info!("Received LLM response (JSON parsed success).");
        
        // Parse OpenAI format: choices[0].message.content
        let mut summary = response_json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                log::warn!("Unexpected LLM response format: {:?}", response_json);
                "Failed to parse summary".to_string()
            });

        // Strip <think> tags if present
        if let Some(idx) = summary.find("</think>") {
             let thought = &summary[..idx + "</think>".len()];
             self.log_audit("THOUGHT", thought);
             summary = summary[idx + "</think>".len()..].trim().to_string();
        }

        self.log_audit("OUTPUT", &summary);

        // 2. Write to Cache
        if let Some(key) = &cache_key {
            if let Some(db) = &self.cache {
                 let entry = CacheEntry {
                     created_at: Local::now().timestamp(),
                     content: summary.clone(),
                 };
                 if let Ok(bytes) = serde_json::to_vec(&entry) {
                     if let Err(e) = db.insert(key, bytes) {
                         log::warn!("Failed to write to LLM cache: {}", e);
                     } else {
                         let _ = db.flush();
                     }
                 }
            }
        }

        Ok(summary)
    }
}
