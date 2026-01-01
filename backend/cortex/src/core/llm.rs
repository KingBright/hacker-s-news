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
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| Client::new()),
            config,
        }
    }

    pub async fn chat(&self, prompt: &str) -> Result<String> {
        let body = json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "stream": false
        });

        // Assume api_url is like "http://localhost:1234/v1"
        let url = format!("{}/chat/completions", self.config.api_url.trim_end_matches('/'));

        log::info!("Sending LLM request to {}. Body: {}", url, body);

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
             return Err(anyhow::anyhow!("LLM API Error {}: {}", status, error_text));
        }

        let response_json: serde_json::Value = res.json().await?;
        log::info!("Received LLM response: {}", response_json);
        
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
             summary = summary[idx + "</think>".len()..].trim().to_string();
        }

        Ok(summary)
    }
}
