use crate::config::{VoicePrompt, VoxCpmConfig};
use crate::engine::TtsEngine;
use aha::models::voxcpm::generate::VoxCPMGenerate;
use anyhow::Result;
use candle_core::{DType, Device};
use std::sync::{Arc, Mutex};

pub struct VoxCpmAdapter {
    model: Arc<Mutex<VoxCPMGenerate>>,
    use_cache: bool,
    prompt_cache_key: Option<VoicePrompt>,
    control_instruction: Option<String>,
    min_len: usize,
    max_len: usize,
    inference_timesteps: usize,
    cfg_value: f64,
    retry_badcase: bool,
    retry_badcase_ratio_threshold: f64,
}

impl VoxCpmAdapter {
    pub fn new(config: &VoxCpmConfig, device: &Device) -> Result<Self> {
        let model = VoxCPMGenerate::init(&config.model_path, Some(device), Some(DType::F16))?;
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            use_cache: false,
            prompt_cache_key: None,
            control_instruction: config.control_instruction.clone(),
            min_len: config.min_len.unwrap_or(2),
            max_len: config.max_len.unwrap_or(1024),
            inference_timesteps: config.inference_timesteps.unwrap_or(10),
            cfg_value: config.cfg_value.unwrap_or(2.0),
            retry_badcase: config.retry_badcase.unwrap_or(false),
            retry_badcase_ratio_threshold: config.retry_badcase_ratio_threshold.unwrap_or(6.0),
        })
    }

    fn text_with_control(&self, chunk: &str) -> String {
        let Some(instruction) = self
            .control_instruction
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return chunk.to_string();
        };

        let trimmed = chunk.trim_start();
        if trimmed.starts_with('(') || trimmed.starts_with('（') {
            chunk.to_string()
        } else {
            format!("({}){}", instruction, chunk)
        }
    }
}

#[async_trait::async_trait]
impl TtsEngine for VoxCpmAdapter {
    fn cache_voice_prompt(&mut self, prompt: &VoicePrompt) -> Result<()> {
        if self.use_cache && self.prompt_cache_key.as_ref() == Some(prompt) {
            return Ok(());
        }

        if let (Some(pt), Some(pw)) = (&prompt.text, &prompt.wav_path) {
            let mut model = self.model.lock().unwrap();
            model.build_prompt_cache(pt.to_string(), pw.to_string())?;
            self.use_cache = true;
            self.prompt_cache_key = Some(prompt.clone());
        }
        Ok(())
    }

    fn sample_rate(&self) -> usize {
        let model = self.model.lock().unwrap();
        model.sample_rate()
    }

    fn synthesize_chunk(&mut self, chunk: &str) -> Result<Vec<f32>> {
        let target_text = self.text_with_control(chunk);
        let mut model = self.model.lock().unwrap();
        let audio_tensor = if self.use_cache {
            model.generate_use_prompt_cache(
                target_text,
                self.min_len,
                self.max_len,
                self.inference_timesteps,
                self.cfg_value,
                self.retry_badcase,
                self.retry_badcase_ratio_threshold,
            )?
        } else {
            model.inference(
                target_text,
                None, // We didn't save prompt strings natively if cache wasn't built
                None,
                self.min_len,
                self.max_len,
                self.inference_timesteps,
                self.cfg_value,
                self.retry_badcase_ratio_threshold,
            )?
        };
        Ok(audio_tensor.flatten_all()?.to_vec1::<f32>()?)
    }
}
