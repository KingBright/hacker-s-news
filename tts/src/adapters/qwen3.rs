use anyhow::Result;
use std::sync::{Arc, Mutex};
use candle_core::Device;
use qwen3_tts::{Qwen3TTS, AudioBuffer, Language, VoiceClonePrompt as QwenPrompt};
use crate::engine::TtsEngine;
use crate::config::VoicePrompt;

pub struct Qwen3Adapter {
    model: Arc<Mutex<Qwen3TTS>>,
    prompt: Option<QwenPrompt>,
    last_sample_rate: u32,
}

impl Qwen3Adapter {
    pub fn new(model_dir: &str, device: &Device) -> Result<Self> {
        let model = if model_dir.contains('/') && !std::path::Path::new(model_dir).exists() {
            let paths = qwen3_tts::ModelPaths::download(Some(model_dir))?;
            Qwen3TTS::from_paths(&paths, device.clone())?
        } else {
            Qwen3TTS::from_pretrained(model_dir, device.clone())?
        };
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            prompt: None,
            last_sample_rate: 24000,
        })
    }
}

#[async_trait::async_trait]
impl TtsEngine for Qwen3Adapter {
    fn cache_voice_prompt(&mut self, prompt: &VoicePrompt) -> Result<()> {
        if let Some(wav_path) = prompt.wav_path.as_ref().map(|p| p.replace("file://", "")) {
            let ref_audio = AudioBuffer::load(&(wav_path))?;
            let model = self.model.lock().unwrap();
            let qwen_prompt = model.create_voice_clone_prompt(&ref_audio, prompt.text.as_deref())?;
            self.prompt = Some(qwen_prompt);
        }
        Ok(())
    }

    fn sample_rate(&self) -> usize {
        self.last_sample_rate as usize
    }

    fn synthesize_chunk(&mut self, chunk: &str) -> Result<Vec<f32>> {
        let model = self.model.lock().unwrap();
        let audio_buffer = if let Some(ref prompt) = self.prompt {
            model.synthesize_voice_clone(
                chunk,
                prompt,
                Language::Chinese,
                Some(qwen3_tts::SynthesisOptions {
                    max_length: 2048,
                    ..Default::default()
                })
            )?
        } else {
            model.synthesize(chunk, None)?
        };
        self.last_sample_rate = audio_buffer.sample_rate;
        Ok(audio_buffer.samples)
    }
}
