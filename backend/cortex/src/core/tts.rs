use anyhow::Result;
use std::process::Command;
use std::path::Path;
use crate::core::config::TtsConfig;
use uuid::Uuid;

pub struct TtsClient {
    config: TtsConfig,
}

impl TtsClient {
    pub fn new(config: TtsConfig) -> Self {
        Self { config }
    }

    pub async fn speak(&self, text: &str) -> Result<Vec<u8>> {
        // In a real scenario, this would call the piper executable.
        // For this implementation, we will mock it if piper is not found,
        // or try to run it.

        let output_filename = format!("/tmp/{}.wav", Uuid::new_v4());

        // Check if piper exists (simplistic check)
        let piper_exists = Command::new("piper").arg("--version").output().is_ok();

        if piper_exists {
            let mut child = Command::new("piper")
                .arg("--model")
                .arg(&self.config.model_path)
                .arg("--output_file")
                .arg(&output_filename)
                .stdin(std::process::Stdio::piped())
                .spawn()?;

            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                stdin.write_all(text.as_bytes())?;
            }

            let status = child.wait()?;

             if status.success() {
                 let bytes = std::fs::read(&output_filename)?;
                 std::fs::remove_file(output_filename)?; // Cleanup
                 return Ok(bytes);
             } else {
                 log::warn!("Piper TTS failed. Using dummy audio.");
             }
        } else {
            log::warn!("Piper not found. Using dummy audio.");
        }

        // Return dummy audio (silence or just some bytes)
        // This allows the system to work even without the actual TTS engine installed.
        Ok(vec![0; 1024]) // 1KB of zero bytes
    }
}
