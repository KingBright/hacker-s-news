use crate::core::config::{Config, VoiceWorkerConfig};
use crate::core::nexus::{NexusClient, VoiceJobPayload};
use crate::core::tts::TtsClient;
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{self, Duration, MissedTickBehavior};
use uuid::Uuid;

const DEFAULT_POLL_INTERVAL_SECS: u64 = 30;
const DEFAULT_CONCURRENCY: usize = 1;
const MAX_CONCURRENCY: usize = 4;
const DEFAULT_MAX_JOBS_PER_TICK: usize = 1;
const DEFAULT_LEASE_SECONDS: i64 = 30 * 60;
const DEFAULT_REPAIR_INTERVAL_SECS: u64 = 10 * 60;
const DEFAULT_REPAIR_LIMIT: i64 = 20;
const MIN_VALID_MP3_BYTES: usize = 4 * 1024;
const MAX_ERROR_CHARS: usize = 1_000;

#[derive(Debug, Clone)]
struct VoiceWorkerSettings {
    enabled: bool,
    worker_id: String,
    poll_interval_secs: u64,
    concurrency: usize,
    max_jobs_per_tick: usize,
    lease_seconds: i64,
    repair_missing_audio: bool,
    repair_interval_secs: u64,
    repair_limit: i64,
    voice_kinds: Option<Vec<String>>,
    hosts: Vec<crate::core::config::Host>,
    tts_fingerprint: String,
}

impl VoiceWorkerSettings {
    fn from_config(config: &Config) -> Self {
        let raw = config.voice_worker.clone();
        let enabled = raw.as_ref().map(|value| value.enabled).unwrap_or(true);
        let worker_id = raw
            .as_ref()
            .and_then(|value| value.worker_id.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(default_worker_id);
        let poll_interval_secs = raw
            .as_ref()
            .and_then(|value| value.poll_interval_secs)
            .unwrap_or(DEFAULT_POLL_INTERVAL_SECS)
            .clamp(5, 3600);
        let concurrency = raw
            .as_ref()
            .and_then(|value| value.concurrency)
            .unwrap_or(DEFAULT_CONCURRENCY)
            .clamp(1, MAX_CONCURRENCY);
        let max_jobs_per_tick = raw
            .as_ref()
            .and_then(|value| value.max_jobs_per_tick)
            .unwrap_or(DEFAULT_MAX_JOBS_PER_TICK)
            .clamp(1, 10);
        let lease_seconds = raw
            .as_ref()
            .and_then(|value| value.lease_seconds)
            .unwrap_or(DEFAULT_LEASE_SECONDS)
            .clamp(60, 6 * 60 * 60);
        let repair_missing_audio = raw
            .as_ref()
            .and_then(|value| value.repair_missing_audio)
            .unwrap_or(true);
        let repair_interval_secs = raw
            .as_ref()
            .and_then(|value| value.repair_interval_secs)
            .unwrap_or(DEFAULT_REPAIR_INTERVAL_SECS)
            .clamp(60, 24 * 60 * 60);
        let repair_limit = raw
            .as_ref()
            .and_then(|value| value.repair_limit)
            .unwrap_or(DEFAULT_REPAIR_LIMIT)
            .clamp(1, 200);
        let voice_kinds = raw.as_ref().and_then(normalize_voice_kinds);

        Self {
            enabled,
            worker_id,
            poll_interval_secs,
            concurrency,
            max_jobs_per_tick,
            lease_seconds,
            repair_missing_audio,
            repair_interval_secs,
            repair_limit,
            voice_kinds,
            hosts: config.hosts.clone().unwrap_or_default(),
            tts_fingerprint: format!("{:?}", config.tts),
        }
    }

    fn worker_for_slot(&self, slot: usize) -> Self {
        let mut settings = self.clone();
        if self.concurrency > 1 {
            settings.worker_id = format!("{}-{}", self.worker_id, slot + 1);
        }
        settings
    }
}

pub async fn run_voice_worker_loop(
    config: Arc<Config>,
    tts: Arc<TtsClient>,
    nexus: Arc<NexusClient>,
) {
    let settings = VoiceWorkerSettings::from_config(&config);
    if !settings.enabled {
        log::info!("[VoiceWorker] Disabled by config");
        return;
    }

    log::info!(
        "[VoiceWorker] Starting worker_group={} concurrency={} poll_interval={}s lease={}s max_jobs_per_tick={} repair_missing_audio={}",
        settings.worker_id,
        settings.concurrency,
        settings.poll_interval_secs,
        settings.lease_seconds,
        settings.max_jobs_per_tick,
        settings.repair_missing_audio
    );

    if settings.repair_missing_audio {
        let repair_settings = settings.clone();
        let repair_nexus = nexus.clone();
        tokio::spawn(async move {
            run_voice_repair_loop(repair_settings, repair_nexus).await;
        });
    }

    if settings.concurrency == 1 {
        run_voice_worker_instance(settings, tts, nexus).await;
        return;
    }

    let mut workers = JoinSet::new();
    for slot in 0..settings.concurrency {
        let slot_settings = settings.worker_for_slot(slot);
        let slot_tts = tts.clone();
        let slot_nexus = nexus.clone();
        workers.spawn(async move {
            run_voice_worker_instance(slot_settings, slot_tts, slot_nexus).await;
        });
    }

    while let Some(result) = workers.join_next().await {
        match result {
            Ok(()) => log::warn!("[VoiceWorker] A worker loop exited unexpectedly"),
            Err(e) => log::error!("[VoiceWorker] A worker loop panicked: {}", e),
        }
    }
}

async fn run_voice_worker_instance(
    settings: VoiceWorkerSettings,
    tts: Arc<TtsClient>,
    nexus: Arc<NexusClient>,
) {
    log::info!(
        "[VoiceWorker] Started worker_id={} poll_interval={}s lease={}s max_jobs_per_tick={}",
        settings.worker_id,
        settings.poll_interval_secs,
        settings.lease_seconds,
        settings.max_jobs_per_tick
    );

    let mut interval = time::interval(Duration::from_secs(settings.poll_interval_secs));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        interval.tick().await;
        for _ in 0..settings.max_jobs_per_tick {
            match process_next_voice_job(&settings, tts.clone(), nexus.clone()).await {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    log::error!(
                        "[VoiceWorker] Cycle failed for worker_id={}: {}",
                        settings.worker_id,
                        e
                    );
                    break;
                }
            }
        }
    }
}

async fn run_voice_repair_loop(settings: VoiceWorkerSettings, nexus: Arc<NexusClient>) {
    let mut interval = time::interval(Duration::from_secs(settings.repair_interval_secs));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        interval.tick().await;
        match nexus
            .repair_missing_voice_jobs(Some(settings.repair_limit))
            .await
        {
            Ok(stats)
                if stats.created > 0
                    || stats.backfilled_completed > 0
                    || stats.skipped_recent_failures > 0 =>
            {
                log::info!(
                    "[VoiceWorker] Voice repair scanned={} created={} backfilled={} active={} recent_failures={} missing_text={}",
                    stats.scanned,
                    stats.created,
                    stats.backfilled_completed,
                    stats.skipped_active,
                    stats.skipped_recent_failures,
                    stats.skipped_missing_text
                );
            }
            Ok(_) => {}
            Err(e) => log::warn!("[VoiceWorker] Voice repair failed: {}", e),
        }
    }
}

async fn process_next_voice_job(
    settings: &VoiceWorkerSettings,
    tts: Arc<TtsClient>,
    nexus: Arc<NexusClient>,
) -> Result<bool> {
    let job = nexus
        .lease_voice_job(
            &settings.worker_id,
            settings.voice_kinds.clone(),
            Some(settings.lease_seconds),
        )
        .await
        .context("lease voice job")?;

    let Some(job) = job else {
        return Ok(false);
    };
    let job_id = job.id.clone();
    let lease_token = job
        .lease_token
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("leased voice job {} missing lease_token", job_id))?;

    log::info!(
        "[VoiceWorker] Processing job={} target={}:{} kind={} text_chars={}",
        job.id,
        job.target_type,
        job.target_id,
        job.voice_kind,
        job.text.chars().count()
    );

    let heartbeat = start_voice_job_heartbeat(
        nexus.clone(),
        job.id.clone(),
        lease_token.clone(),
        settings.lease_seconds,
    );
    let result = synthesize_and_upload(&job, tts, nexus.clone(), settings).await;
    heartbeat.abort();

    match result {
        Ok((audio_url, duration_sec)) => {
            nexus
                .complete_voice_job(&job.id, &lease_token, &audio_url, Some(duration_sec))
                .await
                .with_context(|| format!("complete voice job {}", job.id))?;
            if job.voice_kind == "radio_program" {
                let _ = tokio::fs::remove_dir_all(program_cache(&job.id)).await;
            }
            log::info!(
                "[VoiceWorker] Completed job={} audio_url={} duration={}s",
                job.id,
                audio_url,
                duration_sec
            );
            Ok(true)
        }
        Err(e) => {
            let message = truncate_error(&format!("{e:#}"));
            if let Err(fail_err) = nexus.fail_voice_job(&job.id, &lease_token, &message).await {
                return Err(fail_err)
                    .with_context(|| format!("mark voice job {} failed after: {}", job.id, e));
            }
            log::warn!(
                "[VoiceWorker] Job {} failed and was requeued: {}",
                job.id,
                e
            );
            Ok(true)
        }
    }
}

fn start_voice_job_heartbeat(
    nexus: Arc<NexusClient>,
    job_id: String,
    lease_token: String,
    lease_seconds: i64,
) -> JoinHandle<()> {
    let interval_secs = (lease_seconds / 3).clamp(60, 20 * 60) as u64;
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(interval_secs));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            match nexus
                .heartbeat_voice_job(&job_id, &lease_token, Some(lease_seconds))
                .await
            {
                Ok(()) => log::info!(
                    "[VoiceWorker] Heartbeat renewed job={} lease={}s",
                    job_id,
                    lease_seconds
                ),
                Err(e) => log::warn!("[VoiceWorker] Heartbeat failed for job={}: {}", job_id, e),
            }
        }
    })
}

async fn synthesize_and_upload(
    job: &VoiceJobPayload,
    tts: Arc<TtsClient>,
    nexus: Arc<NexusClient>,
    settings: &VoiceWorkerSettings,
) -> Result<(String, i64)> {
    let (mp3_bytes, duration_sec) = if job.voice_kind == "radio_program" {
        super::radio_program::render(
            &job.text,
            &settings.hosts,
            tts,
            &program_cache(&job.id),
            &settings.tts_fingerprint,
            settings.concurrency,
        )
        .await?
    } else {
        let text = normalize_voice_text(&job.text)?;
        tts.speak_mp3(&text)
            .await
            .with_context(|| format!("synthesize voice job {}", job.id))?
    };
    validate_generated_mp3(&mp3_bytes, duration_sec)
        .with_context(|| format!("validate voice job {}", job.id))?;
    let file_prefix = sanitize_file_prefix(&job.file_prefix);
    let file_name = format!("{}_{}.mp3", file_prefix, Uuid::new_v4());
    let audio_url = nexus
        .upload_audio(mp3_bytes, &file_name)
        .await
        .with_context(|| format!("upload voice job {}", job.id))?;
    Ok((audio_url, duration_sec))
}

fn program_cache(job_id: &str) -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".freshloop/cache/program-audio")
        .join(sanitize_file_prefix(job_id))
}

fn validate_generated_mp3(mp3_bytes: &[u8], duration_sec: i64) -> Result<()> {
    if duration_sec <= 0 {
        anyhow::bail!(
            "generated audio duration is not positive: {}s",
            duration_sec
        );
    }
    if mp3_bytes.len() < MIN_VALID_MP3_BYTES {
        anyhow::bail!(
            "generated audio is too small: {} bytes < {} bytes",
            mp3_bytes.len(),
            MIN_VALID_MP3_BYTES
        );
    }
    Ok(())
}

fn normalize_voice_kinds(config: &VoiceWorkerConfig) -> Option<Vec<String>> {
    let kinds = config
        .voice_kinds
        .as_ref()?
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if kinds.is_empty() {
        None
    } else {
        Some(kinds)
    }
}

fn normalize_voice_text(text: &str) -> Result<String> {
    let normalized = super::speech_text::prepare_speech(text);
    if normalized.is_empty() {
        anyhow::bail!("voice job text is empty");
    }
    Ok(normalized)
}

fn sanitize_file_prefix(prefix: &str) -> String {
    let cleaned = prefix
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    if cleaned.is_empty() {
        "freshloop_voice".to_string()
    } else {
        cleaned.chars().take(80).collect()
    }
}

fn truncate_error(message: &str) -> String {
    message.chars().take(MAX_ERROR_CHARS).collect()
}

fn default_worker_id() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .map(|value| sanitize_file_prefix(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "local".to_string());
    format!("cortex-{}-{}", host, std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_voice_worker(extra: &str) -> Config {
        let content = format!(
            r#"
[nexus]
api_url = "http://127.0.0.1:8899"
auth_key = "test-key"

[llm]
model = "test-model"
api_url = "http://127.0.0.1:1234/v1"

[tts]
engine = "voxcpm"

[voice_worker]
worker_id = "voice-worker"
{extra}
"#
        );
        toml::from_str(&content).expect("valid config")
    }

    #[test]
    fn file_prefix_is_safe_for_upload_name() {
        assert_eq!(
            sanitize_file_prefix("curated/article 01"),
            "curated_article_01"
        );
        assert_eq!(sanitize_file_prefix("///"), "freshloop_voice");
    }

    #[test]
    fn voice_text_rejects_empty_jobs() {
        assert!(normalize_voice_text("  \0 ").is_err());
        assert_eq!(normalize_voice_text("  hello\0 ").unwrap(), "hello");
    }

    #[test]
    fn generated_mp3_validation_rejects_tiny_or_zero_duration_audio() {
        assert!(validate_generated_mp3(&vec![0; MIN_VALID_MP3_BYTES], 1).is_ok());
        assert!(validate_generated_mp3(&vec![0; MIN_VALID_MP3_BYTES - 1], 1).is_err());
        assert!(validate_generated_mp3(&vec![0; MIN_VALID_MP3_BYTES], 0).is_err());
    }

    #[test]
    fn errors_are_truncated_for_api_storage() {
        let long = "a".repeat(MAX_ERROR_CHARS + 10);
        assert_eq!(truncate_error(&long).chars().count(), MAX_ERROR_CHARS);
    }

    #[test]
    fn worker_settings_default_to_single_concurrency() {
        let config = config_with_voice_worker("");
        let settings = VoiceWorkerSettings::from_config(&config);

        assert_eq!(settings.concurrency, 1);
        assert_eq!(settings.worker_for_slot(0).worker_id, "voice-worker");
        assert!(settings.repair_missing_audio);
        assert_eq!(settings.repair_interval_secs, DEFAULT_REPAIR_INTERVAL_SECS);
        assert_eq!(settings.repair_limit, DEFAULT_REPAIR_LIMIT);
    }

    #[test]
    fn worker_settings_clamp_concurrency() {
        let too_low = config_with_voice_worker("concurrency = 0");
        let too_high = config_with_voice_worker("concurrency = 99");

        assert_eq!(VoiceWorkerSettings::from_config(&too_low).concurrency, 1);
        assert_eq!(
            VoiceWorkerSettings::from_config(&too_high).concurrency,
            MAX_CONCURRENCY
        );
    }

    #[test]
    fn concurrent_workers_get_distinct_ids() {
        let config = config_with_voice_worker("concurrency = 3");
        let settings = VoiceWorkerSettings::from_config(&config);

        assert_eq!(settings.worker_for_slot(0).worker_id, "voice-worker-1");
        assert_eq!(settings.worker_for_slot(1).worker_id, "voice-worker-2");
        assert_eq!(settings.worker_for_slot(2).worker_id, "voice-worker-3");
    }
}
