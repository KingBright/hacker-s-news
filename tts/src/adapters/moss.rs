use crate::config::{MossTtsConfig, VoicePrompt};
use crate::engine::TtsEngine;
use anyhow::{anyhow, Result};
use ort::error::Error as OrtError;
use ort::session::builder::SessionBuilder;
use ort::session::Session;
use ort::AsPointer;
use qwen3_tts::audio::{resample, AudioBuffer};
use rand::{Rng, SeedableRng};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use tokenizers::Tokenizer;

pub struct MossTtsAdapter {
    prefill_session: Session,
    fixed_session: Option<Session>,
    greedy_session: Option<Session>,
    codec_encode_session: Session,
    codec_decode_session: Session,
    decode_session: Session,

    tokenizer: Tokenizer,
    manifest: serde_json::Value,

    // 超参数
    sample_mode: String,
    text_temperature: f32,
    text_top_p: f32,
    text_top_k: usize,
    audio_temperature: f32,
    audio_top_p: f32,
    audio_top_k: usize,
    audio_repetition_penalty: f32,
    max_new_frames: usize,
    voice_clone_max_text_tokens: usize,
    seed: Option<u64>,
    intra_threads: usize,
    inter_threads: usize,

    // 缓存的 VQ 码
    cached_prompt_codes: Option<Vec<Vec<i32>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MossOrtExecutionProvider {
    Cpu,
    CoreMl,
}

impl MossTtsAdapter {
    pub fn new(config: &MossTtsConfig, device: Option<&str>) -> Result<Self> {
        let model_dir = Path::new(&config.model_dir);
        let execution_provider = moss_execution_provider_from_device(device)?;

        // 1. 寻找 manifest.json
        let relative_candidates = [
            "browser_poc_manifest.json",
            "MOSS-TTS-Nano-100M-ONNX/browser_poc_manifest.json",
            "MOSS-TTS-Nano-ONNX-CPU/browser_poc_manifest.json",
        ];

        let mut manifest_path = None;
        for rel in &relative_candidates {
            let path = model_dir.join(rel);
            if path.exists() {
                manifest_path = Some(path);
                break;
            }
        }

        let manifest_path = manifest_path.ok_or_else(|| {
            anyhow!(
                "MOSS-TTS-Nano: browser_poc_manifest.json not found in {}",
                config.model_dir
            )
        })?;

        let manifest_dir = manifest_path.parent().unwrap();
        let file = File::open(&manifest_path)?;
        let reader = BufReader::new(file);
        let manifest: serde_json::Value = serde_json::from_reader(reader)?;

        // 2. 加载 Tokenizer
        let tokenizer_json_path = manifest_dir.join("tokenizer.json");
        if !tokenizer_json_path.exists() {
            return Err(anyhow!("MOSS-TTS-Nano: tokenizer.json not found in manifest directory. Run conversion script first."));
        }
        log::info!(
            "MOSS-TTS-Nano: Loading tokenizer from {:?}",
            tokenizer_json_path
        );
        let tokenizer = Tokenizer::from_file(&tokenizer_json_path)
            .map_err(|e| anyhow!("Failed to load tokenizer.json: {}", e))?;

        // 3. 读取 ONNX 模型名称
        let prefill_name = manifest["model_files"]
            .get("prefill")
            .or_else(|| manifest["files"].get("prefill"))
            .and_then(|v| v.as_str())
            .unwrap_or("moss_tts_prefill.onnx");

        let _decode_name = manifest["model_files"]
            .get("decode_step")
            .or_else(|| manifest["files"].get("decode_step"))
            .and_then(|v| v.as_str())
            .unwrap_or("moss_tts_decode_step.onnx");

        let _local_decoder_name = manifest["model_files"]
            .get("local_decoder")
            .or_else(|| manifest["files"].get("local_decoder"))
            .and_then(|v| v.as_str())
            .unwrap_or("moss_tts_local_decoder.onnx");

        let fixed_name = manifest["model_files"]
            .get("local_fixed_sampled_frame")
            .or_else(|| manifest["files"].get("local_fixed_sampled_frame"))
            .and_then(|v| v.as_str())
            .unwrap_or("moss_tts_local_fixed_sampled_frame.onnx");

        let greedy_name = manifest["model_files"]
            .get("local_greedy_frame")
            .or_else(|| manifest["files"].get("local_greedy_frame"))
            .and_then(|v| v.as_str())
            .unwrap_or("moss_tts_local_greedy_frame.onnx");

        // Codec 相对路径通常指向上级目录
        let codec_meta_rel = manifest["model_files"]
            .get("codec_meta")
            .and_then(|v| v.as_str())
            .unwrap_or("../MOSS-Audio-Tokenizer-Nano-ONNX/codec_browser_onnx_meta.json");

        let codec_meta_path = manifest_dir.join(codec_meta_rel);
        let codec_dir = codec_meta_path
            .parent()
            .ok_or_else(|| anyhow!("Invalid codec meta path"))?;

        let codec_file = File::open(&codec_meta_path)?;
        let codec_reader = BufReader::new(codec_file);
        let codec_meta: serde_json::Value = serde_json::from_reader(codec_reader)?;

        let codec_encode_name = codec_meta["files"]
            .get("encode")
            .and_then(|v| v.as_str())
            .unwrap_or("moss_audio_tokenizer_encode.onnx");
        let codec_decode_name = codec_meta["files"]
            .get("decode_full")
            .and_then(|v| v.as_str())
            .unwrap_or("moss_audio_tokenizer_decode_full.onnx");

        // 4. 初始化 ONNX sessions (应用线程数和图优化级别配置)
        let intra_threads = config.intra_threads.unwrap_or(4);
        let inter_threads = config.inter_threads.unwrap_or(1);
        println!(
            "MOSS-TTS-Nano: Initializing ONNX sessions with intra_threads={}, inter_threads={}, provider={:?}...",
            intra_threads, inter_threads, execution_provider
        );

        let create_session = |path: &Path| -> Result<Session> {
            let mut builder = Session::builder()
                .map_err(|e| anyhow!("Failed to create SessionBuilder: {}", e))?
                .with_intra_threads(intra_threads)
                .map_err(|e| anyhow!("Failed to set intra threads to {}: {}", intra_threads, e))?
                .with_inter_threads(inter_threads)
                .map_err(|e| anyhow!("Failed to set inter threads to {}: {}", inter_threads, e))?;

            if execution_provider == MossOrtExecutionProvider::CoreMl {
                builder = configure_moss_coreml_provider(builder)?;
            }

            let session = builder
                .commit_from_file(path)
                .map_err(|e| anyhow!("Failed to load session from {:?}: {}", path, e))?;
            Ok(session)
        };

        let prefill_path = manifest_dir.join(prefill_name);
        println!(
            "MOSS-TTS-Nano: Loading prefill_session from {:?}",
            prefill_path
        );
        let prefill_session = create_session(&prefill_path)?;
        println!("MOSS-TTS-Nano: Loaded prefill_session successfully");

        let fixed_path = manifest_dir.join(fixed_name);
        let fixed_session = if fixed_path.exists() {
            println!("MOSS-TTS-Nano: Loading fixed_session from {:?}", fixed_path);
            let s = create_session(&fixed_path)?;
            println!("MOSS-TTS-Nano: Loaded fixed_session successfully");
            Some(s)
        } else {
            println!("MOSS-TTS-Nano: fixed_session path does not exist");
            None
        };

        let greedy_path = manifest_dir.join(greedy_name);
        let greedy_session = if greedy_path.exists() {
            println!(
                "MOSS-TTS-Nano: Loading greedy_session from {:?}",
                greedy_path
            );
            let s = create_session(&greedy_path)?;
            println!("MOSS-TTS-Nano: Loaded greedy_session successfully");
            Some(s)
        } else {
            println!("MOSS-TTS-Nano: greedy_session path does not exist");
            None
        };

        let codec_encode_path = codec_dir.join(codec_encode_name);
        println!(
            "MOSS-TTS-Nano: Loading codec_encode_session from {:?}",
            codec_encode_path
        );
        let codec_encode_session = create_session(&codec_encode_path)?;
        println!("MOSS-TTS-Nano: Loaded codec_encode_session successfully");

        let codec_decode_path = codec_dir.join(codec_decode_name);
        println!(
            "MOSS-TTS-Nano: Loading codec_decode_session from {:?}",
            codec_decode_path
        );
        let codec_decode_session = create_session(&codec_decode_path)?;
        println!("MOSS-TTS-Nano: Loaded codec_decode_session successfully");

        let decode_path = manifest_dir.join(_decode_name);
        println!(
            "MOSS-TTS-Nano: Loading decode_session from {:?}",
            decode_path
        );
        let decode_session = create_session(&decode_path)?;
        println!("MOSS-TTS-Nano: Loaded decode_session successfully");

        // 5. 设置超参数，后备采用 manifests 的默认配置
        let sample_mode = config.sample_mode.clone().unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("sample_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("fixed")
                .to_string()
        });

        let text_temperature = config.text_temperature.unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("text_temperature")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0)
        }) as f32;

        let text_top_p = config.text_top_p.unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("text_top_p")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0)
        }) as f32;

        let text_top_k = config.text_top_k.unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("text_top_k")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize
        });

        let audio_temperature = config.audio_temperature.unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("audio_temperature")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.8)
        }) as f32;

        let audio_top_p = config.audio_top_p.unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("audio_top_p")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.95)
        }) as f32;

        let audio_top_k = config.audio_top_k.unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("audio_top_k")
                .and_then(|v| v.as_u64())
                .unwrap_or(25) as usize
        });

        let audio_repetition_penalty = config.audio_repetition_penalty.unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("audio_repetition_penalty")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.2)
        }) as f32;

        let max_new_frames = config.max_new_frames.unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("max_new_frames")
                .and_then(|v| v.as_u64())
                .unwrap_or(375) as usize
        });

        let voice_clone_max_text_tokens = config.voice_clone_max_text_tokens.unwrap_or_else(|| {
            manifest["generation_defaults"]
                .get("voice_clone_max_text_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(75) as usize
        });

        Ok(Self {
            prefill_session,
            fixed_session,
            greedy_session,
            codec_encode_session,
            codec_decode_session,
            decode_session,
            tokenizer,
            manifest,
            sample_mode,
            text_temperature,
            text_top_p,
            text_top_k,
            audio_temperature,
            audio_top_p,
            audio_top_k,
            audio_repetition_penalty,
            max_new_frames,
            voice_clone_max_text_tokens,
            seed: config.seed,
            intra_threads: intra_threads as usize,
            inter_threads: inter_threads as usize,
            cached_prompt_codes: None,
        })
    }
}

fn moss_execution_provider_from_device(device: Option<&str>) -> Result<MossOrtExecutionProvider> {
    match device.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(MossOrtExecutionProvider::Cpu),
        Some(value) if value.eq_ignore_ascii_case("cpu") => Ok(MossOrtExecutionProvider::Cpu),
        Some(value)
            if value.eq_ignore_ascii_case("gpu")
                || value.eq_ignore_ascii_case("metal")
                || value.eq_ignore_ascii_case("mps")
                || value.eq_ignore_ascii_case("coreml") =>
        {
            Ok(MossOrtExecutionProvider::CoreMl)
        }
        Some(value) => Err(anyhow!(
            "MOSS-TTS-Nano: unsupported device '{}'; use cpu or coreml/gpu/metal",
            value
        )),
    }
}

fn configure_moss_coreml_provider(builder: SessionBuilder) -> Result<SessionBuilder> {
    let mut builder = builder;
    apply_moss_coreml_provider_ort_1_19(&mut builder)?;
    Ok(builder)
}

fn apply_moss_coreml_provider_ort_1_19(builder: &mut SessionBuilder) -> Result<()> {
    type AppendCoreMlFn =
        unsafe extern "C" fn(*mut ort::sys::OrtSessionOptions, u32) -> ort::sys::OrtStatusPtr;

    let dylib_path =
        std::env::var("ORT_DYLIB_PATH").unwrap_or_else(|_| "libonnxruntime.dylib".to_string());
    let library = unsafe { libloading::Library::new(&dylib_path) }.map_err(|e| {
        anyhow!(
            "MOSS-TTS-Nano: failed to open ORT dylib {}: {}",
            dylib_path,
            e
        )
    })?;
    let append_coreml: libloading::Symbol<AppendCoreMlFn> = unsafe {
        library
            .get(b"OrtSessionOptionsAppendExecutionProvider_CoreML\0")
            .map_err(|e| {
                anyhow!(
                    "MOSS-TTS-Nano: ORT dylib does not expose CoreML provider API: {}",
                    e
                )
            })?
    };

    unsafe {
        OrtError::result_from_status(append_coreml(
            builder.ptr_mut(),
            moss_coreml_provider_flags(),
        ))
    }
    .map_err(|e| anyhow!("MOSS-TTS-Nano: failed to append CoreML provider: {}", e))?;
    Ok(())
}

fn moss_coreml_provider_flags() -> u32 {
    const COREML_FLAG_ENABLE_ON_SUBGRAPH: u32 = 0x002;
    const COREML_FLAG_ONLY_ALLOW_STATIC_INPUT_SHAPES: u32 = 0x008;

    COREML_FLAG_ENABLE_ON_SUBGRAPH | COREML_FLAG_ONLY_ALLOW_STATIC_INPUT_SHAPES
}

#[async_trait::async_trait]
impl TtsEngine for MossTtsAdapter {
    fn cache_voice_prompt(&mut self, prompt: &VoicePrompt) -> Result<()> {
        if let Some(wav_path) = &prompt.wav_path {
            let clean_path = wav_path.strip_prefix("file://").unwrap_or(wav_path);
            println!(
                "MOSS-TTS-Nano: Loading custom voice prompt from {:?}",
                clean_path
            );

            let audio = AudioBuffer::load(clean_path)?;
            println!(
                "MOSS-TTS-Nano: Loaded AudioBuffer: rate={}, length={}",
                audio.sample_rate,
                audio.samples.len()
            );

            // MOSS Codec 采样率固定为 48000Hz (48kHz)
            let audio_48k = if audio.sample_rate != 48000 {
                println!("MOSS-TTS-Nano: Resampling to 48kHz...");
                let r = resample(&audio, 48000)?;
                println!("MOSS-TTS-Nano: Resampled successfully");
                r
            } else {
                audio
            };

            // 构造 2通道 形状为 [1, 2, num_samples] 的 float32 tensor
            let num_samples = audio_48k.samples.len();
            let mut waveform_flat = Vec::with_capacity(num_samples * 2);
            // 复制单声道数据为双声道立体声 (Planar 格式：先是所有左声道，再是所有右声道)
            waveform_flat.extend_from_slice(&audio_48k.samples);
            waveform_flat.extend_from_slice(&audio_48k.samples);

            let waveform_val = ort::value::Value::from_array(([1, 2, num_samples], waveform_flat))?;
            let input_lengths_val = ort::value::Value::from_array(([1], vec![num_samples as i32]))?;

            println!("MOSS-TTS-Nano: Extracting prompt audio VQ codes...");
            let outputs = self.codec_encode_session.run(ort::inputs![
                "waveform" => waveform_val,
                "input_lengths" => input_lengths_val
            ])?;
            println!("MOSS-TTS-Nano: Finished codec_encode_session run successfully");

            let audio_codes_out = outputs["audio_codes"].try_extract_tensor::<i32>()?;
            let code_lengths_out = outputs["audio_code_lengths"].try_extract_tensor::<i32>()?;
            let code_length = code_lengths_out.1[0] as usize;

            // VQ 层数一般为 16
            let n_vq = self.manifest["tts_config"]["n_vq"].as_u64().unwrap_or(16) as usize;
            let mut prompt_codes = Vec::with_capacity(code_length);

            for f in 0..code_length {
                let mut frame_codes = Vec::with_capacity(n_vq);
                for q in 0..n_vq {
                    // index 逻辑：[0, frame_index, quantizer_index] -> 对应 flattened array 中对应的数据
                    // shape 一般是 [1, code_length, n_vq]
                    let idx = f * n_vq + q;
                    frame_codes.push(audio_codes_out.1[idx]);
                }
                prompt_codes.push(frame_codes);
            }

            self.cached_prompt_codes = Some(prompt_codes);
        } else {
            self.cached_prompt_codes = None;
        }
        Ok(())
    }

    fn sample_rate(&self) -> usize {
        48000
    }

    fn synthesize_chunk(&mut self, chunk: &str) -> Result<Vec<f32>> {
        let mut clean_text = chunk.trim().to_string();
        if clean_text.is_empty() {
            return Ok(Vec::new());
        }

        // 规范化标点符号以对齐 SentencePiece 预处理
        clean_text = clean_text
            .replace("，", ",")
            .replace("；", ";")
            .replace("：", ":")
            .replace("？", "?")
            .replace("！", "!")
            .replace("（", "(")
            .replace("）", ")")
            .replace("【", "[")
            .replace("】", "]")
            .replace("「", "\"")
            .replace("」", "\"")
            .replace("『", "\"")
            .replace("』", "\"");

        // 1. 分词获取文本 tokens，禁用 add_special_tokens (即不自动添加 <s>)
        // 手动在 clean_text 最前面加一个空格以对齐 SentencePiece 的 add_dummy_prefix 行为
        let text_for_tokenization = format!(" {}", clean_text);
        let encoding = self
            .tokenizer
            .encode(text_for_tokenization.as_str(), false)
            .map_err(|e| anyhow!("Tokenization failed: {}", e))?;
        let text_token_ids: Vec<u32> = encoding.get_ids().to_vec();

        // 2. 读取 VQ codes 配置
        let n_vq = self.manifest["tts_config"]["n_vq"].as_u64().unwrap_or(16) as usize;
        let row_width = n_vq + 1; // 17
        let audio_pad_token_id = self.manifest["tts_config"]["audio_pad_token_id"]
            .as_i64()
            .unwrap_or(1024) as i32;
        let audio_start_token_id = self.manifest["tts_config"]["audio_start_token_id"]
            .as_i64()
            .unwrap_or(6) as i32;
        let audio_end_token_id = self.manifest["tts_config"]["audio_end_token_id"]
            .as_i64()
            .unwrap_or(7) as i32;
        let audio_user_slot_token_id = self.manifest["tts_config"]["audio_user_slot_token_id"]
            .as_i64()
            .unwrap_or(8) as i32;
        let audio_assistant_slot_token_id = self.manifest["tts_config"]
            ["audio_assistant_slot_token_id"]
            .as_i64()
            .unwrap_or(9) as i32;

        // 3. 决定参考音频 codes 序列
        let prompt_codes = if let Some(cached) = &self.cached_prompt_codes {
            cached.clone()
        } else {
            // 回退到 manifests 中第一个内置声音的 codes
            let voice_presets = self.manifest["builtin_voices"]
                .as_array()
                .ok_or_else(|| anyhow!("builtin_voices missing"))?;
            let preset = &voice_presets[0];
            let codes_array = preset["prompt_audio_codes"]
                .as_array()
                .ok_or_else(|| anyhow!("prompt_audio_codes missing in default voice"))?;

            let mut codes = Vec::with_capacity(codes_array.len());
            for item in codes_array {
                let frame_array = item
                    .as_array()
                    .ok_or_else(|| anyhow!("Invalid prompt code frame"))?;
                let mut frame_codes = Vec::with_capacity(n_vq);
                for q in 0..n_vq {
                    frame_codes.push(frame_array[q].as_i64().unwrap_or(0) as i32);
                }
                codes.push(frame_codes);
            }
            codes
        };

        // 4. 读取 prompt 序列模版并拼接 input_ids
        let user_prompt_prefix: Vec<i32> = self.manifest["prompt_templates"]
            ["user_prompt_prefix_token_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i32)
            .collect();

        let user_prompt_after_ref: Vec<i32> = self.manifest["prompt_templates"]
            ["user_prompt_after_reference_token_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i32)
            .collect();

        let assistant_prompt_prefix: Vec<i32> = self.manifest["prompt_templates"]
            ["assistant_prompt_prefix_token_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i32)
            .collect();

        // 构造拼接：
        // (a) User Prompt Prefix
        // (b) Audio Start Token
        // (c) Prompt Audio Codes (每一帧开头是 audio_user_slot_token_id)
        // (d) Audio End Token
        // (e) User Prompt After Reference
        // (f) Synthesized Text (每一行开头是 text_token_id, 其它用 pad)
        // (g) Assistant Prompt Prefix
        // (h) Audio Start Token
        let mut input_rows = Vec::new();

        // Helper 构造文本行
        let build_text_row = |tid: i32| {
            let mut row = vec![audio_pad_token_id; row_width];
            row[0] = tid;
            row
        };

        // (a)
        for &tid in &user_prompt_prefix {
            input_rows.push(build_text_row(tid));
        }
        // (b)
        input_rows.push(build_text_row(audio_start_token_id));
        // (c)
        for frame in &prompt_codes {
            let mut row = vec![audio_pad_token_id; row_width];
            row[0] = audio_user_slot_token_id;
            for q in 0..n_vq {
                row[q + 1] = frame[q];
            }
            input_rows.push(row);
        }
        // (d)
        input_rows.push(build_text_row(audio_end_token_id));
        // (e)
        for &tid in &user_prompt_after_ref {
            input_rows.push(build_text_row(tid));
        }
        // (f)
        for &tid in &text_token_ids {
            input_rows.push(build_text_row(tid as i32));
        }
        // (g)
        for &tid in &assistant_prompt_prefix {
            input_rows.push(build_text_row(tid));
        }
        // (h)
        input_rows.push(build_text_row(audio_start_token_id));

        let seq_len = input_rows.len();
        let mut input_ids_flat = Vec::with_capacity(seq_len * row_width);
        for row in input_rows {
            input_ids_flat.extend(row);
        }

        let attention_mask_flat = vec![1i32; seq_len];

        // 5. 运行 Prefill 获取 global_hidden
        let input_ids_val =
            ort::value::Value::from_array(([1, seq_len, row_width], input_ids_flat))?;
        let attention_mask_val =
            ort::value::Value::from_array(([1, seq_len], attention_mask_flat))?;

        let mut prefill_outputs = self.prefill_session.run(ort::inputs![
            "input_ids" => input_ids_val,
            "attention_mask" => attention_mask_val
        ])?;

        let global_hidden_out = prefill_outputs["global_hidden"].try_extract_tensor::<f32>()?;
        let hidden_dim = global_hidden_out.0[2] as usize; // 一般是 768 或 512，这里我们根据输出维度自适应

        // 提取最后一个 token 的隐藏层表示 [1, hidden_dim]
        let start_idx = (seq_len - 1) * hidden_dim;
        let last_hidden = global_hidden_out.1[start_idx..start_idx + hidden_dim].to_vec();
        let mut global_hidden_val = ort::value::Value::from_array(([1, hidden_dim], last_hidden))?;

        // 提取 KV Cache 状态
        let mut past_keys = Vec::with_capacity(12);
        let mut past_values = Vec::with_capacity(12);
        for i in 0..12 {
            let k_name = format!("present_key_{}", i);
            let v_name = format!("present_value_{}", i);
            past_keys.push(
                prefill_outputs
                    .remove(&k_name)
                    .ok_or_else(|| anyhow!("Missing {}", k_name))?,
            );
            past_values.push(
                prefill_outputs
                    .remove(&v_name)
                    .ok_or_else(|| anyhow!("Missing {}", v_name))?,
            );
        }
        let mut past_valid_length = seq_len;

        // 6. 自回归生成循环
        let mut generated_frames = Vec::new();

        // 准备 repetition penalty 相关的结构
        // 二维 state_seen: channel x 1024
        let mut previous_token_sets = vec![std::collections::HashSet::new(); n_vq];

        let greedy_mode = self.sample_mode == "greedy";

        let mut rng = if self.seed.is_some() {
            // 在简单的 CPU 测试上支持可选的 seed
            rand::rngs::StdRng::seed_from_u64(self.seed.unwrap())
        } else {
            // 在这里我们虽然不用 StdRng 也可以直接使用随机数
            rand::rngs::StdRng::from_entropy()
        };

        for _ in 0..self.max_new_frames {
            // 准备 repetition_seen_mask [1, n_vq, 1024]
            let mut seen_mask_flat = vec![0i32; n_vq * 1024];
            for q in 0..n_vq {
                for &token in &previous_token_sets[q] {
                    if token < 1024 {
                        seen_mask_flat[q * 1024 + token] = 1;
                    }
                }
            }
            let seen_mask_val = ort::value::Value::from_array(([1, n_vq, 1024], seen_mask_flat))?;

            let (should_continue, frame) = if greedy_mode {
                let greedy_session = self
                    .greedy_session
                    .as_mut()
                    .or(self.fixed_session.as_mut())
                    .ok_or_else(|| anyhow!("MOSS-TTS-Nano: greedy session not loaded"))?;

                let penalty_val =
                    ort::value::Value::from_array(([1], vec![self.audio_repetition_penalty]))?;

                let outputs = greedy_session.run(ort::inputs![
                    "global_hidden" => global_hidden_val.view(),
                    "repetition_seen_mask" => seen_mask_val,
                    "repetition_penalty" => penalty_val
                ])?;

                let should_continue_out = outputs["should_continue"].try_extract_tensor::<i32>()?;
                let frame_tokens_out = outputs["frame_token_ids"].try_extract_tensor::<i32>()?;

                let cont = should_continue_out.1[0] != 0;
                let frame_vec = frame_tokens_out.1.to_vec();
                (cont, frame_vec)
            } else {
                let fixed_session = self
                    .fixed_session
                    .as_mut()
                    .or(self.greedy_session.as_mut())
                    .ok_or_else(|| anyhow!("MOSS-TTS-Nano: fixed session not loaded"))?;

                let assistant_u = rng.gen::<f32>().clamp(0.0, 0.99999994);
                let mut audio_u = Vec::with_capacity(n_vq);
                for _ in 0..n_vq {
                    audio_u.push(rng.gen::<f32>().clamp(0.0, 0.99999994));
                }

                let assistant_u_val = ort::value::Value::from_array(([1], vec![assistant_u]))?;
                let audio_u_val = ort::value::Value::from_array(([1, n_vq], audio_u))?;

                let outputs = fixed_session.run(ort::inputs![
                    "global_hidden" => global_hidden_val.view(),
                    "repetition_seen_mask" => seen_mask_val,
                    "assistant_random_u" => assistant_u_val,
                    "audio_random_u" => audio_u_val
                ])?;

                let should_continue_out = outputs["should_continue"].try_extract_tensor::<i32>()?;
                let frame_tokens_out = outputs["frame_token_ids"].try_extract_tensor::<i32>()?;

                let cont = should_continue_out.1[0] != 0;
                let frame_vec = frame_tokens_out.1.to_vec();
                (cont, frame_vec)
            };

            if !should_continue {
                break;
            }

            // 保存这一帧
            for q in 0..n_vq {
                let t = frame[q] as usize;
                previous_token_sets[q].insert(t);
            }
            generated_frames.push(frame.clone());

            // 调用 decode_session 更新 KV cache
            let mut next_row = vec![audio_pad_token_id; row_width];
            next_row[0] = audio_assistant_slot_token_id;
            for q in 0..n_vq {
                next_row[q + 1] = frame[q];
            }
            let next_row_val = ort::value::Value::from_array(([1, 1, row_width], next_row))?;
            let past_valid_lengths_val =
                ort::value::Value::from_array(([1], vec![past_valid_length as i32]))?;

            let mut decode_outputs = self.decode_session.run(ort::inputs![
                "input_ids" => next_row_val,
                "past_valid_lengths" => past_valid_lengths_val,
                "past_key_0" => past_keys[0].view(),
                "past_value_0" => past_values[0].view(),
                "past_key_1" => past_keys[1].view(),
                "past_value_1" => past_values[1].view(),
                "past_key_2" => past_keys[2].view(),
                "past_value_2" => past_values[2].view(),
                "past_key_3" => past_keys[3].view(),
                "past_value_3" => past_values[3].view(),
                "past_key_4" => past_keys[4].view(),
                "past_value_4" => past_values[4].view(),
                "past_key_5" => past_keys[5].view(),
                "past_value_5" => past_values[5].view(),
                "past_key_6" => past_keys[6].view(),
                "past_value_6" => past_values[6].view(),
                "past_key_7" => past_keys[7].view(),
                "past_value_7" => past_values[7].view(),
                "past_key_8" => past_keys[8].view(),
                "past_value_8" => past_values[8].view(),
                "past_key_9" => past_keys[9].view(),
                "past_value_9" => past_values[9].view(),
                "past_key_10" => past_keys[10].view(),
                "past_value_10" => past_values[10].view(),
                "past_key_11" => past_keys[11].view(),
                "past_value_11" => past_values[11].view()
            ])?;

            let next_global_hidden = decode_outputs["global_hidden"].try_extract_tensor::<f32>()?;
            let last_hidden = next_global_hidden.1.to_vec();
            global_hidden_val = ort::value::Value::from_array(([1, hidden_dim], last_hidden))?;

            for i in 0..12 {
                past_keys[i] = decode_outputs
                    .remove(&format!("present_key_{}", i))
                    .ok_or_else(|| anyhow!("missing present_key_{}", i))?;
                past_values[i] = decode_outputs
                    .remove(&format!("present_value_{}", i))
                    .ok_or_else(|| anyhow!("missing present_value_{}", i))?;
            }
            past_valid_length += 1;
        }

        let total_frames = generated_frames.len();
        if total_frames == 0 {
            return Err(anyhow!(
                "MOSS-TTS-Nano: Generated 0 frames, synthesis aborted."
            ));
        }

        // 7. 解码生成的 VQ 码
        let mut audio_codes_flat: Vec<i32> = Vec::with_capacity(total_frames * n_vq);
        for frame in &generated_frames {
            audio_codes_flat.extend(frame);
        }

        let audio_codes_val =
            ort::value::Value::from_array(([1, total_frames, n_vq], audio_codes_flat))?;
        let code_lengths_val = ort::value::Value::from_array(([1], vec![total_frames as i32]))?;

        log::info!("MOSS-TTS-Nano: Decoding VQ codes to raw audio waveform...");
        let codec_outputs = self.codec_decode_session.run(ort::inputs![
            "audio_codes" => audio_codes_val,
            "audio_code_lengths" => code_lengths_val
        ])?;

        let audio_out = codec_outputs["audio"].try_extract_tensor::<f32>()?;
        let audio_lengths_out = codec_outputs["audio_lengths"].try_extract_tensor::<i32>()?;

        let audio_length = audio_lengths_out.1[0] as usize;

        // 8. 转换双声道立体声为单声道 (Planar 格式：[1, 2, audio_length])
        let mut pcm = Vec::with_capacity(audio_length);
        for i in 0..audio_length {
            let left = audio_out.1[i];
            let right = audio_out.1[audio_length + i];
            pcm.push((left + right) / 2.0);
        }

        log::info!(
            "MOSS-TTS-Nano: Generated {} raw float audio samples at 48kHz.",
            pcm.len()
        );
        Ok(pcm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MossTtsConfig;

    #[test]
    fn coreml_provider_flags_skip_dynamic_input_shapes() {
        assert_eq!(moss_coreml_provider_flags() & 0x002, 0x002);
        assert_eq!(moss_coreml_provider_flags() & 0x008, 0x008);
    }

    #[test]
    fn moss_gpu_aliases_request_coreml_provider() {
        assert_eq!(
            moss_execution_provider_from_device(Some("gpu")).unwrap(),
            MossOrtExecutionProvider::CoreMl
        );
        assert_eq!(
            moss_execution_provider_from_device(Some("metal")).unwrap(),
            MossOrtExecutionProvider::CoreMl
        );
        assert_eq!(
            moss_execution_provider_from_device(Some("coreml")).unwrap(),
            MossOrtExecutionProvider::CoreMl
        );
    }

    #[test]
    fn test_moss_tts_adapter_and_synthesis() {
        let model_dir = "/Users/jinliang/.aha/OpenMOSS/MOSS-TTS-Nano";

        if !Path::new(model_dir).exists() {
            println!("Skipping MOSS-TTS-Nano test: model directory not found.");
            return;
        }

        let config = MossTtsConfig {
            model_dir: model_dir.to_string(),
            prompt_text: None,
            prompt_wav_path: None,
            sample_mode: Some("fixed".to_string()),
            text_temperature: None,
            text_top_p: None,
            text_top_k: None,
            audio_temperature: None,
            audio_top_p: None,
            audio_top_k: None,
            audio_repetition_penalty: None,
            max_new_frames: Some(20), // 仅合成 20 帧，加快测试速度
            voice_clone_max_text_tokens: None,
            seed: Some(42),
            intra_threads: None,
            inter_threads: None,
            chunk_max_chars: None,
        };

        let mut adapter = MossTtsAdapter::new(&config, Some("cpu")).unwrap();
        assert_eq!(adapter.sample_rate(), 48000);

        // 使用默认内置声音测试一句话
        let pcm = adapter
            .synthesize_chunk("你好，这是纯 Rust 推理测试。")
            .unwrap();
        assert!(pcm.len() > 0);
        println!("Synthesized {} float samples.", pcm.len());
    }
}
