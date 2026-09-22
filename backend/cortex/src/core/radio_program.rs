use super::{config::Host, tts::TtsClient};
use anyhow::{bail, Context, Result};
use futures::{stream, FutureExt, StreamExt, TryStreamExt};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Deserialize)]
struct Plan {
    segments: Vec<Segment>,
}
#[derive(Deserialize)]
struct Segment {
    category: Option<String>,
    text: String,
}

fn host_for<'a>(hosts: &'a [Host], category: Option<&str>) -> Result<&'a Host> {
    match category {
        Some(category) => hosts
            .iter()
            .find(|h| h.categories.iter().any(|c| c == category))
            .with_context(|| format!("no configured host for {category}")),
        None => hosts.first().context("program requires a lead host"),
    }
}

pub async fn render(
    text: &str,
    hosts: &[Host],
    tts: Arc<TtsClient>,
    cache: &Path,
    fingerprint: &str,
    concurrency: usize,
) -> Result<(Vec<u8>, i64)> {
    let plan: Plan = serde_json::from_str(text).context("decode program plan")?;
    if plan.segments.len() < 3 || plan.segments.len() > 32 {
        bail!("invalid program segment count");
    }
    tokio::fs::create_dir_all(cache).await?;
    let mut tasks = Vec::new();
    for segment in plan.segments {
        let tts = tts.clone();
        let host = host_for(hosts, segment.category.as_deref())?.clone();
        let cache = cache.to_path_buf();
        let fingerprint = fingerprint.to_string();
        tasks.push(
            async move {
                let text = super::speech_text::prepare_speech(&segment.text);
                if text.is_empty() {
                    bail!("empty program segment");
                }
                let reference =
                    tokio::fs::read(host.voice.strip_prefix("file://").unwrap_or(&host.voice))
                        .await
                        .context("read host reference")?;
                let key = hex::encode(Sha256::digest(format!(
                    "{fingerprint}\n{}\n{:?}\n{}\n{}",
                    host.voice,
                    host.prompt_text,
                    hex::encode(Sha256::digest(&reference)),
                    text
                )));
                let path = cache.join(format!("{key}.wav"));
                if let Ok(bytes) = tokio::fs::read(&path).await {
                    if decode(&bytes).is_ok() {
                        return Ok(path);
                    }
                }
                log::info!(
                    "[RadioProgram] Synthesizing category={:?} host={} chars={}",
                    segment.category,
                    host.name,
                    text.chars().count()
                );
                let wav = tts
                    .speak_with_voice(&text, &host.voice, host.prompt_text.as_deref())
                    .await?;
                decode(&wav)?;
                let temp = cache.join(format!("{key}-{}.tmp", uuid::Uuid::new_v4()));
                tokio::fs::write(&temp, wav).await?;
                tokio::fs::rename(temp, &path).await?;
                Ok::<_, anyhow::Error>(path)
            }
            .boxed(),
        );
    }
    let paths: Vec<PathBuf> = stream::iter(tasks)
        .buffered(concurrency.clamp(1, 4))
        .try_collect()
        .await?;
    // Decode one section at a time and encode only once: no MP3 joins or accumulated PCM.
    let mut encoder = None;
    let mut rate = 0;
    let mut samples = 0_u64;
    for (index, path) in paths.iter().enumerate() {
        let bytes = tokio::fs::read(path).await?;
        let (segment_rate, pcm) = tokio::task::spawn_blocking(move || decode(&bytes)).await??;
        if index == 0 {
            rate = segment_rate;
            encoder = Some(tts::Mp3StreamProcess::start_s16le(rate)?);
        }
        if rate != segment_rate {
            bail!("program host sample rates differ");
        }
        if index > 0 {
            let silence = vec![0_u8; (rate / 4) as usize * 2];
            encoder.as_mut().unwrap().write_all(&silence).await?;
            samples += (rate / 4) as u64;
        }
        samples += (pcm.len() / 2) as u64;
        encoder.as_mut().unwrap().write_all(&pcm).await?;
    }
    let mp3 = encoder.context("empty program")?.finish().await?;
    Ok((mp3, samples.div_ceil(rate as u64) as i64))
}

fn decode(wav: &[u8]) -> Result<(u32, Vec<u8>)> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(wav))?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
        || spec.sample_rate == 0
    {
        bail!("unsupported program WAV format");
    }
    let mut pcm = Vec::with_capacity(reader.len() as usize * 2);
    for sample in reader.samples::<i16>() {
        pcm.extend_from_slice(&sample?.to_le_bytes());
    }
    if pcm.len() < spec.sample_rate as usize {
        bail!("program segment shorter than half a second");
    }
    Ok((spec.sample_rate, pcm))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_hosts_are_stable_and_unknown_categories_fail() {
        let hosts = vec![
            Host {
                name: "lead".into(),
                voice: "a.wav".into(),
                prompt_text: None,
                categories: vec!["AI".into()],
            },
            Host {
                name: "other".into(),
                voice: "b.wav".into(),
                prompt_text: None,
                categories: vec!["Science".into()],
            },
        ];
        assert_eq!(host_for(&hosts, None).unwrap().name, "lead");
        assert_eq!(host_for(&hosts, Some("Science")).unwrap().name, "other");
        assert!(host_for(&hosts, Some("unknown")).is_err());
    }
    #[test]
    fn wav_validation_rejects_truncation_and_preserves_samples() {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(
            &mut out,
            hound::WavSpec {
                channels: 1,
                sample_rate: 24000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..24000 {
            w.write_sample(123_i16).unwrap();
        }
        w.finalize().unwrap();
        let bytes = out.into_inner();
        let (rate, pcm) = decode(&bytes).unwrap();
        assert_eq!(rate, 24000);
        assert_eq!(pcm.len(), 48000);
        assert_eq!(&pcm[..2], &123_i16.to_le_bytes());
        assert!(decode(&bytes[..100]).is_err());
    }
}
