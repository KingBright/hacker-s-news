use anyhow::Result;
use crate::core::config::TtsConfig;
use std::sync::{Arc, Mutex};
use candle_core::{Device, DType};
use tts::voxcpm::generate::VoxCPMGenerate;
use tts::utils::audio_utils::get_audio_wav_u8;

pub struct TtsClient {
    config: TtsConfig,
    voxcpm_model: Option<Arc<Mutex<VoxCPMGenerate>>>,
}

impl TtsClient {
    pub fn new(config: TtsConfig) -> Self {
        let device = Device::new_metal(0).unwrap_or(Device::Cpu);
        log::info!("TTS using device: {:?}", device);

        let mut voxcpm_model = None;

        // Check if engine is voxcpm (default or explicit)
        let engine = config.engine.as_deref().unwrap_or("voxcpm");

        if engine == "voxcpm" {
            if let Some(vox_config) = &config.voxcpm {
                log::info!("Initializing VoxCPM model from: {}", vox_config.model_path);
                match VoxCPMGenerate::init(&vox_config.model_path, Some(&device), Some(DType::F32)) {
                    Ok(model) => voxcpm_model = Some(Arc::new(Mutex::new(model))),
                    Err(e) => log::error!("Failed to load VoxCPM model: {}", e),
                }
            } else {
                log::warn!("VoxCPM engine selected but no config provided");
            }
        }

        Self {
            config,
            voxcpm_model,
        }
    }

    pub async fn speak(&self, text: &str) -> Result<Vec<u8>> {
        let engine = self.config.engine.as_deref().unwrap_or("voxcpm");
        
        if engine == "voxcpm" {
            return self.speak_voxcpm(text, None).await;
        }

        // Fallback or other engines
        Err(anyhow::anyhow!("Unsupported TTS engine: {}", engine))
    }

    /// Speak with a specific voice file (for multi-host support)
    pub async fn speak_with_voice(&self, text: &str, voice_path: &str) -> Result<Vec<u8>> {
        let engine = self.config.engine.as_deref().unwrap_or("voxcpm");
        
        if engine == "voxcpm" {
            return self.speak_voxcpm(text, Some(voice_path.to_string())).await;
        }

        Err(anyhow::anyhow!("Unsupported TTS engine: {}", engine))
    }

    async fn speak_voxcpm(&self, text: &str, voice_override: Option<String>) -> Result<Vec<u8>> {
        let vox_config = self.config.voxcpm.as_ref()
            .ok_or_else(|| anyhow::anyhow!("VoxCPM config missing"))?;
            
        let model_mutex = self.voxcpm_model.as_ref()
            .ok_or_else(|| anyhow::anyhow!("VoxCPM model not loaded"))?;

        // Simple chunking strategy: split by common sentence terminators
        // to avoid passing too long text to the model which causes degradation.
        // Improved chunking: Prioritize sentence boundaries to avoid audio glitches
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        
        let terminators = ['。', '！', '？', '\n'];
        let secondary_terminators = ['，', '；'];
        
        for char in text.chars() {
            current_chunk.push(char);
            
            let len = current_chunk.chars().count();
            
            // 1. Mandatory split on Newline (Paragraph)
            if char == '\n' {
                if !current_chunk.trim().is_empty() {
                    chunks.push(current_chunk.clone());
                    current_chunk.clear();
                }
                continue;
            }

            // 2. Primary split: Sentence Endings (if length is sufficient)
            // We aim for chunks around 50-150 chars for optimal flow
            if terminators.contains(&char) && len > 50 {
                chunks.push(current_chunk.clone());
                current_chunk.clear();
                continue;
            }

            // 3. Safety valve: If chunk gets too long (>300), split at next comma/semicolon
            if len > 300 && secondary_terminators.contains(&char) {
                 chunks.push(current_chunk.clone());
                 current_chunk.clear();
                 continue;
            }
            
            // 4. Emergency: > 500 chars, split anyway (very rare)
            if len > 500 {
                chunks.push(current_chunk.clone());
                current_chunk.clear();
            }
        }
        
        if !current_chunk.trim().is_empty() {
            chunks.push(current_chunk);
        }

        // Concatenate all chunks with Cross-Fade
        let mut all_samples: Vec<f32> = Vec::new();
        // Variables moved inside loop for dynamic calculation based on model sample rate

        for (idx, chunk) in chunks.iter().enumerate() {
            if chunk.trim().is_empty() { continue; }
            
            log::info!("Generating audio for chunk {}/{}: {}...", idx+1, chunks.len(), &chunk.chars().take(20).collect::<String>());
            
            let mut model = model_mutex.lock().map_err(|e| anyhow::anyhow!("Model lock failed: {}", e))?;
            
            // Get sample rate dynamically from the model
            let sample_rate = model.sample_rate();

            // Parameters
            let prompt_text = vox_config.prompt_text.clone();
            // Use voice_override if provided, otherwise use default from config
            let prompt_wav_path = voice_override.clone().or(vox_config.prompt_wav_path.clone());
            
            let start = std::time::Instant::now();
            let audio_tensor = model.inference(
                chunk.to_string(),
                prompt_text,
                prompt_wav_path,
                2,      // min_len
                4096,   // max_len
                10,     // inference_timesteps
                2.0,    // cfg_value
                6.0,    // retry_badcase_ratio_threshold
            )?;
            log::info!("Chunk {} generated in {:.2?}", idx+1, start.elapsed());
            
            let new_samples = audio_tensor.flatten_all()?.to_vec1::<f32>()?;
            
             if all_samples.is_empty() {
                all_samples.extend(new_samples);
            } else {
                // Cross-fade logic:
                // Blend end of all_samples with start of new_samples
                let crossfade_duration = 0.05; // 50ms overlap
                let crossfade_samples = (sample_rate as f64 * crossfade_duration) as usize;
                
                let overlap_len = std::cmp::min(all_samples.len(), crossfade_samples);
                let overlap_len = std::cmp::min(overlap_len, new_samples.len());

                let start_idx = all_samples.len() - overlap_len;
                for i in 0..overlap_len {
                    let fade_out = 1.0 - (i as f32 / overlap_len as f32);
                    let fade_in = i as f32 / overlap_len as f32;
                    let old_val = all_samples[start_idx + i];
                    let new_val = new_samples[i];
                    all_samples[start_idx + i] = old_val * fade_out + new_val * fade_in;
                }
                
                if new_samples.len() > overlap_len {
                    all_samples.extend(&new_samples[overlap_len..]);
                }
            }
        }

        // Convert all samples back to tensor for wav conversion
        // Reshape to (1, N) as get_audio_wav_u8 expects [Channels, Samples]
        let combined_tensor = candle_core::Tensor::from_vec(all_samples.clone(), (1, all_samples.len()), &Device::Cpu)?;
        
        // Use the sample rate from the last model interaction (should be consistent)
        // We need to unlock or just get it again. Since we are in a loop above, we can just grab it briefly.
        let sample_rate = {
             let model = model_mutex.lock().map_err(|e| anyhow::anyhow!("Model lock failed: {}", e))?;
             model.sample_rate()
        };
        
        let wav_bytes = get_audio_wav_u8(&combined_tensor, sample_rate as u32)?;
        
        Ok(wav_bytes)
    }
}
