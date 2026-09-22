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
    pub content_generation: Option<ContentGenerationConfig>,
    pub curated_feed: Option<CuratedFeedConfig>,
    pub loop_preferences: Option<LoopPreferencesConfig>,
    pub voice_worker: Option<VoiceWorkerConfig>,
    pub hosts: Option<Vec<Host>>,
    pub interval_min: Option<u64>,
    pub schedule_times: Option<Vec<String>>, // Format: "HH:MM"
    pub timezone_offset: Option<i32>,        // Offset from UTC in hours (e.g., 8 for CST)
}

impl Config {
    pub fn external_agent_enabled(&self) -> bool {
        self.content_generation
            .as_ref()
            .is_some_and(|c| c.mode == GenerationMode::ExternalAgent)
    }

    pub fn content_generation_enabled(&self) -> bool {
        self.content_generation
            .as_ref()
            .map(|value| value.enabled && value.mode == GenerationMode::SelfDriven)
            .unwrap_or(true)
    }
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
    /// Optional trusted route override; URL hostname and TLS verification stay unchanged.
    pub connect_ip: Option<std::net::IpAddr>,
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
    pub worker_max_processes: Option<usize>,

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

#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenerationMode {
    #[default]
    SelfDriven,
    ExternalAgent,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ContentGenerationConfig {
    #[serde(default)]
    pub mode: GenerationMode,
    #[serde(default = "default_true")]
    pub enabled: bool,
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

#[derive(Debug, Deserialize, Clone)]
pub struct VoiceWorkerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub worker_id: Option<String>,
    pub poll_interval_secs: Option<u64>,
    pub concurrency: Option<usize>,
    pub max_jobs_per_tick: Option<usize>,
    pub lease_seconds: Option<i64>,
    pub repair_missing_audio: Option<bool>,
    pub repair_interval_secs: Option<u64>,
    pub repair_limit: Option<i64>,
    pub voice_kinds: Option<Vec<String>>,
}

fn default_true() -> bool {
    true
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_config(extra: &str) -> Config {
        let content = format!(
            r#"
[nexus]
api_url = "http://127.0.0.1:8899"
auth_key = "test-key"

[llm]
model = "test-model"
api_url = "http://127.0.0.1:1234/v1"

[tts]
engine = "voxcpm"

{extra}
"#
        );
        toml::from_str(&content).expect("valid config")
    }

    #[test]
    fn content_generation_defaults_to_enabled() {
        let config = parse_config("");

        assert!(config.content_generation_enabled());
    }

    #[test]
    fn content_generation_can_be_disabled() {
        let config = parse_config(
            r#"
[content_generation]
enabled = false
"#,
        );

        assert!(!config.content_generation_enabled());
    }

    #[test]
    fn tts_worker_max_processes_is_optional() {
        let config = parse_config("");

        assert_eq!(config.tts.worker_max_processes, None);
    }

    #[test]
    fn voice_worker_concurrency_is_optional() {
        let config = parse_config(
            r#"
[voice_worker]
enabled = true
"#,
        );

        assert_eq!(
            config
                .voice_worker
                .as_ref()
                .and_then(|worker| worker.concurrency),
            None
        );
    }
    #[test]
    fn external_mode_never_calls_local_generation_even_with_enabled_true() {
        let config: ContentGenerationConfig =
            toml::from_str("mode = \"external_agent\"\nenabled = true").unwrap();
        assert_eq!(config.mode, GenerationMode::ExternalAgent);
        let config: ContentGenerationConfig = toml::from_str("enabled = true").unwrap();
        assert_eq!(config.mode, GenerationMode::SelfDriven);
        assert!(toml::from_str::<ContentGenerationConfig>("mode = \"typo\"").is_err());
    }
}
