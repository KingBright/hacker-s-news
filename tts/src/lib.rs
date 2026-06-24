pub mod adapters;
pub mod audio;
pub mod config;
pub mod engine;

pub use audio::{
    append_with_crossfade, chunk_text, convert_to_mp3, f32_samples_to_s16le_bytes,
    normalize_for_tts, Mp3StreamProcess, NormalizeOptions, PcmStreamEncoder,
};
pub use config::{MagicTtsConfig, MossTtsConfig, Qwen3Config, TtsConfig, VoicePrompt, VoxCpmConfig};
pub use engine::{EngineFactory, TtsEngine};
