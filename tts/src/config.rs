use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsConfig {
    pub engine: Option<String>,
    pub device: Option<String>,
    pub voxcpm: Option<VoxCpmConfig>,
    pub qwen3: Option<Qwen3Config>,
    pub magictts: Option<MagicTtsConfig>,
    pub moss: Option<MossTtsConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoxCpmConfig {
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePrompt {
    pub text: Option<String>,
    pub wav_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
