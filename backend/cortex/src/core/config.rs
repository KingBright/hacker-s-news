use serde::Deserialize;
use std::fs;
use anyhow::Result;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub nexus: NexusConfig,
    pub llm: LlmConfig,
    pub tts: TtsConfig,
    pub news: Option<Vec<NewsCategory>>,
    pub interval_min: Option<u64>,
    pub schedule_times: Option<Vec<String>>, // Format: "HH:MM"
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
    pub engine: Option<String>,

    pub voxcpm: Option<VoxCPMConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VoxCPMConfig {
    pub model_path: String,
    pub prompt_text: Option<String>,
    pub prompt_wav_path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NewsCategory {
    pub category: String,
    pub urls: Vec<String>,
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
