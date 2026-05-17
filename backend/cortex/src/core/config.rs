use anyhow::Result;
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub nexus: NexusConfig,
    pub llm: LlmConfig,
    pub tts: TtsConfig,
    pub rss_feeds: Option<Vec<String>>, // Flat list of RSS URLs
    pub http_proxy: Option<String>,     // Optional HTTP proxy for RSS fetching
    pub categories: Option<Vec<CategoryDef>>, // Categories with descriptions for LLM classification
    pub curated_feed: Option<CuratedFeedConfig>,
    pub hosts: Option<Vec<Host>>,
    pub interval_min: Option<u64>,
    pub schedule_times: Option<Vec<String>>, // Format: "HH:MM"
    pub timezone_offset: Option<i32>,        // Offset from UTC in hours (e.g., 8 for CST)
}

#[derive(Debug, Deserialize, Clone)]
pub struct Host {
    pub name: String,
    pub voice: String,
    pub prompt_text: Option<String>,
    pub categories: Vec<String>,
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
    pub fallback_url: Option<String>, // Fallback endpoint URL
    /// JSON output mode: "json_schema" (strictest, default), "json_object" (wider compat), "none" (prompt-only)
    #[serde(default = "default_json_mode")]
    pub json_mode: String,
}

fn default_json_mode() -> String {
    "json_schema".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct TtsConfig {
    pub engine: Option<String>,

    pub voxcpm: Option<VoxCPMConfig>,
    pub qwen3: Option<Qwen3Config>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VoxCPMConfig {
    pub model_path: String,
    pub prompt_text: Option<String>,
    pub prompt_wav_path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Qwen3Config {
    pub model_dir: String,
    pub prompt_text: Option<String>,
    pub prompt_wav_path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CategoryDef {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CuratedFeedConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub schedule_times: Option<Vec<String>>,
    pub max_items_per_cycle: Option<usize>,
    pub max_age_days: Option<i64>,
    pub min_quality_score: Option<u8>,
    pub prefer_proxy: Option<bool>,
    pub article_audio_enabled: Option<bool>,
    pub article_audio_min_quality_score: Option<u8>,
    pub article_audio_max_items_per_cycle: Option<usize>,
    pub weekly_digest_enabled: Option<bool>,
    pub weekly_digest_schedule_times: Option<Vec<String>>,
    pub weekly_digest_min_items: Option<usize>,
    pub weekly_digest_max_items: Option<usize>,
    pub source_group: Option<String>,
    pub feeds: Option<Vec<CuratedFeedSource>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CuratedFeedSource {
    pub name: Option<String>,
    pub url: String,
    pub kind: Option<String>,
    pub tags: Option<Vec<String>>,
    pub source_group: Option<String>,
}

fn default_true() -> bool {
    true
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
