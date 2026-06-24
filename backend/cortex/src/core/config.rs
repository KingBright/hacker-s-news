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
    pub loop_preferences: Option<LoopPreferencesConfig>,
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
    pub device: Option<String>,
    pub keep_engine_loaded: Option<bool>,
    pub memory_pressure_relief: Option<bool>,
    pub process_isolation: Option<bool>,
    pub worker_memory_limit_mb: Option<u64>,
    pub worker_idle_timeout_secs: Option<u64>,
    pub worker_timeout_secs: Option<u64>,

    pub voxcpm: Option<VoxCPMConfig>,
    pub qwen3: Option<Qwen3Config>,
    pub magictts: Option<MagicTtsConfig>,
    pub moss: Option<MossTtsConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MagicTtsConfig {
    pub model_dir: String,
    pub prompt_text: Option<String>,
    pub prompt_wav_path: Option<String>,
    pub vocab_path: Option<String>,
    pub steps: Option<usize>,
    pub cfg_strength: Option<f64>,
    pub default_content_ms: Option<f64>,
    pub default_punct_ms: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MossTtsConfig {
    pub model_dir: String,
    pub prompt_text: Option<String>,
    pub prompt_wav_path: Option<String>,
    pub sample_mode: Option<String>,
    pub text_temperature: Option<f64>,
    pub text_top_p: Option<f64>,
    pub text_top_k: Option<usize>,
    pub audio_temperature: Option<f64>,
    pub audio_top_p: Option<f64>,
    pub audio_top_k: Option<usize>,
    pub audio_repetition_penalty: Option<f64>,
    pub max_new_frames: Option<usize>,
    pub voice_clone_max_text_tokens: Option<usize>,
    pub seed: Option<u64>,
    pub intra_threads: Option<usize>,
    pub inter_threads: Option<usize>,
    pub chunk_max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VoxCPMConfig {
    pub model_path: String,
    pub prompt_text: Option<String>,
    pub prompt_wav_path: Option<String>,
    pub control_instruction: Option<String>,
    pub min_len: Option<usize>,
    pub max_len: Option<usize>,
    pub inference_timesteps: Option<usize>,
    pub cfg_value: Option<f64>,
    pub retry_badcase: Option<bool>,
    pub retry_badcase_ratio_threshold: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Qwen3Config {
    pub model_dir: String,
    pub prompt_text: Option<String>,
    pub prompt_wav_path: Option<String>,
    pub language: Option<String>,
    pub speaker: Option<String>,
    pub voice_design_instruction: Option<String>,
    pub max_length: Option<usize>,
    pub temperature: Option<f64>,
    pub top_k: Option<usize>,
    pub top_p: Option<f64>,
    pub repetition_penalty: Option<f64>,
    pub seed: Option<u64>,
    pub chunk_frames: Option<usize>,
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

#[derive(Debug, Deserialize, Clone)]
pub struct LoopPreferencesConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub schedule_times: Option<Vec<String>>,
    pub max_posts_per_cycle: Option<usize>,
    pub personalization_user_id: Option<String>,
    pub profile_context_max_chars: Option<usize>,
}

fn default_true() -> bool {
    true
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
