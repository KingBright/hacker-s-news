use anyhow::Result;
use std::time::Instant;
use std::path::Path;
use tts::{EngineFactory, TtsConfig, VoxCpmConfig, Qwen3Config, VoicePrompt};

fn create_wav(samples: &[f32], rate: usize) -> Vec<u8> {
    use std::io::Cursor;
    let mut cursor = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: 1, sample_rate: rate as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
    for &sample in samples {
        let s = (sample.max(-1.0).min(1.0) * 32767.0) as i16;
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();
    cursor.into_inner()
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    
    println!("🚀 Starting TTS Adapter Benchmark!\n");

    let test_text = "欢迎收听今天的黑客新闻。最近科技圈可谓是风起云涌。首先我们来看看人工智能领域的最新进展。就在昨天，某知名AI实验室发布了他们最新的超大参数量多模态模型。这款模型不仅能够处理复杂的逻辑推理，还能完美理解极其深奥的长文本上下文。据测试，它在许多权威榜单上已经刷新了历史纪录。更加令人震惊的是，它的开源版本甚至能够在普通的消费级显卡上流畅运行，这无疑大大降低了开发者入门人工智能的门槛，掀起了一场小参数模型的狂欢。与此同时，在遥远的硅谷，另一家芯片巨头也抛出了重磅炸弹。他们宣布成功研制出基于三纳米工艺的全新架构芯片，不仅功耗降低了40%，核心算力更是提升了两倍之多。分析人士指出，这款芯片的发售将彻底改变目前云服务器的市场格局，尤其是对那些强依赖图形渲染和并行计算的元宇宙公司而言，简直是一剂强心针。不仅如此，多家新能源车企也纷纷表态，希望能够在这款芯片量产的首日抢占份额，用于他们新一代的自动驾驶计算平台。最后，让我们把目光投向开源社区。随着Rust语言的普及，越来越多的底层系统正在被重写。不仅是Linux内核的主线越来越拥抱Rust，甚至连很多关键的网络驱动和密码学库也相继推出了Rust替代方案。今天，一款号称完全由纯Rust编写的高性能TTS引擎引发了社区的极大关注。它主打安全、极速和内存隔离，据说不仅彻底剥离了沉重的历史包袱，还能做到极其平滑的音频合成体验。以上就是本期的简讯，感谢您的收听，我们下期再见！";
    let audio_ref = "../aha/assets/audio/voice_01.wav";
    let prompt_text = "华为致力于把数字世界带给每个人，每个家庭，每个组织，构建万物互联的智能世界。";

    println!("========== CONFIGURATION ==========");
    println!("Prompt: {}", test_text);
    println!("Reference Audio: {}", audio_ref);
    println!("Reference Text: {}", prompt_text);
    println!("===================================\n");

    let prompt = VoicePrompt {
        text: Some(prompt_text.to_string()),
        wav_path: Some(format!("file://{}", audio_ref)),
    };

    // ================= VoxCPM 2 ==================
    let vox_path = dirs::home_dir().unwrap().join(".aha/OpenBMB/VoxCPM2").to_string_lossy().to_string();
    if Path::new(&vox_path).exists() {
        println!("⏳ Loading VoxCPM 2 Adapter...");
        let config = TtsConfig {
            engine: Some("voxcpm".into()),
            voxcpm: Some(VoxCpmConfig {
                model_path: vox_path.clone(),
                prompt_text: prompt.text.clone(),
                prompt_wav_path: prompt.wav_path.clone(),
            }),
            qwen3: None,
        };
        let i_load = Instant::now();
        let mut engine = EngineFactory::create(&config)?;
        let load_ms = i_load.elapsed().as_millis();
        
        println!("🎙️ Generating VoxCPM 2 Audio ...");
        let i_gen = Instant::now();
        let samples = engine.synthesize_long_text(test_text).await?;
        let gen_ms = i_gen.elapsed().as_millis();
        
        std::fs::write("voxcpm2_bench.wav", create_wav(&samples, engine.sample_rate()))?;
        println!("✅ VoxCPM 2 - Load: {} ms | Generate: {} ms", load_ms, gen_ms);
    }

    println!();
    
    // ================= Qwen3-TTS ==================
    let qwen_path = dirs::home_dir().unwrap().join(".aha/Qwen/Qwen3-TTS").to_string_lossy().to_string();
    if Path::new(&qwen_path).exists() {
        println!("⏳ Loading Qwen3 Adapter...");
        let config = TtsConfig {
            engine: Some("qwen3".into()),
            voxcpm: None,
            qwen3: Some(Qwen3Config {
                model_dir: qwen_path.clone(),
                prompt_text: prompt.text.clone(),
                prompt_wav_path: prompt.wav_path.clone(),
            }),
        };
        let i_load = Instant::now();
        let mut engine = EngineFactory::create(&config)?;
        let load_ms = i_load.elapsed().as_millis();
        
        println!("🎙️ Generating Qwen3 Audio ...");
        let i_gen = Instant::now();
        let samples = engine.synthesize_long_text(test_text).await?;
        let gen_ms = i_gen.elapsed().as_millis();
        
        std::fs::write("qwen3_bench.wav", create_wav(&samples, engine.sample_rate()))?;
        println!("✅ Qwen3-TTS - Load: {} ms | Generate: {} ms", load_ms, gen_ms);
    }
    
    Ok(())
}
