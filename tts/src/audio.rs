use anyhow::Result;

pub fn chunk_text(raw_text: &str) -> Vec<String> {
    // Pre-TTS Normalization
    let mut text = raw_text
        .replace("%", "百分之")
        .replace("℃", "度")
        .replace("$", "美元")
        .replace("**", "")
        .replace("##", "")
        .replace("  ", " ");

    if let Ok(re) = regex::Regex::new(r"（.*?）|\(.*?\)|【.*?】") {
        text = re.replace_all(&text, "").into_owned();
    }

    text = text
        .replace("本条播放完毕", "")
        .replace("本条新闻播报结束", "")
        .replace("谢谢收听", "")
        .replace("报道结束", "");

    let mut chunks = Vec::new();
    let mut current_chunk = String::new();

    let terminators = ['。', '！', '？'];
    let secondary_terminators = ['，', '；', '：', '、'];

    for char in text.chars() {
        current_chunk.push(char);
        let len = current_chunk.chars().count();
        if char == '\n' {
            if !current_chunk.trim().is_empty() {
                chunks.push(current_chunk.clone());
                current_chunk.clear();
            }
            continue;
        }
        if terminators.contains(&char) && len > 80 {
            chunks.push(current_chunk.clone());
            current_chunk.clear();
            continue;
        }
        if len > 120 && secondary_terminators.contains(&char) {
            chunks.push(current_chunk.clone());
            current_chunk.clear();
            continue;
        }
        if len > 200 {
            chunks.push(current_chunk.clone());
            current_chunk.clear();
        }
    }

    if !current_chunk.trim().is_empty() {
        chunks.push(current_chunk);
    }
    chunks
}

pub fn append_with_crossfade(
    all_samples: &mut Vec<f32>,
    new_samples: &[f32],
    sample_rate: usize,
    crossfade_duration: f64,
) {
    if all_samples.is_empty() {
        all_samples.extend(new_samples);
    } else {
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

pub async fn convert_to_mp3(wav_bytes: &[u8]) -> Result<Vec<u8>> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let mut child = Command::new("ffmpeg")
        .args(&[
            "-f", "wav", "-i", "pipe:0", "-f", "mp3", "-b:a", "128k", "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg: {}", e))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to open stdin"))?;
    let data = wav_bytes.to_vec();

    tokio::spawn(async move {
        if let Err(e) = stdin.write_all(&data).await {
            log::error!("Failed to write to ffmpeg stdin: {}", e);
        }
    });

    let output = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("ffmpeg timeout 60s"))??;

    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(anyhow::anyhow!("ffmpeg failed"))
    }
}
