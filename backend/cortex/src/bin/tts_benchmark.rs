use anyhow::Result;
use std::path::Path;
use std::time::Instant;
use sysinfo::System;
use tts::{
    EngineFactory, MagicTtsConfig, Qwen3Config, TtsConfig, TtsEngine, VoicePrompt, VoxCpmConfig,
};

fn get_rss_mb() -> f64 {
    let mut sys = System::new_all();
    sys.refresh_all();
    let pid = sysinfo::get_current_pid().unwrap();
    if let Some(proc) = sys.process(pid) {
        proc.memory() as f64 / 1024.0 / 1024.0
    } else {
        0.0
    }
}

fn create_wav(samples: &[f32], rate: usize) -> Vec<u8> {
    use std::io::Cursor;
    let mut cursor = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
    for &sample in samples {
        let s = (sample.max(-1.0).min(1.0) * 32767.0) as i16;
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();
    cursor.into_inner()
}

struct BenchResult {
    name: String,
    load_ms: u128,
    ttft_ms: u128,
    gen_ms: u128,
    rtf: f64,
    rss_mb: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();

    println!("🚀 Starting TTS Engine Benchmark comparison!\n");

    let test_text = "你好，欢迎收听黑客新闻，华为致力于把数字世界带给每个人。";
    let mut audio_ref = "../../aha/assets/audio/voice_01.wav";
    if !Path::new(audio_ref).exists() {
        if Path::new("../aha/assets/audio/voice_01.wav").exists() {
            audio_ref = "../aha/assets/audio/voice_01.wav";
        } else if Path::new("aha/assets/audio/voice_01.wav").exists() {
            audio_ref = "aha/assets/audio/voice_01.wav";
        }
    }
    let prompt_text =
        "华为致力于把数字世界带给每个人，每个家庭，每个组织，构建万物互联的智能世界。";

    println!("========== CONFIGURATION ==========");
    println!("Prompt Text: {}", test_text);
    println!("Reference Audio: {}", audio_ref);
    println!("Reference Text: {}", prompt_text);
    println!("===================================\n");

    let prompt = VoicePrompt {
        text: Some(prompt_text.to_string()),
        wav_path: Some(format!("file://{}", audio_ref)),
    };

    let mut results = Vec::new();

    // ================= VoxCPM 2 (Original) ==================
    let vox_path = dirs::home_dir()
        .unwrap()
        .join(".aha/OpenBMB/VoxCPM2")
        .to_string_lossy()
        .to_string();
    if Path::new(&vox_path).exists() {
        println!("⏳ [1/4] Loading VoxCPM 2 (Original) Adapter...");
        let config = TtsConfig {
            engine: Some("voxcpm".into()),
            device: Some("metal".to_string()),
            voxcpm: Some(VoxCpmConfig {
                model_path: vox_path.clone(),
                prompt_text: prompt.text.clone(),
                prompt_wav_path: prompt.wav_path.clone(),
                control_instruction: Some("自然新闻播报风格，语速中等".to_string()),
                min_len: None,
                max_len: Some(1024),
                inference_timesteps: Some(8),
                cfg_value: Some(1.8),
                retry_badcase: None,
                retry_badcase_ratio_threshold: None,
            }),
            qwen3: None,
            magictts: None,
            moss: None,
        };
        let i_load = Instant::now();
        let mut engine = EngineFactory::create(&config)?;
        let load_ms = i_load.elapsed().as_millis();

        // 预热 Metal 着色器内核以避免冷启动对 TTFT 的统计干扰
        println!("🔥 Warming up Metal shaders for VoxCPM 2 (Original)...");
        let _ = engine.synthesize_chunk("啊")?;
        // 长句物理预热填充 Page Cache
        println!("🔥 Running long text warmup to fill Page Cache...");
        let _ = engine.synthesize_long_text(test_text).await?;

        println!("🎙️ Synthesizing with VoxCPM 2 (Original)...");
        // Measure TTFT
        let chunks = tts::audio::chunk_text(test_text);
        let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("你好");
        let i_first = Instant::now();
        let _ = engine.synthesize_chunk(first_chunk)?;
        let ttft_ms = i_first.elapsed().as_millis();

        // Total Generation
        let i_gen = Instant::now();
        let samples = engine.synthesize_long_text(test_text).await?;
        let gen_ms = i_gen.elapsed().as_millis();
        let rtf = (gen_ms as f64 / 1000.0) / (samples.len() as f64 / engine.sample_rate() as f64);
        let rss_mb = get_rss_mb();

        std::fs::write(
            "voxcpm2_orig_bench.wav",
            create_wav(&samples, engine.sample_rate()),
        )?;

        results.push(BenchResult {
            name: "VoxCPM 2 (Orig)".to_string(),
            load_ms,
            ttft_ms,
            gen_ms,
            rtf,
            rss_mb,
        });
        println!("✅ VoxCPM 2 (Original) Synthesized.");
        drop(engine);
    }

    // ================= VoxCPM 2 (Metal Hybrid) ==================
    if Path::new(&vox_path).exists() {
        println!("⏳ [2/4] Loading VoxCPM 2 (Metal-Optimized Hybrid) Adapter...");
        let config = TtsConfig {
            engine: Some("voxcpm_metal".into()),
            device: Some("metal".to_string()),
            voxcpm: Some(VoxCpmConfig {
                model_path: vox_path.clone(),
                prompt_text: prompt.text.clone(),
                prompt_wav_path: prompt.wav_path.clone(),
                control_instruction: Some("自然新闻播报风格，语速中等".to_string()),
                min_len: None,
                max_len: Some(1024),
                inference_timesteps: Some(8),
                cfg_value: Some(1.8),
                retry_badcase: None,
                retry_badcase_ratio_threshold: None,
            }),
            qwen3: None,
            magictts: None,
            moss: None,
        };
        let i_load = Instant::now();
        let mut engine = EngineFactory::create(&config)?;
        let load_ms = i_load.elapsed().as_millis();

        // 预热 Metal 着色器内核
        println!("🔥 Warming up Metal shaders for VoxCPM 2 (Metal)...");
        let _ = engine.synthesize_chunk("啊")?;
        // 长句物理预热填充 Page Cache
        println!("🔥 Running long text warmup to fill Page Cache...");
        let _ = engine.synthesize_long_text(test_text).await?;

        println!("🎙️ Synthesizing with VoxCPM 2 (Metal Hybrid)...");
        // Measure TTFT
        let chunks = tts::audio::chunk_text(test_text);
        let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("你好");
        let i_first = Instant::now();
        let _ = engine.synthesize_chunk(first_chunk)?;
        let ttft_ms = i_first.elapsed().as_millis();

        // Total Generation
        let i_gen = Instant::now();
        let samples = engine.synthesize_long_text(test_text).await?;
        let gen_ms = i_gen.elapsed().as_millis();
        let rtf = (gen_ms as f64 / 1000.0) / (samples.len() as f64 / engine.sample_rate() as f64);
        let rss_mb = get_rss_mb();

        std::fs::write(
            "voxcpm2_metal_bench.wav",
            create_wav(&samples, engine.sample_rate()),
        )?;

        results.push(BenchResult {
            name: "VoxCPM 2 (Metal)".to_string(),
            load_ms,
            ttft_ms,
            gen_ms,
            rtf,
            rss_mb,
        });
        println!("✅ VoxCPM 2 (Metal Hybrid) Synthesized.");
        drop(engine);
    }

    // ================= Qwen3-TTS ==================
    let qwen_path = format!(
        "{}/.omlx/models/Qwen3-TTS-12Hz-1.7B-Base",
        dirs::home_dir().unwrap().to_string_lossy()
    );
    if Path::new(&qwen_path).exists() {
        // ---- 1. Qwen3-TTS (Original Direct Read) ----
        println!("⏳ [3/5] Loading Qwen3 (Original Direct Read)...");
        let device = candle_core::Device::new_metal(0).unwrap_or(candle_core::Device::Cpu);
        let i_load = Instant::now();

        // 物理模拟普通全读取 I/O 行为：全量读入权重和配置文件
        let _w_bytes = std::fs::read(Path::new(&qwen_path).join("model.safetensors"))?;
        let _st_bytes =
            std::fs::read(Path::new(&qwen_path).join("speech_tokenizer/model.safetensors"))?;
        let _cfg_bytes = std::fs::read(Path::new(&qwen_path).join("config.json"))?;

        let qwen_config = Qwen3Config {
            model_dir: qwen_path.clone(),
            prompt_text: prompt.text.clone(),
            prompt_wav_path: prompt.wav_path.clone(),
            language: Some("Chinese".to_string()),
            speaker: None,
            voice_design_instruction: None,
            max_length: Some(1024),
            temperature: Some(0.85),
            top_k: Some(50),
            top_p: Some(0.9),
            repetition_penalty: Some(1.05),
            seed: None,
            chunk_frames: None,
        };

        let qwen_model = qwen3_tts::Qwen3TTS::from_pretrained(&qwen_path, device.clone())?;
        // 核心优化：加载完毕后立刻 drop 掉 5.2GB 的无用内存 buffer，避免 Swap
        drop(_w_bytes);
        drop(_st_bytes);
        drop(_cfg_bytes);

        let mut engine = tts::adapters::Qwen3Adapter::new_from_model(qwen_model, &qwen_config);

        engine.cache_voice_prompt(&VoicePrompt {
            text: qwen_config.prompt_text.clone(),
            wav_path: qwen_config.prompt_wav_path.clone(),
        })?;
        let load_ms = i_load.elapsed().as_millis();

        // 预热 Metal 着色器内核
        println!("🔥 Warming up Metal shaders for Qwen3-TTS (Original)...");
        let _ = engine.synthesize_chunk("啊")?;
        // 长句物理预热填充 Page Cache
        println!("🔥 Running long text warmup to fill Page Cache...");
        let _ = engine.synthesize_long_text(test_text).await?;

        println!("🎙️ Synthesizing with Qwen3-TTS (Original)...");
        let chunks = tts::audio::chunk_text(test_text);
        let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("你好");
        let i_first = Instant::now();
        let _ = engine.synthesize_chunk(first_chunk)?;
        let ttft_ms = i_first.elapsed().as_millis();

        let i_gen = Instant::now();
        let samples = engine.synthesize_long_text(test_text).await?;
        let gen_ms = i_gen.elapsed().as_millis();
        let rtf = (gen_ms as f64 / 1000.0) / (samples.len() as f64 / engine.sample_rate() as f64);
        let rss_mb = get_rss_mb();

        std::fs::write(
            "qwen3_orig_bench.wav",
            create_wav(&samples, engine.sample_rate()),
        )?;

        results.push(BenchResult {
            name: "Qwen3-TTS (Orig)".to_string(),
            load_ms,
            ttft_ms,
            gen_ms,
            rtf,
            rss_mb,
        });
        println!("✅ Qwen3-TTS (Original) Synthesized.");
        drop(engine);

        // ---- 2. Qwen3-TTS (Mmap Optimized) ----
        println!("⏳ [3/5 (Mmap)] Loading Qwen3 Adapter (Mmap Optimized)...");
        let config = TtsConfig {
            engine: Some("qwen3".into()),
            device: Some("metal".to_string()),
            voxcpm: None,
            qwen3: Some(Qwen3Config {
                model_dir: qwen_path.clone(),
                prompt_text: prompt.text.clone(),
                prompt_wav_path: prompt.wav_path.clone(),
                language: Some("Chinese".to_string()),
                speaker: None,
                voice_design_instruction: None,
                max_length: Some(1024),
                temperature: Some(0.85),
                top_k: Some(50),
                top_p: Some(0.9),
                repetition_penalty: Some(1.05),
                seed: None,
                chunk_frames: None,
            }),
            magictts: None,
            moss: None,
        };
        let i_load = Instant::now();
        let mut engine = EngineFactory::create(&config)?;
        let load_ms = i_load.elapsed().as_millis();

        // 预热 Metal 着色器内核
        println!("🔥 Warming up Metal shaders for Qwen3-TTS (Mmap)...");
        let _ = engine.synthesize_chunk("啊")?;
        // 长句物理预热填充 Page Cache
        println!("🔥 Running long text warmup to fill Page Cache...");
        let _ = engine.synthesize_long_text(test_text).await?;

        println!("🎙️ Synthesizing with Qwen3-TTS (Mmap)...");
        let chunks = tts::audio::chunk_text(test_text);
        let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("你好");
        let i_first = Instant::now();
        let _ = engine.synthesize_chunk(first_chunk)?;
        let ttft_ms = i_first.elapsed().as_millis();

        let i_gen = Instant::now();
        let samples = engine.synthesize_long_text(test_text).await?;
        let gen_ms = i_gen.elapsed().as_millis();
        let rtf = (gen_ms as f64 / 1000.0) / (samples.len() as f64 / engine.sample_rate() as f64);
        let rss_mb = get_rss_mb();

        std::fs::write(
            "qwen3_mmap_bench.wav",
            create_wav(&samples, engine.sample_rate()),
        )?;

        results.push(BenchResult {
            name: "Qwen3-TTS (Mmap)".to_string(),
            load_ms,
            ttft_ms,
            gen_ms,
            rtf,
            rss_mb,
        });
        println!("✅ Qwen3-TTS (Mmap) Synthesized.");
        drop(engine);

        // ---- 3. Qwen3-TTS (Streaming CF5) ----
        println!("⏳ [3/5 (Stream CF5)] Loading Qwen3 Adapter (Streaming GPU)...");
        let mut stream_config_cf5 = config.clone();
        if let Some(ref mut qwen) = stream_config_cf5.qwen3 {
            qwen.chunk_frames = Some(5);
        }
        let i_load = Instant::now();
        let mut engine = EngineFactory::create(&stream_config_cf5)?;
        let load_ms = i_load.elapsed().as_millis();

        // 预热 Metal 着色器内核
        println!("🔥 Warming up Metal shaders for Qwen3-TTS (Streaming CF5)...");
        let _ = engine.synthesize_chunk("啊")?;
        // 长句物理预热填充 Page Cache
        println!("🔥 Running long text warmup to fill Page Cache...");
        let _ = engine.synthesize_long_text(test_text).await?;

        println!("🎙️ Synthesizing with Qwen3-TTS (Streaming CF5)...");
        let chunks = tts::audio::chunk_text(test_text);
        let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("你好");

        // 测量 TTFT
        let i_first = Instant::now();
        let mut ttft_ms = 0;
        let mut first_callback_called = false;

        let _ = engine.synthesize_streaming(first_chunk, &mut |_samples| {
            if !first_callback_called {
                ttft_ms = i_first.elapsed().as_millis();
                first_callback_called = true;
            }
            Ok(())
        })?;
        if !first_callback_called {
            ttft_ms = i_first.elapsed().as_millis();
        }

        // 测量完整生成时间
        let i_gen = Instant::now();
        let mut all_samples = Vec::new();
        for chunk in &chunks {
            if chunk.trim().is_empty() {
                continue;
            }
            let mut chunk_samples = Vec::new();
            let _ = engine.synthesize_streaming(chunk, &mut |samples| {
                chunk_samples.extend_from_slice(&samples);
                Ok(())
            })?;
            tts::audio::append_with_crossfade(
                &mut all_samples,
                &chunk_samples,
                engine.sample_rate(),
                0.05,
            );
        }
        let gen_ms = i_gen.elapsed().as_millis();
        let rtf =
            (gen_ms as f64 / 1000.0) / (all_samples.len() as f64 / engine.sample_rate() as f64);
        let rss_mb = get_rss_mb();

        std::fs::write(
            "qwen3_stream_cf5_bench.wav",
            create_wav(&all_samples, engine.sample_rate()),
        )?;

        results.push(BenchResult {
            name: "Qwen3-TTS (Str-CF5)".to_string(),
            load_ms,
            ttft_ms,
            gen_ms,
            rtf,
            rss_mb,
        });
        println!("✅ Qwen3-TTS (Streaming CF5) Synthesized.");
        drop(engine);

        // ---- 4. Qwen3-TTS (Streaming CF3) ----
        println!("⏳ [3/5 (Stream CF3)] Loading Qwen3 Adapter (Streaming GPU)...");
        let mut stream_config_cf3 = config.clone();
        if let Some(ref mut qwen) = stream_config_cf3.qwen3 {
            qwen.chunk_frames = Some(3);
        }
        let i_load = Instant::now();
        let mut engine = EngineFactory::create(&stream_config_cf3)?;
        let load_ms = i_load.elapsed().as_millis();

        // 预热 Metal 着色器内核
        println!("🔥 Warming up Metal shaders for Qwen3-TTS (Streaming CF3)...");
        let _ = engine.synthesize_chunk("啊")?;
        // 长句物理预热填充 Page Cache
        println!("🔥 Running long text warmup to fill Page Cache...");
        let _ = engine.synthesize_long_text(test_text).await?;

        println!("🎙️ Synthesizing with Qwen3-TTS (Streaming CF3)...");
        let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("你好");

        // 测量 TTFT
        let i_first = Instant::now();
        let mut ttft_ms = 0;
        let mut first_callback_called = false;

        let _ = engine.synthesize_streaming(first_chunk, &mut |_samples| {
            if !first_callback_called {
                ttft_ms = i_first.elapsed().as_millis();
                first_callback_called = true;
            }
            Ok(())
        })?;
        if !first_callback_called {
            ttft_ms = i_first.elapsed().as_millis();
        }

        // 测量完整生成时间
        let i_gen = Instant::now();
        let mut all_samples = Vec::new();
        for chunk in &chunks {
            if chunk.trim().is_empty() {
                continue;
            }
            let mut chunk_samples = Vec::new();
            let _ = engine.synthesize_streaming(chunk, &mut |samples| {
                chunk_samples.extend_from_slice(&samples);
                Ok(())
            })?;
            tts::audio::append_with_crossfade(
                &mut all_samples,
                &chunk_samples,
                engine.sample_rate(),
                0.05,
            );
        }
        let gen_ms = i_gen.elapsed().as_millis();
        let rtf =
            (gen_ms as f64 / 1000.0) / (all_samples.len() as f64 / engine.sample_rate() as f64);
        let rss_mb = get_rss_mb();

        std::fs::write(
            "qwen3_stream_cf3_bench.wav",
            create_wav(&all_samples, engine.sample_rate()),
        )?;

        results.push(BenchResult {
            name: "Qwen3-TTS (Str-CF3)".to_string(),
            load_ms,
            ttft_ms,
            gen_ms,
            rtf,
            rss_mb,
        });
        println!("✅ Qwen3-TTS (Streaming CF3) Synthesized.");
        drop(engine);
    }

    // ================= Magic-TTS ==================
    let magic_path = dirs::home_dir()
        .unwrap()
        .join(".aha/SCUT/MAGIC-TTS")
        .to_string_lossy()
        .to_string();
    if Path::new(&magic_path).exists() {
        let run_magic_bench = async {
            // ---- 1. Magic-TTS (Original / Standard synthesis) ----
            println!("⏳ [4/5 (Orig)] Loading Magic-TTS (Orig Standard) Adapter...");
            let config = TtsConfig {
                engine: Some("magictts".into()),
                device: Some("metal".to_string()),
                voxcpm: None,
                qwen3: None,
                magictts: Some(MagicTtsConfig {
                    model_dir: magic_path.clone(),
                    prompt_text: None,
                    prompt_wav_path: None,
                    vocab_path: None,
                    steps: Some(16),
                    cfg_strength: Some(2.0),
                    default_content_ms: None,
                    default_punct_ms: None,
                }),
                moss: None,
            };
            let i_load = Instant::now();
            let mut engine = EngineFactory::create(&config)?;
            let load_ms = i_load.elapsed().as_millis();

            // 预热 Metal 着色器内核
            println!("🔥 Warming up Metal shaders for Magic-TTS (Orig)...");
            let _ = engine.synthesize_chunk("啊")?;
            // 长句物理预热填充 Page Cache
            println!("🔥 Running long text warmup to fill Page Cache...");
            let _ = engine.synthesize_long_text(test_text).await?;

            println!("🎙️ Synthesizing with Magic-TTS (Orig Standard)...");
            let chunks = tts::audio::chunk_text(test_text);
            let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("你好");
            let i_first = Instant::now();
            let _ = engine.synthesize_chunk(first_chunk)?;
            let ttft_ms = i_first.elapsed().as_millis();

            let i_gen = Instant::now();
            let samples = engine.synthesize_long_text(test_text).await?;
            let gen_ms = i_gen.elapsed().as_millis();
            let rtf =
                (gen_ms as f64 / 1000.0) / (samples.len() as f64 / engine.sample_rate() as f64);
            let rss_mb = get_rss_mb();

            std::fs::write(
                "magictts_orig_bench.wav",
                create_wav(&samples, engine.sample_rate()),
            )?;

            results.push(BenchResult {
                name: "Magic-TTS (Orig)".to_string(),
                load_ms,
                ttft_ms,
                gen_ms,
                rtf,
                rss_mb,
            });
            println!("✅ Magic-TTS (Orig Standard) Synthesized.");
            drop(engine);

            // ---- 2. Magic-TTS (Clone / Zero-shot cloning) ----
            println!("⏳ [4/5 (Clone)] Loading Magic-TTS (Clone Zero-shot) Adapter...");
            let config = TtsConfig {
                engine: Some("magictts".into()),
                device: Some("metal".to_string()),
                voxcpm: None,
                qwen3: None,
                magictts: Some(MagicTtsConfig {
                    model_dir: magic_path.clone(),
                    prompt_text: prompt.text.clone(),
                    prompt_wav_path: prompt.wav_path.clone(),
                    vocab_path: None,
                    steps: Some(16),
                    cfg_strength: Some(2.0),
                    default_content_ms: None,
                    default_punct_ms: None,
                }),
                moss: None,
            };
            let i_load = Instant::now();
            let mut engine = EngineFactory::create(&config)?;
            let load_ms = i_load.elapsed().as_millis();

            // 预热 Metal 着色器内核
            println!("🔥 Warming up Metal shaders for Magic-TTS (Clone)...");
            let _ = engine.synthesize_chunk("啊")?;
            // 长句物理预热填充 Page Cache
            println!("🔥 Running long text warmup to fill Page Cache...");
            let _ = engine.synthesize_long_text(test_text).await?;

            println!("🎙️ Synthesizing with Magic-TTS (Clone)...");
            let chunks = tts::audio::chunk_text(test_text);
            let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("你好");
            let i_first = Instant::now();
            let _ = engine.synthesize_chunk(first_chunk)?;
            let ttft_ms = i_first.elapsed().as_millis();

            let i_gen = Instant::now();
            let samples = engine.synthesize_long_text(test_text).await?;
            let gen_ms = i_gen.elapsed().as_millis();
            let rtf =
                (gen_ms as f64 / 1000.0) / (samples.len() as f64 / engine.sample_rate() as f64);
            let rss_mb = get_rss_mb();

            std::fs::write(
                "magictts_clone_bench.wav",
                create_wav(&samples, engine.sample_rate()),
            )?;

            results.push(BenchResult {
                name: "Magic-TTS (Clone)".to_string(),
                load_ms,
                ttft_ms,
                gen_ms,
                rtf,
                rss_mb,
            });
            println!("✅ Magic-TTS (Clone) Synthesized.");
            drop(engine);
            Ok::<(), anyhow::Error>(())
        };

        if let Err(e) = run_magic_bench.await {
            println!("⚠️ Skipping Magic-TTS benchmark due to load error: {}", e);
        }
    }

    // ================= BENCHMARK REPORT ==================
    println!("\n=================================== BENCHMARK REPORT ===================================");
    println!(
        "{:<20} | {:<12} | {:<12} | {:<12} | {:<8} | {:<12}",
        "Engine Name", "Load Time", "TTFT", "Gen Time", "RTF", "Memory (RSS)"
    );
    println!(
        "{:<20} | {:<12} | {:<12} | {:<12} | {:<8} | {:<12}",
        "--------------------",
        "------------",
        "------------",
        "------------",
        "--------",
        "------------"
    );
    for res in results {
        println!(
            "{:<20} | {:>8} ms  | {:>8} ms  | {:>8} ms  | {:>6.3}   | {:>8.1} MB",
            res.name, res.load_ms, res.ttft_ms, res.gen_ms, res.rtf, res.rss_mb
        );
    }
    println!("========================================================================================\n");

    println!("🎉 All benchmarks finished successfully!");
    Ok(())
}
