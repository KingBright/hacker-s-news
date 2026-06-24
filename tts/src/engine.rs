use crate::adapters::{
    MagicTtsAdapter, MossTtsAdapter, Qwen3Adapter, VoxCpmAdapter, VoxCpmMetalAdapter,
};
use crate::config::{TtsConfig, VoicePrompt};
use anyhow::{anyhow, Result};
use candle_core::Device;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DevicePreference {
    Cpu,
    Metal,
}

#[async_trait::async_trait]
pub trait TtsEngine: Send + Sync {
    /// Provide a Voice Prompt ahead of time to build cache (e.g. for long paragraphs)
    fn cache_voice_prompt(&mut self, prompt: &VoicePrompt) -> Result<()>;

    /// Obtain the configured sample rate
    fn sample_rate(&self) -> usize;

    /// Synthesize a specific chunk of text and return raw float audio samples
    fn synthesize_chunk(&mut self, text: &str) -> Result<Vec<f32>>;

    /// Optional: Stream speech synthesis by chunking internally and calling a callback on each chunk.
    /// Default implementation just invokes `synthesize_chunk` once.
    fn synthesize_streaming(
        &mut self,
        text: &str,
        callback: &mut dyn FnMut(Vec<f32>) -> Result<()>,
    ) -> Result<()> {
        let samples = self.synthesize_chunk(text)?;
        callback(samples)?;
        Ok(())
    }

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
        let engine = config.engine.as_deref().unwrap_or("voxcpm").trim();
        let device_preference = resolve_device_preference(engine, config.device.as_deref());
        let device = create_device(device_preference)?;
        log::info!("TTS using device: {:?} ({:?})", device, device_preference);

        if engine == "voxcpm" {
            if let Some(vox_config) = &config.voxcpm {
                log::info!("Initializing VoxCPM model from: {}", vox_config.model_path);
                let mut adapter = VoxCpmAdapter::new(vox_config, &device)?;
                // Pre-cache if parameters exist in config
                adapter.cache_voice_prompt(&VoicePrompt {
                    text: vox_config.prompt_text.clone(),
                    wav_path: vox_config.prompt_wav_path.clone(),
                })?;
                return Ok(Box::new(adapter));
            } else {
                return Err(anyhow!("VoxCPM config missing"));
            }
        } else if engine == "voxcpm_metal" {
            if let Some(vox_config) = &config.voxcpm {
                log::info!(
                    "Initializing VoxCPM (Metal CPU-GPU hybrid) model from: {}",
                    vox_config.model_path
                );
                let mut adapter = VoxCpmMetalAdapter::new(vox_config, &device)?;
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
                let mut adapter = Qwen3Adapter::new(qwen_config, &device)?;
                adapter.cache_voice_prompt(&VoicePrompt {
                    text: qwen_config.prompt_text.clone(),
                    wav_path: qwen_config.prompt_wav_path.clone(),
                })?;
                return Ok(Box::new(adapter));
            } else {
                return Err(anyhow!("Qwen3 config missing"));
            }
        } else if engine == "magictts" {
            if let Some(magic_config) = &config.magictts {
                log::info!(
                    "Initializing Magic-TTS model from: {}",
                    magic_config.model_dir
                );
                let mut adapter = MagicTtsAdapter::new(magic_config)?;
                adapter.cache_voice_prompt(&VoicePrompt {
                    text: magic_config.prompt_text.clone(),
                    wav_path: magic_config.prompt_wav_path.clone(),
                })?;
                return Ok(Box::new(adapter));
            } else {
                return Err(anyhow!("Magic-TTS config missing"));
            }
        } else if engine == "moss" {
            if let Some(moss_config) = &config.moss {
                log::info!(
                    "Initializing MOSS-TTS-Nano model from: {}",
                    moss_config.model_dir
                );
                let mut adapter = MossTtsAdapter::new(moss_config, config.device.as_deref())?;
                adapter.cache_voice_prompt(&VoicePrompt {
                    text: moss_config.prompt_text.clone(),
                    wav_path: moss_config.prompt_wav_path.clone(),
                })?;
                return Ok(Box::new(adapter));
            } else {
                return Err(anyhow!("MOSS-TTS-Nano config missing"));
            }
        }

        Err(anyhow!("Unknown TTS Engine: {}", engine))
    }
}

fn create_device(preference: DevicePreference) -> Result<Device> {
    match preference {
        DevicePreference::Cpu => Ok(Device::Cpu),
        DevicePreference::Metal => Device::new_metal(0).map_err(|e| {
            anyhow!(
                "Metal requested for TTS but unavailable; refusing CPU fallback: {}",
                e
            )
        }),
    }
}

fn resolve_device_preference(engine: &str, configured: Option<&str>) -> DevicePreference {
    match configured.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.eq_ignore_ascii_case("cpu") => DevicePreference::Cpu,
        Some(value)
            if value.eq_ignore_ascii_case("metal")
                || value.eq_ignore_ascii_case("gpu")
                || value.eq_ignore_ascii_case("mps") =>
        {
            DevicePreference::Metal
        }
        Some(value) => {
            log::warn!(
                "Unknown TTS device preference '{}'; using engine default",
                value
            );
            default_device_preference(engine)
        }
        None => default_device_preference(engine),
    }
}

fn default_device_preference(engine: &str) -> DevicePreference {
    if engine.trim().eq_ignore_ascii_case("voxcpm_metal") {
        DevicePreference::Metal
    } else {
        DevicePreference::Cpu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voxcpm_defaults_to_cpu_to_avoid_service_gpu_footprint() {
        assert_eq!(
            resolve_device_preference("voxcpm", None),
            DevicePreference::Cpu
        );
    }

    #[test]
    fn voxcpm_metal_defaults_to_metal() {
        assert_eq!(
            resolve_device_preference("voxcpm_metal", None),
            DevicePreference::Metal
        );
    }

    #[test]
    fn explicit_device_overrides_engine_default() {
        assert_eq!(
            resolve_device_preference("voxcpm_metal", Some("cpu")),
            DevicePreference::Cpu
        );
        assert_eq!(
            resolve_device_preference("voxcpm", Some("metal")),
            DevicePreference::Metal
        );
    }
}
