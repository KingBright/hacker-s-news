use crate::config::VoicePrompt;
use crate::engine::TtsEngine;
use aha::models::voxcpm::generate::VoxCPMGenerate;
use anyhow::Result;
use candle_core::{DType, Device};
use std::sync::{Arc, Mutex};

pub struct VoxCpmAdapter {
    model: Arc<Mutex<VoxCPMGenerate>>,
    use_cache: bool,
}

impl VoxCpmAdapter {
    pub fn new(model_path: &str, device: &Device) -> Result<Self> {
        let model = VoxCPMGenerate::init(model_path, Some(device), Some(DType::F16))?;
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            use_cache: false,
        })
    }
}

#[async_trait::async_trait]
impl TtsEngine for VoxCpmAdapter {
    fn cache_voice_prompt(&mut self, prompt: &VoicePrompt) -> Result<()> {
        if let (Some(pt), Some(pw)) = (&prompt.text, &prompt.wav_path) {
            let mut model = self.model.lock().unwrap();
            model.build_prompt_cache(pt.to_string(), pw.to_string())?;
            self.use_cache = true;
        }
        Ok(())
    }

    fn sample_rate(&self) -> usize {
        let model = self.model.lock().unwrap();
        model.sample_rate()
    }

    fn synthesize_chunk(&mut self, chunk: &str) -> Result<Vec<f32>> {
        let mut model = self.model.lock().unwrap();
        let audio_tensor = if self.use_cache {
            model.generate_use_prompt_cache(chunk.to_string(), 2, 1024, 10, 2.0, false, 6.0)?
        } else {
            model.inference(
                chunk.to_string(),
                None, // We didn't save prompt strings natively if cache wasn't built
                None,
                2,
                1024,
                10,
                2.0,
                6.0,
            )?
        };
        Ok(audio_tensor.flatten_all()?.to_vec1::<f32>()?)
    }
}
