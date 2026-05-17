use crate::adapters::{Qwen3Adapter, VoxCpmAdapter};
use crate::config::{TtsConfig, VoicePrompt};
use anyhow::{anyhow, Result};
use candle_core::Device;

#[async_trait::async_trait]
pub trait TtsEngine: Send + Sync {
    /// Provide a Voice Prompt ahead of time to build cache (e.g. for long paragraphs)
    fn cache_voice_prompt(&mut self, prompt: &VoicePrompt) -> Result<()>;

    /// Obtain the configured sample rate
    fn sample_rate(&self) -> usize;

    /// Synthesize a specific chunk of text and return raw float audio samples
    fn synthesize_chunk(&mut self, text: &str) -> Result<Vec<f32>>;

    /// Optional: Fully synthesize a long text by breaking it up internally and managing cross-fades
    /// Default implementation covers typical paragraph parsing, or the Adapter can override it.
    async fn synthesize_long_text(&mut self, raw_text: &str) -> Result<Vec<f32>> {
        // Shared naive chunking logic across models
        // ... (We will implement default behavior in audio.rs or directly here)
        let chunks = crate::audio::chunk_text(raw_text);
        let mut all_samples: Vec<f32> = Vec::new();

        for (idx, chunk) in chunks.iter().enumerate() {
            if chunk.trim().is_empty() {
                continue;
            }
            log::info!(
                "Generating audio chunk {}/{} ({} chars)...",
                idx + 1,
                chunks.len(),
                chunk.chars().count()
            );

            let new_samples = self.synthesize_chunk(chunk)?;
            crate::audio::append_with_crossfade(
                &mut all_samples,
                &new_samples,
                self.sample_rate(),
                0.05,
            );
        }

        Ok(all_samples)
    }
}

pub struct EngineFactory;

impl EngineFactory {
    pub fn create(config: &TtsConfig) -> Result<Box<dyn TtsEngine>> {
        let device = Device::new_metal(0).unwrap_or(Device::Cpu);
        log::info!("TTS using device: {:?}", device);

        let engine = config.engine.as_deref().unwrap_or("voxcpm").trim();

        if engine == "voxcpm" {
            if let Some(vox_config) = &config.voxcpm {
                log::info!("Initializing VoxCPM model from: {}", vox_config.model_path);
                let mut adapter = VoxCpmAdapter::new(&vox_config.model_path, &device)?;
                // Pre-cache if parameters exist in config
                adapter.cache_voice_prompt(&VoicePrompt {
                    text: vox_config.prompt_text.clone(),
                    wav_path: vox_config.prompt_wav_path.clone(),
                })?;
                return Ok(Box::new(adapter));
            } else {
                return Err(anyhow!("VoxCPM config missing"));
            }
        } else if engine == "qwen3" {
            if let Some(qwen_config) = &config.qwen3 {
                log::info!("Initializing Qwen3 model from: {}", qwen_config.model_dir);
                let mut adapter = Qwen3Adapter::new(&qwen_config.model_dir, &device)?;
                adapter.cache_voice_prompt(&VoicePrompt {
                    text: qwen_config.prompt_text.clone(),
                    wav_path: qwen_config.prompt_wav_path.clone(),
                })?;
                return Ok(Box::new(adapter));
            } else {
                return Err(anyhow!("Qwen3 config missing"));
            }
        }

        Err(anyhow!("Unknown TTS Engine: {}", engine))
    }
}
