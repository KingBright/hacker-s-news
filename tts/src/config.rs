use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsConfig {
    pub engine: Option<String>,
    pub voxcpm: Option<VoxCpmConfig>,
    pub qwen3: Option<Qwen3Config>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoxCpmConfig {
    pub model_path: String,
    pub prompt_text: Option<String>,
    pub prompt_wav_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Qwen3Config {
    pub model_dir: String,
    pub prompt_text: Option<String>,
    pub prompt_wav_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VoicePrompt {
    pub text: Option<String>,
    pub wav_path: Option<String>,
}
