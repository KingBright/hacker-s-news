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
            return self.speak_voxcpm(text).await;
        }

        // Fallback or other engines
        Err(anyhow::anyhow!("Unsupported TTS engine: {}", engine))
    }

    async fn speak_voxcpm(&self, text: &str) -> Result<Vec<u8>> {
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

        // Concatenate all chunks
        // Best way: collect all tensors then concat? Or all samples.
        // Since we are inside async function and iterating, collecting samples is easier.
        let mut all_samples: Vec<f32> = Vec::new();

        for (idx, chunk) in chunks.iter().enumerate() {
            if chunk.trim().is_empty() { continue; }
            
            log::info!("Generating audio for chunk {}/{}: {}...", idx+1, chunks.len(), &chunk.chars().take(20).collect::<String>());
            
            // Lock the model for each inference
            let mut model = model_mutex.lock().map_err(|e| anyhow::anyhow!("Model lock failed: {}", e))?;

            // Parameters
            let prompt_text = vox_config.prompt_text.clone();
            let prompt_wav_path = vox_config.prompt_wav_path.clone();
            
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
            
            // Convert tensor to samples immediately
            let samples = audio_tensor.flatten_all()?.to_vec1::<f32>()?;
            all_samples.extend(samples);
            
            // Add silence (0.5s)
            let silence_samples = (16000.0 * 0.5) as usize;
            all_samples.extend(vec![0.0f32; silence_samples]);
        }

        // Convert all samples back to tensor for wav conversion
        // Reshape to (1, N) as get_audio_wav_u8 expects [Channels, Samples]
        let combined_tensor = candle_core::Tensor::from_vec(all_samples.clone(), (1, all_samples.len()), &Device::Cpu)?;
        let sample_rate = 16000;
        let wav_bytes = get_audio_wav_u8(&combined_tensor, sample_rate)?;
        
        Ok(wav_bytes)
    }
}
