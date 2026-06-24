use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Copy)]
pub struct NormalizeOptions {
    pub preserve_leading_parenthetical: bool,
}

impl Default for NormalizeOptions {
    fn default() -> Self {
        Self {
            preserve_leading_parenthetical: true,
        }
    }
}

pub fn normalize_for_tts(raw_text: &str, options: NormalizeOptions) -> String {
    let mut text = raw_text.replace("\r\n", "\n").replace('\r', "\n");

    let leading_control = if options.preserve_leading_parenthetical {
        take_leading_parenthetical(&text)
    } else {
        None
    };
    if leading_control.is_some() {
        text = strip_equivalent_leading_parenthetical(&text);
    }

    text = text
        .replace("```", "")
        .replace("**", "")
        .replace('*', "")
        .replace('`', "")
        .replace("##", "")
        .replace('#', "");

    if let Ok(re) = regex::Regex::new(r"\[([^\]\n]+)\]\([^)]+\)") {
        text = re.replace_all(&text, "$1").into_owned();
    }
    if let Ok(re) = regex::Regex::new(r"https?://\S+|www\.\S+") {
        text = re.replace_all(&text, " 链接见原文 ").into_owned();
    }
    if let Ok(re) = regex::Regex::new(r"[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}") {
        text = re.replace_all(&text, " 邮箱地址 ").into_owned();
    }
    if let Ok(re) = regex::Regex::new(r"(?i)[（(]\s*(source|来源|via)[:：][^）)]*[）)]") {
        text = re.replace_all(&text, "").into_owned();
    }
    if let Ok(re) = regex::Regex::new(r"(?:US\$|\$)\s*([0-9]+(?:\.[0-9]+)?)") {
        text = re.replace_all(&text, "$1 美元").into_owned();
    }
    if let Ok(re) = regex::Regex::new(r"([0-9]+(?:\.[0-9]+)?)\s*%") {
        text = re.replace_all(&text, "百分之 $1").into_owned();
    }
    if let Ok(re) = regex::Regex::new(r"([0-9]+(?:\.[0-9]+)?)\s*(?:℃|°C)") {
        text = re.replace_all(&text, "$1 摄氏度").into_owned();
    }

    text = text
        .replace("C++", "C plus plus")
        .replace("c++", "C plus plus")
        .replace("C#", "C sharp")
        .replace("c#", "C sharp")
        .replace("->", " 指向 ")
        .replace("=>", " 推出 ")
        .replace(">=", " 大于等于 ")
        .replace("<=", " 小于等于 ")
        .replace("!=", " 不等于 ")
        .replace("==", " 等于 ")
        .replace('±', " 正负 ")
        .replace('√', " 根号 ")
        .replace('×', " 乘以 ")
        .replace('÷', " 除以 ")
        .replace('≈', " 约等于 ")
        .replace('℃', "摄氏度")
        .replace('%', "百分号")
        .replace('$', "美元")
        .replace('&', " 和 ");

    if let Ok(re) = regex::Regex::new(r"(?m)^\s*[-+*]\s+") {
        text = re.replace_all(&text, "").into_owned();
    }
    if let Ok(re) = regex::Regex::new(r"(?m)^\s*\d+[.)、]\s+") {
        text = re.replace_all(&text, "").into_owned();
    }

    text = text
        .replace('[', "")
        .replace(']', "")
        .replace('（', "，")
        .replace('）', "，")
        .replace('(', "，")
        .replace(')', "，");

    if let Some(control) = leading_control {
        let normalized_control = control
            .replace('（', "(")
            .replace('）', ")")
            .trim()
            .to_string();
        text = format!("{}{}", normalized_control, text.trim_start());
    }

    if let Ok(re) = regex::Regex::new(r"[ \t]{2,}") {
        text = re.replace_all(&text, " ").into_owned();
    }
    if let Ok(re) = regex::Regex::new(r"\n{3,}") {
        text = re.replace_all(&text, "\n\n").into_owned();
    }

    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn take_leading_parenthetical(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let (open, close) = if trimmed.starts_with('(') {
        ('(', ')')
    } else if trimmed.starts_with('（') {
        ('（', '）')
    } else {
        return None;
    };

    let mut chars = trimmed.char_indices();
    let (_, first) = chars.next()?;
    if first != open {
        return None;
    }
    for (idx, ch) in chars {
        if ch == close {
            let end = idx + ch.len_utf8();
            let candidate = &trimmed[..end];
            let char_count = candidate.chars().count();
            let inner = candidate
                .trim_matches(['(', ')', '（', '）'])
                .trim()
                .to_ascii_lowercase();
            if (3..=80).contains(&char_count)
                && !inner.starts_with("source:")
                && !inner.starts_with("source：")
                && !inner.starts_with("via:")
                && !inner.starts_with("via：")
                && !inner.starts_with("来源:")
                && !inner.starts_with("来源：")
            {
                return Some(candidate.to_string());
            }
            return None;
        }
        if ch == '\n' {
            return None;
        }
    }
    None
}

fn strip_equivalent_leading_parenthetical(text: &str) -> String {
    let trimmed = text.trim_start();
    let Some(control) = take_leading_parenthetical(trimmed) else {
        return text.to_string();
    };
    trimmed[control.len()..].trim_start().to_string()
}

pub fn chunk_text(raw_text: &str) -> Vec<String> {
    // Pre-TTS Normalization
    let text = normalize_for_tts(raw_text, NormalizeOptions::default())
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

pub struct PcmStreamEncoder {
    tail: Vec<f32>,
    crossfade_samples: usize,
    output_samples: usize,
}

impl PcmStreamEncoder {
    pub fn new(sample_rate: usize, crossfade_duration: f64) -> Self {
        Self {
            tail: Vec::new(),
            crossfade_samples: (sample_rate as f64 * crossfade_duration).round() as usize,
            output_samples: 0,
        }
    }

    pub fn push(&mut self, new_samples: &[f32]) -> Vec<u8> {
        if new_samples.is_empty() {
            return Vec::new();
        }

        let mut combined = if self.tail.is_empty() || self.crossfade_samples == 0 {
            new_samples.to_vec()
        } else {
            let overlap = self
                .crossfade_samples
                .min(self.tail.len())
                .min(new_samples.len());
            let mut mixed = Vec::with_capacity(self.tail.len() + new_samples.len() - overlap);

            if self.tail.len() > overlap {
                mixed.extend_from_slice(&self.tail[..self.tail.len() - overlap]);
            }

            for i in 0..overlap {
                let fade_out = 1.0 - (i as f32 / overlap as f32);
                let fade_in = i as f32 / overlap as f32;
                let old_val = self.tail[self.tail.len() - overlap + i];
                let new_val = new_samples[i];
                mixed.push(old_val * fade_out + new_val * fade_in);
            }

            if overlap < new_samples.len() {
                mixed.extend_from_slice(&new_samples[overlap..]);
            }

            mixed
        };

        self.tail.clear();
        if self.crossfade_samples > 0 && combined.len() > self.crossfade_samples {
            let split_at = combined.len() - self.crossfade_samples;
            self.tail.extend_from_slice(&combined[split_at..]);
            combined.truncate(split_at);
        }

        self.output_samples += combined.len();
        f32_samples_to_s16le_bytes(&combined)
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let tail = std::mem::take(&mut self.tail);
        self.output_samples += tail.len();
        f32_samples_to_s16le_bytes(&tail)
    }

    pub fn output_samples(&self) -> usize {
        self.output_samples
    }
}

pub fn f32_samples_to_s16le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let sample_i16 = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&sample_i16.to_le_bytes());
    }
    bytes
}

pub struct Mp3StreamProcess {
    child: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
    stdout_task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
}

impl Mp3StreamProcess {
    pub fn start_s16le(sample_rate: u32) -> Result<Self> {
        let sample_rate_arg = sample_rate.to_string();
        let mut child = tokio::process::Command::new("ffmpeg")
            .args([
                "-f",
                "s16le",
                "-ar",
                sample_rate_arg.as_str(),
                "-ac",
                "1",
                "-i",
                "pipe:0",
                "-f",
                "mp3",
                "-b:a",
                "128k",
                "pipe:1",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg: {}", e))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open ffmpeg stdin"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open ffmpeg stdout"))?;

        let stdout_task = tokio::spawn(async move {
            let mut output = Vec::new();
            stdout.read_to_end(&mut output).await?;
            Ok(output)
        });

        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout_task,
        })
    }

    pub async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        if let Some(stdin) = self.stdin.as_mut() {
            stdin.write_all(bytes).await?;
        }
        Ok(())
    }

    pub async fn finish(mut self) -> Result<Vec<u8>> {
        drop(self.stdin.take());

        let status = self.child.wait().await?;
        let mp3_bytes = self
            .stdout_task
            .await
            .map_err(|e| anyhow::anyhow!("ffmpeg stdout task failed: {}", e))??;

        if !status.success() {
            anyhow::bail!("ffmpeg failed while encoding PCM stream");
        }

        Ok(mp3_bytes)
    }
}

pub async fn convert_to_mp3(wav_bytes: &[u8]) -> Result<Vec<u8>> {
    use std::process::Stdio;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_symbols_for_speech() {
        let text = "C++ hit 40% at $19.99, temp 23℃, x >= 3 -> done.";
        let normalized = normalize_for_tts(text, NormalizeOptions::default());

        assert!(normalized.contains("C plus plus"));
        assert!(normalized.contains("百分之 40"));
        assert!(normalized.contains("19.99 美元"));
        assert!(normalized.contains("23 摄氏度"));
        assert!(normalized.contains("大于等于"));
        assert!(normalized.contains("指向"));
    }

    #[test]
    fn keeps_markdown_link_text_and_removes_url_noise() {
        let text = "Read [the paper](https://example.com/paper) and https://example.com/raw";
        let normalized = normalize_for_tts(text, NormalizeOptions::default());

        assert!(normalized.contains("the paper"));
        assert!(!normalized.contains("https://example.com/paper"));
        assert!(normalized.contains("链接见原文"));
    }

    #[test]
    fn preserves_leading_voxcpm_control_instruction() {
        let text = "（沉稳、稍快）## Title\n- **Point** (source: feed)";
        let normalized = normalize_for_tts(text, NormalizeOptions::default());

        assert!(normalized.starts_with("(沉稳、稍快)"));
        assert!(normalized.contains("Point"));
        assert!(!normalized.contains("source:"));
    }

    #[test]
    fn does_not_treat_leading_source_citation_as_control() {
        let text = "(source: feed) Important update.";
        let normalized = normalize_for_tts(text, NormalizeOptions::default());

        assert_eq!(normalized, "Important update.");
    }

    #[test]
    fn streams_pcm_with_delayed_crossfade_tail() {
        let mut encoder = PcmStreamEncoder::new(10, 0.2);

        let first = encoder.push(&[0.0, 0.1, 0.2, 0.3]);
        assert_eq!(first.len(), 4);
        assert_eq!(encoder.output_samples(), 2);

        let second = encoder.push(&[0.4, 0.5, 0.6, 0.7]);
        assert_eq!(second.len(), 4);
        assert_eq!(encoder.output_samples(), 4);

        let tail = encoder.finish();
        assert_eq!(tail.len(), 4);
        assert_eq!(encoder.output_samples(), 6);
    }

    #[test]
    fn converts_f32_samples_to_little_endian_pcm16() {
        let bytes = f32_samples_to_s16le_bytes(&[-2.0, 0.0, 2.0]);
        let samples = bytes
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();

        assert_eq!(samples, vec![-32767, 0, 32767]);
    }

    #[tokio::test]
    async fn mp3_stream_process_encodes_pcm_when_ffmpeg_is_available() -> Result<()> {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            return Ok(());
        }

        let mut process = Mp3StreamProcess::start_s16le(24_000)?;
        let silence_pcm = vec![0_u8; 24_000 / 10 * 2];
        process.write_all(&silence_pcm).await?;
        let mp3 = process.finish().await?;

        assert!(mp3.len() > 100);
        assert!(
            mp3.starts_with(b"ID3")
                || mp3.first().copied() == Some(0xff)
                || mp3
                    .windows(2)
                    .any(|bytes| bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
        );

        Ok(())
    }
}
