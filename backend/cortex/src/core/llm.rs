use anyhow::Result;
use reqwest::Client;
use serde_json::json;
use crate::core::config::LlmConfig;

pub struct LlmClient {
    client: Client,
    config: LlmConfig,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    pub async fn summarize(&self, text: &str) -> Result<String> {
        // Truncate text if too long to avoid token limits (simplistic approach)
        let truncated_text = if text.len() > 10000 {
            &text[..10000]
        } else {
            text
        };

        let prompt = format!(
            "Please summarize the following content into a concise paragraph (less than 200 words), focusing on the key points. Content: {}",
            truncated_text
        );

        let body = json!({
            "model": self.config.model,
            "prompt": prompt,
            "stream": false
        });

        let url = format!("{}/api/generate", self.config.api_url);

        // Mock implementation if can't connect (or if in test environment)
        // For now, we try to connect. If it fails, we might return a dummy summary for testing purposes?
        // Let's implement robust error handling.

        let res = match self.client.post(&url)
            .json(&body)
            .send()
            .await {
                Ok(response) => response,
                Err(e) => {
                     log::warn!("Failed to connect to LLM at {}: {}. Using mock summary.", url, e);
                     return Ok(format!("(Mock Summary) Summary generation failed. Original start: {:.100}...", text));
                }
            };

        if !res.status().is_success() {
             return Ok(format!("(Mock Summary) LLM Error {}. Original start: {:.100}...", res.status(), text));
        }

        let response_json: serde_json::Value = res.json().await?;
        let summary = response_json["response"].as_str().unwrap_or("Failed to parse summary").to_string();

        Ok(summary)
    }
}
