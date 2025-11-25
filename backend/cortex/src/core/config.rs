use serde::Deserialize;
use std::fs;
use anyhow::Result;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub nexus: NexusConfig,
    pub llm: LlmConfig,
    pub tts: TtsConfig,
    pub sources: Vec<SourceConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NexusConfig {
    pub api_url: String,
    pub auth_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LlmConfig {
    pub model: String,
    pub api_url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TtsConfig {
    pub model_path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SourceConfig {
    pub name: String,
    pub url: String,
    pub interval_min: u64,
    pub tags: Option<Vec<String>>,
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
