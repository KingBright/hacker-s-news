use crate::core::config::LlmConfig;
use anyhow::Result;
use chrono::Local;
use reqwest::Client;
use serde_json::json;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use tokio::sync::broadcast;
use tokio::time::{sleep, Duration};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    shutdown_tx: broadcast::Sender<()>,
}

impl LlmClient {
    pub fn new(
        config: LlmConfig,
        audit_log_path: Option<PathBuf>,
        cache_path: Option<PathBuf>,
    ) -> Self {
        let cache = cache_path.and_then(|path| sled::open(path).ok());
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

        // Spawn Background GC with shutdown signal support
        if let Some(db) = &cache {
            let db_clone = db.clone();
            tokio::spawn(async move {
                log::info!("LLM Cache GC started.");
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));

                loop {
                    tokio::select! {
                        _ = shutdown_rx.recv() => {
                            log::info!("LLM Cache GC received shutdown signal. Exiting.");
                            break;
                        }
                        _ = interval.tick() => {
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
                    }
                }
            });
        }

        Self {
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(300))
                .no_proxy() // Disable environment variable proxies too, just in case
                .build()
                .unwrap_or_else(|_| Client::new()),
            config,
            audit_log_path,
            cache,
            shutdown_tx,
        }
    }

    /// Signal the GC task to shut down gracefully
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

    fn log_audit(&self, stage: &str, content: &str) {
        if let Some(base_path) = &self.audit_log_path {
            let now = Local::now();
            let timestamp = now.format("%Y-%m-%d %H:%M:%S");
            let date_suffix = now.format("%Y-%m-%d");
            let log_entry = format!(
                "--------------------------------------------------\n[{}] [{}]\n{}\n",
                timestamp, stage, content
            );

            // Write to date-stamped file (e.g. llm_audit_2026-03-15.log)
            let rotated_path = if let Some(stem) = base_path.file_stem() {
                let ext = base_path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "log".to_string());
                base_path.with_file_name(format!("{}_{}.{}", stem.to_string_lossy(), date_suffix, ext))
            } else {
                base_path.clone()
            };

            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&rotated_path) {
                let _ = writeln!(file, "{}", log_entry);
            }

            // Lazy cleanup: remove audit logs older than 7 days (check occasionally)
            // Only run cleanup at the start of a new day (when seconds < 60, ~once per minute window)
            if now.timestamp() % 3600 < 10 {
                Self::cleanup_old_audit_logs(base_path, 7);
            }
        }
    }

    /// Remove audit log files older than `keep_days` days
    fn cleanup_old_audit_logs(base_path: &PathBuf, keep_days: i64) {
        if let Some(parent) = base_path.parent() {
            if let Some(stem) = base_path.file_stem() {
                let prefix = stem.to_string_lossy();
                if let Ok(entries) = std::fs::read_dir(parent) {
                    let cutoff = Local::now() - chrono::Duration::days(keep_days);
                    let cutoff_str = cutoff.format("%Y-%m-%d").to_string();
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        // Match files like "llm_audit_2026-03-08.log"
                        if name.starts_with(&*prefix) && name.contains('_') {
                            // Extract date part
                            if let Some(date_part) = name.strip_prefix(&format!("{}_", prefix)) {
                                let date_str = date_part.trim_end_matches(".log");
                                if date_str.len() == 10 && date_str < cutoff_str.as_str() {
                                    let _ = std::fs::remove_file(entry.path());
                                    log::info!("Audit log rotation: removed old log {}", name);
                                }
                            }
                        }
                    }
                }
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
        let url = format!(
            "{}/chat/completions",
            self.config.api_url.trim_end_matches('/')
        );

        log::info!(
            "Sending LLM request to {} (Prompt Length: {} chars)",
            url,
            prompt.len()
        );
        self.log_audit("INPUT", prompt);

        // 4-level retry mechanism with different strategies
        let res = {
            let mut last_err: Option<String> = None;
            let mut response = None;
            let mut use_fallback_endpoint = false;

            for attempt in 0..7 {
                // 4-level retry strategy:
                // Level 0-1: Quick retries (3s) for light connection errors
                // Level 2-3: Delayed retries (30s) for service busy
                // Level 4-5: Fallback endpoint (120s) for persistent issues
                // Level 6: Final fallback (300s) if everything fails

                let delay = match attempt {
                    0 => Duration::from_secs(3),   // Quick retry
                    1 => Duration::from_secs(3),   // Quick retry
                    2 => Duration::from_secs(30),  // Delayed retry
                    3 => Duration::from_secs(30),  // Delayed retry
                    4 => Duration::from_secs(120), // Fallback endpoint
                    5 => Duration::from_secs(120), // Fallback endpoint
                    _ => Duration::from_secs(300), // Final fallback
                };

                let current_url = if attempt >= 4 && !use_fallback_endpoint {
                    if let Some(ref fb_url) = self.config.fallback_url {
                        if !fb_url.is_empty() {
                            use_fallback_endpoint = true;
                            let fallback_url = format!(
                                "{}/chat/completions",
                                fb_url.trim_end_matches('/')
                            );
                            log::warn!("Switching to fallback endpoint: {}", fallback_url);
                            fallback_url
                        } else {
                            url.clone()
                        }
                    } else {
                        url.clone()
                    }
                } else {
                    url.clone()
                };

                match self.client.post(&current_url).json(&body).send().await {
                    Ok(r) => {
                        response = Some(r);
                        if attempt > 0 {
                            log::info!("LLM connection succeeded on attempt {}/7", attempt + 1);
                        }
                        break;
                    }
                    Err(e) => {
                        last_err = Some(e.to_string());
                        if attempt < 6 {
                            log::warn!("LLM connection attempt {}/7 failed: {}. Retrying in {}s...",
                                attempt + 1, e, delay.as_secs());
                            sleep(delay).await;
                        } else {
                            log::error!("LLM connection failed after all 7 attempts: {}", e);
                        }
                    }
                }
            }

            match response {
                Some(r) => r,
                None => {
                    let e = last_err.unwrap_or_else(|| "Unknown connection error".to_string());
                    log::error!("Failed to connect to LLM at {} after all attempts: {}", url, e);
                    return Err(anyhow::anyhow!("LLM Connection Failed: {}", e));
                }
            }
        };

        if !res.status().is_success() {
            let status = res.status();
            let error_text = res.text().await.unwrap_or_default();
            log::error!("LLM Error {}: {}", status, error_text);
            self.log_audit(
                "ERROR",
                &format!("Status: {}, Body: {}", status, error_text),
            );
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

    /// Structured JSON output: sends a prompt with `response_format: json_schema`
    /// so LM Studio constrains the model to output valid JSON matching the schema.
    /// Returns the deserialized Rust type directly.
    pub async fn chat_json<T: serde::de::DeserializeOwned + schemars::JsonSchema>(
        &self,
        prompt: &str,
        schema_name: &str,
        skip_cache: bool,
    ) -> Result<T> {
        // 1. Check cache (same key scheme as chat())
        let cache_key = if !skip_cache {
            let mut hasher = Sha256::new();
            hasher.update(prompt);
            hasher.update(b"__json__");
            hasher.update(schema_name.as_bytes());
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
                            log::info!("LLM Cache Hit (JSON)! Key: {}", key);
                            self.log_audit("CACHE HIT (JSON)", &entry.content);
                            return serde_json::from_str::<T>(&entry.content)
                                .map_err(|e| anyhow::anyhow!("Cached JSON parse error: {}", e));
                        }
                    }
                }
            }
        }

        // 2. Generate JSON Schema from Rust type (used for json_schema mode and prompt enrichment)
        let schema = schemars::schema_for!(T);
        let schema_value = serde_json::to_value(&schema)?;

        // 3. Build request body based on json_mode config
        //    - "json_schema": OpenAI Structured Outputs (strictest, guaranteed schema compliance)
        //    - "json_object": OpenAI JSON mode (valid JSON, no schema enforcement)
        //    - "none" / other: plain text, schema hint in prompt
        let json_mode = self.config.json_mode.as_str();
        
        // For json_object and none modes, embed schema description in prompt
        // (json_schema mode enforces at API level, so no prompt hint needed)
        let content_with_hint = if json_mode != "json_schema" {
            // Build a compact schema description from the JSON Schema
            let schema_str = serde_json::to_string(&schema_value).unwrap_or_default();
            format!("{}\n\nRespond ONLY with a valid JSON object. Schema: {}", prompt, schema_str)
        } else {
            prompt.to_string()
        };

        let mut body = json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "user",
                    "content": content_with_hint
                }
            ],
            "stream": false,
            "max_tokens": 8192
        });

        // Add response_format based on mode
        match json_mode {
            "json_schema" => {
                body["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": schema_name,
                        "strict": true,
                        "schema": schema_value
                    }
                });
            }
            "json_object" => {
                body["response_format"] = json!({
                    "type": "json_object"
                });
            }
            _ => {
                // "none" mode: no response_format, rely on prompt hint
            }
        }

        let url = format!(
            "{}/chat/completions",
            self.config.api_url.trim_end_matches('/')
        );

        log::info!(
            "Sending structured JSON request to {} (schema: {}, prompt: {} chars)",
            url, schema_name, prompt.len()
        );
        self.log_audit("INPUT (JSON)", &format!("[Schema: {}]\n{}", schema_name, prompt));

        // 4. Retry mechanism (same as chat())
        let res = {
            let mut last_err: Option<String> = None;
            let mut response = None;
            let mut use_fallback_endpoint = false;

            for attempt in 0..7 {
                let delay = match attempt {
                    0 => Duration::from_secs(3),
                    1 => Duration::from_secs(3),
                    2 => Duration::from_secs(30),
                    3 => Duration::from_secs(30),
                    4 => Duration::from_secs(120),
                    5 => Duration::from_secs(120),
                    _ => Duration::from_secs(300),
                };

                let current_url = if attempt >= 4 && !use_fallback_endpoint {
                    if let Some(ref fb_url) = self.config.fallback_url {
                        if !fb_url.is_empty() {
                            use_fallback_endpoint = true;
                            let fallback_url = format!(
                                "{}/chat/completions",
                                fb_url.trim_end_matches('/')
                            );
                            log::warn!("Switching to fallback endpoint: {}", fallback_url);
                            fallback_url
                        } else {
                            url.clone()
                        }
                    } else {
                        url.clone()
                    }
                } else {
                    url.clone()
                };

                match self.client.post(&current_url).json(&body).send().await {
                    Ok(r) => {
                        response = Some(r);
                        if attempt > 0 {
                            log::info!("LLM connection succeeded on attempt {}/7", attempt + 1);
                        }
                        break;
                    }
                    Err(e) => {
                        last_err = Some(e.to_string());
                        if attempt < 6 {
                            log::warn!("LLM connection attempt {}/7 failed: {}. Retrying in {}s...",
                                attempt + 1, e, delay.as_secs());
                            sleep(delay).await;
                        } else {
                            log::error!("LLM connection failed after all 7 attempts: {}", e);
                        }
                    }
                }
            }

            match response {
                Some(r) => r,
                None => {
                    let e = last_err.unwrap_or_else(|| "Unknown connection error".to_string());
                    log::error!("Failed to connect to LLM at {} after all attempts: {}", url, e);
                    return Err(anyhow::anyhow!("LLM Connection Failed: {}", e));
                }
            }
        };

        if !res.status().is_success() {
            let status = res.status();
            let error_text = res.text().await.unwrap_or_default();
            log::error!("LLM Error {}: {}", status, error_text);
            self.log_audit("ERROR (JSON)", &format!("Status: {}, Body: {}", status, error_text));
            return Err(anyhow::anyhow!("LLM API Error {}: {}", status, error_text));
        }

        // 5. Parse response
        let response_json: serde_json::Value = res.json().await?;
        log::info!("Received structured JSON response.");

        let content = response_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("");

        // Strip <think> tags if present
        let json_content = if let Some(idx) = content.find("</think>") {
            let thought = &content[..idx + "</think>".len()];
            self.log_audit("THOUGHT (JSON)", thought);
            content[idx + "</think>".len()..].trim()
        } else {
            content.trim()
        };

        self.log_audit("OUTPUT (JSON)", json_content);

        // 6. Deserialize — try direct parse first, fall back to JSON extraction for "none" mode
        let result: T = match serde_json::from_str(json_content) {
            Ok(v) => v,
            Err(e) => {
                // Fallback: try to extract JSON from response (for "none" mode or imperfect output)
                let extracted = if let (Some(s), Some(e_pos)) = (json_content.find('{'), json_content.rfind('}')) {
                    if e_pos >= s { &json_content[s..=e_pos] } else { json_content }
                } else if let (Some(s), Some(e_pos)) = (json_content.find('['), json_content.rfind(']')) {
                    if e_pos >= s { &json_content[s..=e_pos] } else { json_content }
                } else {
                    json_content
                };
                
                serde_json::from_str(extracted).map_err(|_| {
                    log::error!("Structured JSON parse failed (schema: {}): {}. Content: {}", 
                        schema_name, e, json_content);
                    anyhow::anyhow!("Structured JSON parse error: {}", e)
                })?
            }
        };

        // 7. Write to cache
        if let Some(key) = &cache_key {
            if let Some(db) = &self.cache {
                let entry = CacheEntry {
                    created_at: Local::now().timestamp(),
                    content: json_content.to_string(),
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

        Ok(result)
    }

    /// Get cache statistics (entry count and estimated size in bytes)
    pub fn get_cache_stats(&self) -> Result<(usize, u64)> {
        if let Some(db) = &self.cache {
            let mut count = 0;
            let mut total_size: u64 = 0;
            for item in db.iter() {
                let (key, val) = item?;
                count += 1;
                total_size += key.len() as u64 + val.len() as u64;
            }
            Ok((count, total_size))
        } else {
            Ok((0, 0))
        }
    }
}
