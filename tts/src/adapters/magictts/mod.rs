pub mod cfm;
pub mod model;
pub use cfm::UnifiedCFM;
pub use model::DiT;

use ort::session::Session;

use crate::config::{MagicTtsConfig, VoicePrompt};
use crate::engine::TtsEngine;
use anyhow::{anyhow, Result};
use jieba_rs::Jieba;
use pinyin::ToPinyin;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::{Arc, Mutex};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use qwen3_tts::audio::{resample, AudioBuffer, MelConfig, MelSpectrogram};

// 获取当前平台的 GPU/CPU 计算设备
fn get_device() -> Result<Device> {
    #[cfg(feature = "metal")]
    {
        log::info!("Magic-TTS: Using Metal GPU Device");
        Ok(Device::new_metal(0)?)
    }
    #[cfg(not(feature = "metal"))]
    {
        log::info!("Magic-TTS: Using CPU Device");
        Ok(Device::Cpu)
    }
}

pub struct MagicTtsAdapter {
    cfm: Arc<Mutex<UnifiedCFM>>,
    vocab: HashMap<String, i32>,
    vocos: Session,
    steps: usize,
    cfg_strength: f64,
    default_content_ms: f64,
    default_punct_ms: f64,
    jieba: Jieba,
    device: Device,
    cached_mel: Option<Tensor>,
    cached_text: Option<String>,
}

impl MagicTtsAdapter {
    pub fn new(config: &MagicTtsConfig) -> Result<Self> {
        let device = get_device()?;
        let safetensors_path = Path::new(&config.model_dir).join("consolidated.safetensors");
        if !safetensors_path.exists() {
            return Err(anyhow!(
                "Magic-TTS: consolidated.safetensors not found in {}",
                config.model_dir
            ));
        }

        log::info!(
            "Magic-TTS: Loading model parameters from {:?} ...",
            safetensors_path
        );
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[safetensors_path], DType::F32, &device)?
        };

        // 加载 vocab.txt
        let vocab_path = config.vocab_path.as_deref().unwrap_or("vocab.txt");
        let vocab_file = File::open(Path::new(&config.model_dir).join(vocab_path))
            .or_else(|_| File::open(vocab_path))?;
        let reader = BufReader::new(vocab_file);
        let mut vocab = HashMap::new();
        for (idx, line) in reader.lines().enumerate() {
            let line = line?;
            vocab.insert(line, idx as i32);
        }

        // 构建 DiT 与 CFM 求解器
        let transformer_vb = vb.pp("transformer");
        let dit = DiT::new(
            1024,        // dim
            22,          // depth
            16,          // heads
            64,          // dim_head
            2,           // ff_mult
            100,         // mel_dim
            vocab.len(), // text_num_embeds
            512,         // text_dim
            4,           // conv_layers
            true,        // duration_condition
            100.0,       // duration_log_scale
            transformer_vb,
        )?;

        let cfm = UnifiedCFM::new(dit);

        let vocos_path = Path::new(&config.model_dir).join("vocos.onnx");
        if !vocos_path.exists() {
            return Err(anyhow!(
                "Magic-TTS: vocos.onnx not found in {}",
                config.model_dir
            ));
        }
        log::info!("Magic-TTS: Loading Vocos Vocoder from {:?} ...", vocos_path);
        let vocos = Session::builder()?.commit_from_file(&vocos_path)?;

        Ok(Self {
            cfm: Arc::new(Mutex::new(cfm)),
            vocab,
            vocos,
            steps: config.steps.unwrap_or(32),
            cfg_strength: config.cfg_strength.unwrap_or(2.0),
            default_content_ms: config.default_content_ms.unwrap_or(170.0),
            default_punct_ms: config.default_punct_ms.unwrap_or(50.0),
            jieba: Jieba::new(),
            device,
            cached_mel: None,
            cached_text: None,
        })
    }

    fn tokenize(&self, text: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let words = self.jieba.cut(text, true);
        let punct_chars = "，。？！；：、,.!?;:\"'()（）";

        for word in words {
            if word
                .chars()
                .all(|c| c.is_ascii_alphabetic() || c.is_ascii_digit())
            {
                if !tokens.is_empty() && tokens.last().map(|s: &String| s.as_str()) != Some(" ") {
                    tokens.push(" ".to_string());
                }
                for ch in word.chars() {
                    tokens.push(ch.to_string());
                }
            } else if word.len() == 3 && punct_chars.contains(word) {
                tokens.push(word.to_string());
            } else {
                let py_res = word.to_pinyin();
                for py in py_res.flatten() {
                    tokens.push(" ".to_string());
                    let mut py_str = py.with_tone_num_end().to_string();
                    if py_str.is_empty() {
                        py_str = py.plain().to_string();
                    }
                    tokens.push(py_str);
                }
            }
        }
        tokens
    }

    fn ms_to_frames(&self, ms: f64) -> f32 {
        (ms * 24000.0 / 1000.0 / 256.0) as f32
    }
}

#[async_trait::async_trait]
impl TtsEngine for MagicTtsAdapter {
    fn cache_voice_prompt(&mut self, prompt: &VoicePrompt) -> Result<()> {
        if let (Some(wav_path), Some(text)) = (&prompt.wav_path, &prompt.text) {
            let clean_path = wav_path.strip_prefix("file://").unwrap_or(wav_path);
            let audio = AudioBuffer::load(clean_path)?;
            let audio_24k = if audio.sample_rate != 24000 {
                resample(&audio, 24000)?
            } else {
                audio
            };

            let mel_config = MelConfig {
                sample_rate: 24000,
                n_fft: 1024,
                hop_length: 256,
                win_length: Some(1024),
                n_mels: 100,
                fmin: 0.0,
                fmax: None,
            };
            let extractor = MelSpectrogram::new(mel_config);
            let mel_tensor =
                extractor.compute_for_speaker_encoder(&audio_24k.samples, &self.device)?;
            let mel_transposed = mel_tensor.transpose(0, 1)?; // [n_frames, 100]

            self.cached_mel = Some(mel_transposed);
            self.cached_text = Some(text.clone());
        } else {
            self.cached_mel = None;
            self.cached_text = None;
        }
        Ok(())
    }

    fn sample_rate(&self) -> usize {
        24000
    }

    fn synthesize_chunk(&mut self, chunk: &str) -> Result<Vec<f32>> {
        let re_duration = regex::Regex::new(r"([^\s\{\}\[\]]+)\{(\d+)\}").unwrap();
        let re_pause = regex::Regex::new(r"\[(\d+)\]").unwrap();

        let mut custom_durations = HashMap::new();
        let mut custom_pauses = HashMap::new();
        let mut cleaned_text = chunk.to_string();

        for cap in re_pause.captures_iter(chunk) {
            let ms: f64 = cap[1].parse().unwrap_or(0.0);
            let full_match = cap[0].to_string();
            let idx = cleaned_text.find(&full_match).unwrap_or(0);
            if idx > 0 {
                let char_key = cleaned_text[..idx]
                    .chars()
                    .last()
                    .unwrap_or(' ')
                    .to_string();
                custom_pauses.insert(char_key, ms);
            }
            cleaned_text = cleaned_text.replace(&full_match, "");
        }

        let temp_text = cleaned_text.clone();
        for cap in re_duration.captures_iter(&temp_text) {
            let word = cap[1].to_string();
            let ms: f64 = cap[2].parse().unwrap_or(0.0);
            let full_match = cap[0].to_string();
            custom_durations.insert(word, ms);
            cleaned_text = cleaned_text.replace(&full_match, &cap[1]);
        }

        let mut tokens_ids = Vec::new();
        let mut durations_features = Vec::new();
        let mut ref_mel_len = 0;

        let content_frames = self.ms_to_frames(self.default_content_ms);
        let punct_frames = self.ms_to_frames(self.default_punct_ms);
        let punct_chars = "，。？！；：、,.!?;:\"'()（）";

        if let (Some(ref_mel), Some(ref_text)) = (&self.cached_mel, &self.cached_text) {
            let ref_tokens = self.tokenize(ref_text);
            ref_mel_len = ref_mel.dim(0)?;

            let mut ref_default_frames = 0.0f32;
            for token in ref_tokens.iter() {
                let (content_f, pause_f) = if token == " " {
                    (0.0f32, 0.0f32)
                } else if punct_chars.contains(token) {
                    (punct_frames, 0.0f32)
                } else {
                    (content_frames, 0.0f32)
                };
                ref_default_frames += content_f + pause_f;
            }
            if ref_default_frames == 0.0 {
                ref_default_frames = 1.0;
            }
            let scale = ref_mel_len as f32 / ref_default_frames;

            for token in ref_tokens.iter() {
                let id = *self.vocab.get(token).unwrap_or(&0);
                tokens_ids.push(id);
                let (content_f, pause_f) = if token == " " {
                    (0.0f32, 0.0f32)
                } else if punct_chars.contains(token) {
                    (punct_frames * scale, 0.0f32)
                } else {
                    (content_frames * scale, 0.0f32)
                };
                durations_features.push(content_f);
                durations_features.push(pause_f);
            }
        }

        let raw_tokens = self.tokenize(&cleaned_text);
        let mut total_gen_frames = 0.0f32;

        for (idx, token) in raw_tokens.iter().enumerate() {
            let id = *self.vocab.get(token).unwrap_or(&0);
            tokens_ids.push(id);

            let (content_f, pause_f) = if token == " " {
                (0.0f32, 0.0f32)
            } else if punct_chars.contains(token) {
                let mut p_f = 0.0f32;
                if let Some(&ms) = custom_pauses.get(token) {
                    p_f = self.ms_to_frames(ms);
                }
                (punct_frames, p_f)
            } else {
                let mut c_f = content_frames;
                let mut p_f = 0.0f32;
                if let Some(&ms) = custom_durations.get(token) {
                    c_f = self.ms_to_frames(ms);
                }
                if idx + 1 < raw_tokens.len() {
                    let next_token = &raw_tokens[idx + 1];
                    if let Some(&ms) = custom_pauses.get(next_token) {
                        p_f = self.ms_to_frames(ms);
                    }
                }
                (c_f, p_f)
            };
            durations_features.push(content_f);
            durations_features.push(pause_f);
            total_gen_frames += content_f + pause_f;
        }

        let total_duration_frames = ref_mel_len + (total_gen_frames.round() as usize);
        let total_duration_frames = if total_duration_frames == 0 {
            128
        } else {
            total_duration_frames
        };

        let mut tokens_ids_shifted: Vec<u32> =
            tokens_ids.iter().map(|&id| (id + 1) as u32).collect();
        if tokens_ids_shifted.len() < total_duration_frames {
            tokens_ids_shifted.resize(total_duration_frames, 0u32);
        } else {
            tokens_ids_shifted.truncate(total_duration_frames);
        }

        let mut durations_features_aligned = durations_features.clone();
        if durations_features_aligned.len() < total_duration_frames * 2 {
            durations_features_aligned.resize(total_duration_frames * 2, 0.0f32);
        } else {
            durations_features_aligned.truncate(total_duration_frames * 2);
        }

        let cond_tensor = if let Some(ref_mel) = &self.cached_mel {
            ref_mel.unsqueeze(0)?
        } else {
            Tensor::zeros((1, 1, 100), DType::F32, &self.device)?
        };

        let text_tensor = Tensor::from_slice(
            &tokens_ids_shifted,
            (1, total_duration_frames),
            &self.device,
        )?;
        let durations_tensor = Tensor::from_slice(
            &durations_features_aligned,
            (1, total_duration_frames, 2),
            &self.device,
        )?;

        let cfm = self.cfm.lock().unwrap();
        let mel_pred = cfm.solve_euler(
            &cond_tensor,
            &text_tensor,
            &durations_tensor,
            total_duration_frames,
            self.steps,
            self.cfg_strength,
            &self.device,
        )?;

        let mel_transposed = mel_pred.transpose(1, 2)?.contiguous()?;
        let mel_flat = mel_transposed.flatten_all()?.to_vec1::<f32>()?;

        let input_value =
            ort::value::Value::from_array(([1, 100, total_duration_frames], mel_flat))?;
        let outputs = self.vocos.run(ort::inputs![input_value])?;
        let wav_tensor = outputs[0].try_extract_tensor::<f32>()?;
        let pcm_samples = wav_tensor.1.to_vec();

        Ok(pcm_samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MagicTtsConfig;
    use candle_nn::VarMap;
    use std::io::Write;

    #[test]
    fn test_magic_tts_adapter_and_synthesis() {
        let test_dir = std::env::temp_dir().join("magictts_test");
        std::fs::create_dir_all(&test_dir).unwrap();

        let vocab_path = test_dir.join("vocab.txt");
        let mut vocab_file = File::create(&vocab_path).unwrap();
        let dummy_vocab = vec![
            " ", "qian2", "fang1", "lu4", "kou3", "zuo3", "zhuan3", "，", "。", "！",
        ];
        for v in &dummy_vocab {
            writeln!(vocab_file, "{}", v).unwrap();
        }

        let device = Device::Cpu;
        let varmap = VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, &device);

        let text_num_embeds = dummy_vocab.len();
        let text_dim = 512;
        let dim = 1024;
        let mel_dim = 100;
        let freq_embed_dim = 256;

        let _ = vb
            .get(
                (text_num_embeds + 1, text_dim),
                "transformer.text_embed.text_embed.weight",
            )
            .unwrap();
        let _ = vb
            .get(
                (1, 1, text_dim * 2),
                "transformer.text_embed.text_blocks.0.grn.gamma",
            )
            .unwrap();
        let _ = vb
            .get(
                (1, 1, text_dim * 2),
                "transformer.text_embed.text_blocks.0.grn.beta",
            )
            .unwrap();
        let _ = vb
            .get(text_dim, "transformer.text_embed.text_blocks.0.norm.weight")
            .unwrap();
        let _ = vb
            .get(text_dim, "transformer.text_embed.text_blocks.0.norm.bias")
            .unwrap();
        let _ = vb
            .get(
                (text_dim, 1, 7),
                "transformer.text_embed.text_blocks.0.dwconv.weight",
            )
            .unwrap();
        let _ = vb
            .get(text_dim, "transformer.text_embed.text_blocks.0.dwconv.bias")
            .unwrap();
        let _ = vb
            .get(
                (text_dim * 2, text_dim),
                "transformer.text_embed.text_blocks.0.pwconv1.weight",
            )
            .unwrap();
        let _ = vb
            .get(
                text_dim * 2,
                "transformer.text_embed.text_blocks.0.pwconv1.bias",
            )
            .unwrap();
        let _ = vb
            .get(
                (text_dim, text_dim * 2),
                "transformer.text_embed.text_blocks.0.pwconv2.weight",
            )
            .unwrap();
        let _ = vb
            .get(
                text_dim,
                "transformer.text_embed.text_blocks.0.pwconv2.bias",
            )
            .unwrap();

        for i in 1..4 {
            let _ = vb
                .get(
                    (1, 1, text_dim * 2),
                    &format!("transformer.text_embed.text_blocks.{}.grn.gamma", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    (1, 1, text_dim * 2),
                    &format!("transformer.text_embed.text_blocks.{}.grn.beta", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    text_dim,
                    &format!("transformer.text_embed.text_blocks.{}.norm.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    text_dim,
                    &format!("transformer.text_embed.text_blocks.{}.norm.bias", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    (text_dim, 1, 7),
                    &format!("transformer.text_embed.text_blocks.{}.dwconv.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    text_dim,
                    &format!("transformer.text_embed.text_blocks.{}.dwconv.bias", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    (text_dim * 2, text_dim),
                    &format!("transformer.text_embed.text_blocks.{}.pwconv1.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    text_dim * 2,
                    &format!("transformer.text_embed.text_blocks.{}.pwconv1.bias", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    (text_dim, text_dim * 2),
                    &format!("transformer.text_embed.text_blocks.{}.pwconv2.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    text_dim,
                    &format!("transformer.text_embed.text_blocks.{}.pwconv2.bias", i),
                )
                .unwrap();
        }

        let _ = vb
            .get(
                (text_dim, 1),
                "transformer.text_embed.content_duration_mlp.0.weight",
            )
            .unwrap();
        let _ = vb
            .get(
                text_dim,
                "transformer.text_embed.content_duration_mlp.0.bias",
            )
            .unwrap();
        let _ = vb
            .get(
                (text_dim, text_dim),
                "transformer.text_embed.content_duration_mlp.2.weight",
            )
            .unwrap();
        let _ = vb
            .get(
                text_dim,
                "transformer.text_embed.content_duration_mlp.2.bias",
            )
            .unwrap();

        let _ = vb
            .get(
                (text_dim, 1),
                "transformer.text_embed.pause_duration_mlp.0.weight",
            )
            .unwrap();
        let _ = vb
            .get(text_dim, "transformer.text_embed.pause_duration_mlp.0.bias")
            .unwrap();
        let _ = vb
            .get(
                (text_dim, text_dim),
                "transformer.text_embed.pause_duration_mlp.2.weight",
            )
            .unwrap();
        let _ = vb
            .get(text_dim, "transformer.text_embed.pause_duration_mlp.2.bias")
            .unwrap();

        let _ = vb.get(1, "transformer.text_embed.alpha_content").unwrap();
        let _ = vb.get(1, "transformer.text_embed.alpha_pause").unwrap();

        let _ = vb
            .get(
                (dim, freq_embed_dim),
                "transformer.time_embed.time_mlp.0.weight",
            )
            .unwrap();
        let _ = vb
            .get(dim, "transformer.time_embed.time_mlp.0.bias")
            .unwrap();
        let _ = vb
            .get((dim, dim), "transformer.time_embed.time_mlp.2.weight")
            .unwrap();
        let _ = vb
            .get(dim, "transformer.time_embed.time_mlp.2.bias")
            .unwrap();

        let _ = vb
            .get(
                (dim, mel_dim * 2 + text_dim),
                "transformer.input_embed.proj.weight",
            )
            .unwrap();
        let _ = vb.get(dim, "transformer.input_embed.proj.bias").unwrap();

        let _ = vb
            .get(
                (dim, 64, 31),
                "transformer.input_embed.conv_pos_embed.conv1d.0.weight",
            )
            .unwrap();
        let _ = vb
            .get(dim, "transformer.input_embed.conv_pos_embed.conv1d.0.bias")
            .unwrap();
        let _ = vb
            .get(
                (dim, 64, 31),
                "transformer.input_embed.conv_pos_embed.conv1d.2.weight",
            )
            .unwrap();
        let _ = vb
            .get(dim, "transformer.input_embed.conv_pos_embed.conv1d.2.bias")
            .unwrap();

        for i in 0..22 {
            let _ = vb
                .get(
                    (dim * 6, dim),
                    &format!(
                        "transformer.transformer_blocks.{}.attn_norm.linear.weight",
                        i
                    ),
                )
                .unwrap();
            let _ = vb
                .get(
                    dim * 6,
                    &format!("transformer.transformer_blocks.{}.attn_norm.linear.bias", i),
                )
                .unwrap();

            let _ = vb
                .get(
                    (1024, dim),
                    &format!("transformer.transformer_blocks.{}.attn.to_q.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    1024,
                    &format!("transformer.transformer_blocks.{}.attn.to_q.bias", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    (1024, dim),
                    &format!("transformer.transformer_blocks.{}.attn.to_k.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    1024,
                    &format!("transformer.transformer_blocks.{}.attn.to_k.bias", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    (1024, dim),
                    &format!("transformer.transformer_blocks.{}.attn.to_v.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    1024,
                    &format!("transformer.transformer_blocks.{}.attn.to_v.bias", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    (dim, 1024),
                    &format!("transformer.transformer_blocks.{}.attn.to_out.0.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    dim,
                    &format!("transformer.transformer_blocks.{}.attn.to_out.0.bias", i),
                )
                .unwrap();

            let _ = vb
                .get(
                    (dim * 2, dim),
                    &format!("transformer.transformer_blocks.{}.ff.ff.0.0.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    dim * 2,
                    &format!("transformer.transformer_blocks.{}.ff.ff.0.0.bias", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    (dim, dim * 2),
                    &format!("transformer.transformer_blocks.{}.ff.ff.2.weight", i),
                )
                .unwrap();
            let _ = vb
                .get(
                    dim,
                    &format!("transformer.transformer_blocks.{}.ff.ff.2.bias", i),
                )
                .unwrap();
        }

        let _ = vb
            .get((dim * 2, dim), "transformer.norm_out.linear.weight")
            .unwrap();
        let _ = vb.get(dim * 2, "transformer.norm_out.linear.bias").unwrap();

        let _ = vb
            .get((mel_dim, dim), "transformer.proj_out.weight")
            .unwrap();
        let _ = vb.get(mel_dim, "transformer.proj_out.bias").unwrap();
        let _ = vb
            .get((mel_dim, dim), "transformer.proj_out_ln_sig.weight")
            .unwrap();
        let _ = vb.get(mel_dim, "transformer.proj_out_ln_sig.bias").unwrap();

        let safetensors_path = test_dir.join("consolidated.safetensors");
        varmap.save(&safetensors_path).unwrap();

        let real_vocos = Path::new("/Users/jinliang/.aha/SCUT/MAGIC-TTS/vocos.onnx");
        if real_vocos.exists() {
            std::fs::copy(real_vocos, test_dir.join("vocos.onnx")).unwrap();
            let real_data = Path::new("/Users/jinliang/.aha/SCUT/MAGIC-TTS/vocos.onnx.data");
            if real_data.exists() {
                std::fs::copy(real_data, test_dir.join("vocos.onnx.data")).unwrap();
            }
        } else {
            std::fs::write(test_dir.join("vocos.onnx"), b"").unwrap();
        }

        let config = MagicTtsConfig {
            model_dir: test_dir.to_str().unwrap().to_string(),
            prompt_text: None,
            prompt_wav_path: None,
            vocab_path: Some("vocab.txt".to_string()),
            steps: Some(2),
            cfg_strength: Some(2.0),
            default_content_ms: Some(170.0),
            default_punct_ms: Some(50.0),
        };

        let mut adapter = MagicTtsAdapter::new(&config).unwrap();
        let tokens = adapter.tokenize("前方路口，左转。");
        assert!(tokens.contains(&"qian2".to_string()));

        let pcm = adapter
            .synthesize_chunk("前方路口[260]左{300}转{300}。")
            .unwrap();
        assert!(pcm.len() > 0);

        let _ = std::fs::remove_dir_all(&test_dir);
    }
}
