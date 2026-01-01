use anyhow::Result;
use cortex::core::config::load_config;
use cortex::core::tts::TtsClient;
use std::fs;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    log::info!("Starting TTS test...");

    // Load config
    // Assuming config.toml is in the workspace root
    let config_path = "../config.toml";
    if !Path::new(config_path).exists() {
        log::error!("Config file not found at {}", config_path);
        return Ok(());
    }

    let config = load_config(config_path)?;
    log::info!("Loaded config with engine: {:?}", config.tts.engine);

    if let Some(vox) = &config.tts.voxcpm {
         log::info!("VoxCPM model path: {}", vox.model_path);
    }

    // Initialize TTS Client
    let tts_client = TtsClient::new(config.tts.clone());

    // Test text
    let text = "你好，这是一个测试音频，用于验证 VoxCPM 集成是否成功。";
    log::info!("Generating audio for text: {}", text);

    match tts_client.speak(text).await {
        Ok(audio_data) => {
            log::info!("Audio generated successfully. Size: {} bytes", audio_data.len());
            let output_path = "test_output.wav";
            fs::write(output_path, audio_data)?;
            log::info!("Saved audio to {}", output_path);
        }
        Err(e) => {
            log::error!("Failed to generate audio: {:?}", e);
        }
    }

    Ok(())
}
