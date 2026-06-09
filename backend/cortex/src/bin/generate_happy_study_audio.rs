use anyhow::Result;
use std::path::Path;
use std::time::Instant;
use tts::{EngineFactory, TtsConfig, VoicePrompt, VoxCpmConfig};

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

struct Task {
    filename: &'static str,
    text: &'static str,
}

async fn run_speaker_tasks(
    speaker_file: &str,
    tasks: Vec<Task>,
    audio_dir: &Path,
    vox_path: &str,
    prompt_text: &str,
) -> Result<()> {
    let voice_path = audio_dir.join(speaker_file);
    if !voice_path.exists() {
        return Err(anyhow::anyhow!(
            "Speaker voice file not found: {:?}",
            voice_path
        ));
    }

    println!(
        "⏳ Loading VoxCPM 2 Engine for speaker: {} ...",
        speaker_file
    );
    let prompt = VoicePrompt {
        text: Some(prompt_text.to_string()),
        wav_path: Some(format!("file://{}", voice_path.to_string_lossy())),
    };

    let config = TtsConfig {
        engine: Some("voxcpm".into()),
        device: None,
        voxcpm: Some(VoxCpmConfig {
            model_path: vox_path.to_string(),
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

    let mut engine = EngineFactory::create(&config)?;

    for task in tasks {
        println!(
            "🎙️ Generating {} using speaker {} ...",
            task.filename, speaker_file
        );
        let i_gen = Instant::now();
        let samples = engine.synthesize_long_text(task.text).await?;
        let gen_ms = i_gen.elapsed().as_millis();

        let out_path = std::env::current_dir()?.join(task.filename);
        std::fs::write(&out_path, create_wav(&samples, engine.sample_rate()))?;
        println!(
            "✅ Generated {} in {} ms -> {:?}",
            task.filename, gen_ms, out_path
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🚀 Starting Happy Study Audio Pre-Generation!");

    // Paths configuration
    let vox_path = dirs::home_dir()
        .unwrap()
        .join(".aha/OpenBMB/VoxCPM2")
        .to_string_lossy()
        .to_string();

    if !Path::new(&vox_path).exists() {
        anyhow::bail!("VoxCPM2 model path does not exist at {}", vox_path);
    }

    let audio_dir = dirs::home_dir()
        .unwrap()
        .join("workspace/hacker-s-news/audio");

    // Check if prompt audio directory exists
    if !audio_dir.exists() {
        anyhow::bail!("Audio prompts directory does not exist at {:?}", audio_dir);
    }

    let prompt_text = "Hello 大家好，这里是 Fresh Loop 新闻速度，我们将给你带来最新鲜的新闻资讯。";

    // 1. 小凡 (xiaofan.wav) - 数学老师
    let xiaofan_tasks = vec![
        Task {
            filename: "math_fraction_mult_1.wav",
            text: "小朋友们好！今天小强老师带大家去蛋糕店。如果有半个蛋糕，三个人来分，每个人都要拿走半个，大家一共吃掉多少蛋糕呢？那就是二分之一乘以三，也就是三个二分之一相加，最终等于二分之三，也就是一个半蛋糕啦！",
        },
        Task {
            filename: "math_fraction_mult_2.wav",
            text: "那如果是两个分数相乘呢？比如三分之二乘以五分之四。其实非常简单，我们只需要把分子和分子乘起来作为新分子，分母和分母乘起来作为新分母。所以二乘四得八，三乘五得十五，结果就是十五分之八！",
        },
        Task {
            filename: "math_dynamic_triangle_1.wav",
            text: "同学们好！我们一起来看这个三角形。我们可以拿出两个完全一模一样的三角形，把它们反向拼在一起。瞧！它们拼成了一个大大的平行四边形。平行四边形的面积公式是底乘以高，所以单个三角形的面积，就是底乘以高再除以二啦！",
        },
        Task {
            filename: "math_dynamic_triangle_2.wav",
            text: "下面我们来做个小练习。如果有一个三角形，它的底是八厘米，高是五厘米，那么它的面积该怎么算呢？我们把数代入公式，那就是底乘高，八乘五等于四十，然后再除以二，最终得到二十平方厘米。是不是非常神奇又简单呢？",
        },
    ];

    // 2. 小何 (xiaohe.wav) - 科学老师
    let xiaohe_tasks = vec![
        Task {
            filename: "science_water_cycle_1.wav",
            text: "同学们，大海里的一只小水滴今天想去天空中冒险。早晨，暖洋洋的太阳公公升起来了，给大海加热。小水滴吸收了热量，身体变得轻飘飘的，变成了看不见的水蒸气，快乐地飞向了高空！这就是蒸发。",
        },
        Task {
            filename: "science_water_cycle_2.wav",
            text: "飞到高空的小水滴觉得越来越冷了。这时候它遇到了一群冷空气伙伴。于是，水蒸气重新抱成团，凝结成了千千万万颗小水珠，连在一起就变成了天上软绵绵的白云。当水珠太重时，它们就会变成雨滴落回地面！这就是降雨。",
        },
    ];

    // 3. 小萌 (meng.wav) - 语文老师
    let meng_tasks = vec![
        Task {
            filename: "language_jing_ye_si_1.wav",
            text: "同学们，让我们穿越回唐代的一天夜里。大诗人李白睡不着觉，坐在床前看着窗外。窗前洒下了一片冷清清的月光，亮晶晶的，李白眯起眼睛看，还以为是地上结了一层亮闪闪的秋霜呢！",
        },
        Task {
            filename: "language_jing_ye_si_2.wav",
            text: "他忍不住抬起头，看着夜空中那轮孤零零的圆月，想起了自己远方的家乡和家里的亲人们。接着他低下头，深深地叹了一口气，心里充满了无限的思念。这就是最经典的书写旅人思乡的诗句。",
        },
    ];

    // Run all
    run_speaker_tasks(
        "xiaofan.wav",
        xiaofan_tasks,
        &audio_dir,
        &vox_path,
        prompt_text,
    )
    .await?;
    run_speaker_tasks(
        "xiaohe.wav",
        xiaohe_tasks,
        &audio_dir,
        &vox_path,
        prompt_text,
    )
    .await?;
    run_speaker_tasks("meng.wav", meng_tasks, &audio_dir, &vox_path, prompt_text).await?;

    println!("🎉 All audio files pre-generated successfully!");
    Ok(())
}
