use anyhow::Result;
use crate::core::config::TtsConfig as CortexTtsConfig;
use std::sync::Arc;
use tokio::sync::Mutex;
use tts::{TtsEngine, EngineFactory};
use tts::{TtsConfig as LibTtsConfig, VoxCpmConfig, Qwen3Config};

// Maximum total characters to prevent excessive memory usage
// ~8000 chars ≈ 15-25 minutes audio ≈ 80-150MB memory
const MAX_TOTAL_CHARS: usize = 8000;

pub struct TtsClient {
    engine: Arc<Mutex<Box<dyn TtsEngine>>>,
}

impl TtsClient {
    pub fn new(config: CortexTtsConfig) -> Self {
        // Map Cortex internal config to external TTS Library config schema
        let lib_config = LibTtsConfig {
            engine: config.engine.clone(),
            voxcpm: config.voxcpm.as_ref().map(|v| VoxCpmConfig {
                model_path: v.model_path.clone(),
                prompt_text: v.prompt_text.clone(),
                prompt_wav_path: v.prompt_wav_path.clone(),
            }),
            qwen3: config.qwen3.as_ref().map(|q| Qwen3Config {
                model_dir: q.model_dir.clone(),
                prompt_text: q.prompt_text.clone(),
                prompt_wav_path: q.prompt_wav_path.clone(),
            }),
        };

        let engine = EngineFactory::create(&lib_config).unwrap_or_else(|e| {
            log::error!("Failed to create TTS Engine: {}", e);
            panic!("Critical TTS loading failure: {}", e);
        });

        Self {
            engine: Arc::new(Mutex::new(engine)),
        }
    }

    pub async fn speak(&self, text: &str) -> Result<Vec<u8>> {
        self.speak_and_convert(text, None, None).await
    }

    pub async fn speak_with_voice(&self, text: &str, voice_path: &str, prompt_override: Option<&str>) -> Result<Vec<u8>> {
        self.speak_and_convert(text, Some(voice_path.to_string()), prompt_override.map(|s| s.to_string())).await
    }

    async fn speak_and_convert(&self, raw_text: &str, voice_override: Option<String>, prompt_override: Option<String>) -> Result<Vec<u8>> {
        let text = if raw_text.chars().count() > MAX_TOTAL_CHARS {
            log::warn!(
                "[TTS] Text too long ({} chars > {} limit), truncating",
                raw_text.chars().count(),
                MAX_TOTAL_CHARS
            );
            let truncated: String = raw_text.chars().take(MAX_TOTAL_CHARS - 10).collect();
            format!("{}……（内容过长，已截断）", truncated)
        } else {
            raw_text.to_string()
        };

        log::info!("Synthesizing long text through abstracted TTS library...");
        
        let mut engine = self.engine.lock().await;

        // If specific custom voices are requested outside of the default config, build dynamic cache prompt here
        if voice_override.is_some() || prompt_override.is_some() {
            let override_prompt = tts::VoicePrompt {
                text: prompt_override,
                wav_path: voice_override,
            };
            if let Err(e) = engine.cache_voice_prompt(&override_prompt) {
                log::warn!("Failed to apply temporary voice override cache: {}", e);
            }
        }

        let pcm_samples = engine.synthesize_long_text(&text).await?;

        // Encode PCM float samples into standard 16-bit WAV
        let wav_bytes = self.create_wav_bytes(&pcm_samples, engine.sample_rate() as u32)?;
        Ok(wav_bytes)
    }

    /// Helper: Convert WAV bytes to MP3 using TTS library
    pub async fn convert_to_mp3(&self, wav_bytes: &[u8]) -> Result<Vec<u8>> {
        tts::convert_to_mp3(wav_bytes).await
    }

    fn create_wav_bytes(&self, data: &[f32], sample_rate: u32) -> Result<Vec<u8>> {
        use std::io::Cursor;
        let mut cursor = Cursor::new(Vec::new());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
        for &sample in data {
            // Convert f32 to i16 properly avoiding overflow
            let sample_i16 = (sample.max(-1.0).min(1.0) * 32767.0) as i16;
            writer.write_sample(sample_i16)?;
        }
        writer.finalize()?;
        Ok(cursor.into_inner())
    }
}
