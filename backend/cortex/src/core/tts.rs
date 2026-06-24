use crate::core::config::{load_config, TtsConfig as CortexTtsConfig};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{mpsc::RecvTimeoutError, Arc, OnceLock};
use std::time::{Duration, Instant};
use sysinfo::{Pid, System};
use tokio::process::Command;
use tokio::sync::{Mutex, Semaphore};
use tts::{EngineFactory, TtsEngine};
use tts::{
    MagicTtsConfig as LibMagicTtsConfig, MossTtsConfig as LibMossTtsConfig, Qwen3Config,
    TtsConfig as LibTtsConfig, VoxCpmConfig,
};
use uuid::Uuid;

// Maximum total characters to prevent excessive memory usage
// ~8000 chars ≈ 15-25 minutes audio ≈ 80-150MB memory
const MAX_TOTAL_CHARS: usize = 8000;
const DEFAULT_TTS_WORKER_MEMORY_LIMIT_MB: u64 = 24 * 1024;
const DEFAULT_TTS_WORKER_IDLE_TIMEOUT_SECS: u64 = 20 * 60;
const PRODUCTION_VOXCPM_MAX_LEN: usize = 1024;
const PRODUCTION_VOXCPM_INFERENCE_TIMESTEPS: usize = 10;
const PRODUCTION_VOXCPM_CFG_VALUE: f64 = 2.0;
const PRODUCTION_VOXCPM_RATIO_THRESHOLD: f64 = 6.0;
const TTS_CHUNK_MAX_ATTEMPTS: usize = 3;
const TTS_CHUNK_MIN_RMS: f32 = 0.0002;
const TTS_CHUNK_MIN_PEAK: f32 = 0.002;
const TTS_CHUNK_HARD_PEAK_LIMIT: f32 = 1.5;
const TTS_CHUNK_MAX_CLIPPED_RATIO: f64 = 0.03;
const TTS_CHUNK_MAX_NOISY_FRAME_RATIO: f64 = 0.35;
const TTS_CHUNK_MAX_ZERO_CROSSING_RATE: f64 = 0.24;
const TTS_CHUNK_MAX_HIGH_FREQ_ENERGY_RATIO: f64 = 0.9;
const TTS_NOISY_FRAME_ZERO_CROSSING_RATE: f64 = 0.28;
const TTS_NOISY_FRAME_HIGH_FREQ_ENERGY_RATIO: f64 = 1.2;
const TTS_MAX_STABLE_CHUNK_CHARS: usize = 200;
const MOSS_TTS_MAX_STABLE_CHUNK_CHARS: usize = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TtsWorkerMode {
    Wav,
    Mp3,
    Mp3Chunks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TtsWorkerRequest {
    config: LibTtsConfig,
    mode: TtsWorkerMode,
    text: Option<String>,
    chunks: Option<Vec<String>>,
    voice_override: Option<String>,
    prompt_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TtsWorkerResponse {
    sample_rate: usize,
    duration_seconds: Option<i64>,
}

pub struct TtsClient {
    config: LibTtsConfig,
    engine: Arc<Mutex<Option<Box<dyn TtsEngine>>>>,
    keep_engine_loaded: bool,
    memory_pressure_relief: bool,
    process_isolation: bool,
    worker_memory_limit_mb: u64,
    worker_idle_timeout: Duration,
}

impl TtsClient {
    pub fn new(config: CortexTtsConfig) -> Self {
        let requested_engine = config.engine.as_deref().unwrap_or("voxcpm");
        let allow_experimental = experimental_tts_enabled();
        let keep_engine_loaded = if allow_experimental {
            config.keep_engine_loaded.unwrap_or(false)
        } else {
            false
        };
        let memory_pressure_relief = if allow_experimental {
            config.memory_pressure_relief.unwrap_or(true)
        } else {
            true
        };
        let (engine_name, device_name) = resolve_cortex_tts_runtime_policy(
            requested_engine,
            config.device.as_deref(),
            allow_experimental,
        );
        let policy_changed = requested_engine.trim() != engine_name
            || config
                .device
                .as_deref()
                .map(|device| device.trim() != device_name)
                .unwrap_or(false);
        if policy_changed {
            log::warn!(
                "[TTS] Cortex production policy requested engine={}, device={}; using engine={}, device={} instead. Set FRESHLOOP_ALLOW_EXPERIMENTAL_TTS=1 only for explicit local experiments.",
                requested_engine,
                config.device.as_deref().unwrap_or("<engine-default>"),
                engine_name,
                device_name,
            );
        }
        let process_isolation = if allow_experimental {
            config
                .process_isolation
                .unwrap_or_else(|| engine_uses_heavy_native_runtime(engine_name))
        } else {
            true
        };
        let worker_memory_limit_mb = config
            .worker_memory_limit_mb
            .unwrap_or(DEFAULT_TTS_WORKER_MEMORY_LIMIT_MB);
        let worker_idle_timeout = Duration::from_secs(
            config
                .worker_idle_timeout_secs
                .or(config.worker_timeout_secs)
                // worker_timeout_secs is kept as a backwards-compatible alias.
                // It is interpreted as idle timeout, not total generation time.
                .unwrap_or(DEFAULT_TTS_WORKER_IDLE_TIMEOUT_SECS),
        );

        // Map Cortex internal config to external TTS Library config schema
        let lib_config = LibTtsConfig {
            engine: Some(engine_name.to_string()),
            device: Some(device_name.to_string()),
            voxcpm: config.voxcpm.as_ref().map(|v| VoxCpmConfig {
                model_path: v.model_path.clone(),
                prompt_text: v.prompt_text.clone(),
                prompt_wav_path: v.prompt_wav_path.clone(),
                control_instruction: production_voxcpm_control_instruction(
                    v.control_instruction.clone(),
                    allow_experimental,
                ),
                min_len: v.min_len,
                max_len: production_voxcpm_max_len(v.max_len, allow_experimental),
                inference_timesteps: production_voxcpm_inference_timesteps(
                    v.inference_timesteps,
                    allow_experimental,
                ),
                cfg_value: production_voxcpm_cfg_value(v.cfg_value, allow_experimental),
                retry_badcase: v.retry_badcase,
                retry_badcase_ratio_threshold: production_voxcpm_ratio_threshold(
                    v.retry_badcase_ratio_threshold,
                    allow_experimental,
                ),
            }),
            qwen3: config.qwen3.as_ref().map(|q| Qwen3Config {
                model_dir: q.model_dir.clone(),
                prompt_text: q.prompt_text.clone(),
                prompt_wav_path: q.prompt_wav_path.clone(),
                language: q.language.clone(),
                speaker: q.speaker.clone(),
                voice_design_instruction: q.voice_design_instruction.clone(),
                max_length: q.max_length,
                temperature: q.temperature,
                top_k: q.top_k,
                top_p: q.top_p,
                repetition_penalty: q.repetition_penalty,
                seed: q.seed,
                chunk_frames: q.chunk_frames,
            }),
            magictts: config.magictts.as_ref().map(|m| LibMagicTtsConfig {
                model_dir: m.model_dir.clone(),
                prompt_text: m.prompt_text.clone(),
                prompt_wav_path: m.prompt_wav_path.clone(),
                vocab_path: m.vocab_path.clone(),
                steps: m.steps,
                cfg_strength: m.cfg_strength,
                default_content_ms: m.default_content_ms,
                default_punct_ms: m.default_punct_ms,
            }),
            moss: config.moss.as_ref().map(|m| LibMossTtsConfig {
                model_dir: m.model_dir.clone(),
                prompt_text: m.prompt_text.clone(),
                prompt_wav_path: m.prompt_wav_path.clone(),
                sample_mode: m.sample_mode.clone(),
                text_temperature: m.text_temperature,
                text_top_p: m.text_top_p,
                text_top_k: m.text_top_k,
                audio_temperature: m.audio_temperature,
                audio_top_p: m.audio_top_p,
                audio_top_k: m.audio_top_k,
                audio_repetition_penalty: m.audio_repetition_penalty,
                max_new_frames: m.max_new_frames,
                voice_clone_max_text_tokens: m.voice_clone_max_text_tokens,
                seed: m.seed,
                intra_threads: m.intra_threads,
                inter_threads: m.inter_threads,
                chunk_max_chars: m.chunk_max_chars,
            }),
        };

        log::info!(
            "[TTS] Runtime policy: engine={}, device={}, keep_engine_loaded={}, process_isolation={}, worker_memory_limit_mb={}, worker_idle_timeout_secs={}",
            engine_name,
            device_name,
            keep_engine_loaded,
            process_isolation,
            worker_memory_limit_mb,
            worker_idle_timeout.as_secs(),
        );

        Self {
            config: lib_config,
            engine: Arc::new(Mutex::new(None)),
            keep_engine_loaded,
            memory_pressure_relief,
            process_isolation,
            worker_memory_limit_mb,
            worker_idle_timeout,
        }
    }

    pub async fn speak(&self, text: &str) -> Result<Vec<u8>> {
        let result = if self.process_isolation {
            self.speak_via_worker(text, None, None).await
        } else {
            self.speak_and_convert(text, None, None).await
        };
        self.release_engine_after_use("speak").await;
        result
    }

    pub async fn speak_mp3(&self, text: &str) -> Result<(Vec<u8>, i64)> {
        let result = if self.process_isolation {
            self.speak_mp3_via_worker(text, None, None).await
        } else {
            self.speak_and_encode_mp3(text, None, None).await
        };
        self.release_engine_after_use("speak_mp3").await;
        result
    }

    pub async fn speak_with_voice(
        &self,
        text: &str,
        voice_path: &str,
        prompt_override: Option<&str>,
    ) -> Result<Vec<u8>> {
        let voice_override = Some(voice_path.to_string());
        let prompt_override = prompt_override.map(|s| s.to_string());
        let result = if self.process_isolation {
            self.speak_via_worker(text, voice_override, prompt_override)
                .await
        } else {
            self.speak_and_convert(text, voice_override, prompt_override)
                .await
        };
        self.release_engine_after_use("speak_with_voice").await;
        result
    }

    pub async fn speak_mp3_with_voice(
        &self,
        text: &str,
        voice_path: &str,
        prompt_override: Option<&str>,
    ) -> Result<(Vec<u8>, i64)> {
        let voice_override = Some(voice_path.to_string());
        let prompt_override = prompt_override.map(|s| s.to_string());
        let result = if self.process_isolation {
            self.speak_mp3_via_worker(text, voice_override, prompt_override)
                .await
        } else {
            self.speak_and_encode_mp3(text, voice_override, prompt_override)
                .await
        };
        self.release_engine_after_use("speak_mp3_with_voice").await;
        result
    }

    pub async fn speak_mp3_from_chunks<F>(
        &self,
        mut chunks_rx: tokio::sync::mpsc::Receiver<String>,
        voice_override: Option<String>,
        prompt_override: Option<String>,
        mut clean_chunk: F,
    ) -> Result<(Vec<u8>, i64)>
    where
        F: FnMut(&str) -> String + Send,
    {
        if self.process_isolation {
            let mut chunks = Vec::new();
            let mut total_chars = 0_usize;
            while let Some(chunk_text) = chunks_rx.recv().await {
                let tts_text = clean_chunk(&chunk_text);
                if tts_text.trim().is_empty() || tts_text.trim().contains("SKIP") {
                    continue;
                }
                if !push_bounded_chunk(&mut chunks, &tts_text, &mut total_chars) {
                    break;
                }
            }

            let result = self
                .speak_mp3_chunks_via_worker(chunks, voice_override, prompt_override)
                .await;
            self.release_engine_after_use("speak_mp3_from_chunks").await;
            return result;
        }

        let result = async {
            let mut mp3_process = None;
            let mut pcm_stream = None;
            let mut sample_rate = 0_u32;
            let mut failed_chunks = Vec::new();
            let mut chunk_idx = 0_usize;
            let mut total_chars = 0_usize;

            while let Some(chunk_text) = chunks_rx.recv().await {
                chunk_idx += 1;
                let tts_text = clean_chunk(&chunk_text);
                if tts_text.trim().is_empty() || tts_text.trim().contains("SKIP") {
                    continue;
                }
                let mut bounded = Vec::new();
                let keep_reading =
                    push_bounded_chunk(&mut bounded, &tts_text, &mut total_chars);
                let Some(tts_text) = bounded.into_iter().next() else {
                    continue;
                };

                log::info!(
                    "[TTS Pipeline] Generating MP3 audio for chunk {} ({} chars)",
                    chunk_idx,
                    tts_text.chars().count()
                );

                let chunk_result = {
                    let mut engine_guard = self.engine.lock().await;
                    if engine_guard.is_none() {
                        log::info!("[TTS] Lazy-loading engine on first synthesis request...");
                        let engine = EngineFactory::create(&self.config)?;
                        engine_guard.replace(engine);
                    }
                    let engine = engine_guard
                        .as_mut()
                        .expect("TTS engine is initialized above");

                    if voice_override.is_some() || prompt_override.is_some() {
                        let override_prompt = tts::VoicePrompt {
                            text: prompt_override.clone(),
                            wav_path: voice_override.clone(),
                        };
                        if let Err(e) = engine.cache_voice_prompt(&override_prompt) {
                            log::warn!("Failed to apply temporary voice override cache: {}", e);
                        }
                    }

                    let samples = engine.synthesize_chunk(&tts_text);
                    let current_sample_rate = engine.sample_rate() as u32;
                    samples.map(|samples| (samples, current_sample_rate))
                };

                let (samples, current_sample_rate) = match chunk_result {
                    Ok(result) => result,
                    Err(e) => {
                        log::error!("[TTS Pipeline] FAILED for chunk {}: {}", chunk_idx, e);
                        failed_chunks.push(chunk_idx);
                        continue;
                    }
                };

                if mp3_process.is_none() {
                    sample_rate = current_sample_rate;
                    mp3_process = Some(tts::Mp3StreamProcess::start_s16le(sample_rate)?);
                    pcm_stream = Some(tts::PcmStreamEncoder::new(sample_rate as usize, 0.05));
                } else if current_sample_rate != sample_rate {
                    log::warn!(
                        "[TTS Pipeline] sample rate changed from {} to {}; continuing with initial encoder rate",
                        sample_rate,
                        current_sample_rate
                    );
                }

                let bytes = pcm_stream
                    .as_mut()
                    .expect("PCM stream initialized with MP3 process")
                    .push(&samples);
                if !bytes.is_empty() {
                    mp3_process
                        .as_mut()
                        .expect("MP3 process initialized above")
                        .write_all(&bytes)
                        .await?;
                }

                if !keep_reading {
                    break;
                }
            }

            if !failed_chunks.is_empty() {
                anyhow::bail!("TTS generation failed for chunks: {:?}", failed_chunks);
            }

            let mut mp3_process = mp3_process
                .ok_or_else(|| anyhow::anyhow!("TTS pipeline produced no audio samples"))?;
            let mut pcm_stream = pcm_stream
                .ok_or_else(|| anyhow::anyhow!("TTS pipeline produced no audio samples"))?;
            let tail = pcm_stream.finish();
            if !tail.is_empty() {
                mp3_process.write_all(&tail).await?;
            }

            let mp3_bytes = mp3_process.finish().await?;
            let duration = (pcm_stream.output_samples() as f64 / sample_rate as f64).ceil() as i64;
            Ok((mp3_bytes, duration))
        }
        .await;
        self.release_engine_after_use("speak_mp3_from_chunks").await;
        result
    }

    async fn release_engine_after_use(&self, reason: &str) {
        if self.keep_engine_loaded {
            return;
        }

        let released = {
            let mut engine_guard = self.engine.lock().await;
            engine_guard.take().is_some()
        };

        if released {
            log::info!(
                "[TTS] Released TTS engine after {} to cap Cortex memory",
                reason
            );
            if self.memory_pressure_relief {
                relieve_allocator_pressure();
            }
        }
    }

    async fn speak_via_worker(
        &self,
        raw_text: &str,
        voice_override: Option<String>,
        prompt_override: Option<String>,
    ) -> Result<Vec<u8>> {
        let request = TtsWorkerRequest {
            config: self.config.clone(),
            mode: TtsWorkerMode::Wav,
            text: Some(Self::bounded_text(raw_text)),
            chunks: None,
            voice_override,
            prompt_override,
        };
        let (bytes, _) = self.run_worker_request(request, "wav").await?;
        Ok(bytes)
    }

    async fn speak_mp3_via_worker(
        &self,
        raw_text: &str,
        voice_override: Option<String>,
        prompt_override: Option<String>,
    ) -> Result<(Vec<u8>, i64)> {
        let request = TtsWorkerRequest {
            config: self.config.clone(),
            mode: TtsWorkerMode::Mp3,
            text: Some(Self::bounded_text(raw_text)),
            chunks: None,
            voice_override,
            prompt_override,
        };
        let (bytes, response) = self.run_worker_request(request, "mp3").await?;
        Ok((bytes, response.duration_seconds.unwrap_or(0)))
    }

    async fn speak_mp3_chunks_via_worker(
        &self,
        chunks: Vec<String>,
        voice_override: Option<String>,
        prompt_override: Option<String>,
    ) -> Result<(Vec<u8>, i64)> {
        if chunks.is_empty() {
            anyhow::bail!("TTS pipeline produced no audio samples");
        }

        let request = TtsWorkerRequest {
            config: self.config.clone(),
            mode: TtsWorkerMode::Mp3Chunks,
            text: None,
            chunks: Some(chunks),
            voice_override,
            prompt_override,
        };
        let (bytes, response) = self.run_worker_request(request, "mp3").await?;
        Ok((bytes, response.duration_seconds.unwrap_or(0)))
    }

    async fn run_worker_request(
        &self,
        request: TtsWorkerRequest,
        output_ext: &str,
    ) -> Result<(Vec<u8>, TtsWorkerResponse)> {
        let semaphore = tts_worker_semaphore();
        if semaphore.available_permits() == 0 {
            log::info!("[TTS] Waiting for existing TTS worker to finish before starting another");
        }
        let _worker_slot = semaphore
            .acquire()
            .await
            .map_err(|e| anyhow::anyhow!("TTS worker semaphore closed: {}", e))?;

        let temp_dir = tts_worker_temp_dir();
        std::fs::create_dir_all(&temp_dir)?;

        let id = Uuid::new_v4();
        let input_path = temp_dir.join(format!("{id}.json"));
        let output_path = temp_dir.join(format!("{id}.{output_ext}"));
        let response_path = temp_dir.join(format!("{id}.response.json"));
        let progress_path = temp_dir.join(format!("{id}.progress"));
        let stdout_path = temp_dir.join(format!("{id}.stdout.log"));
        let stderr_path = temp_dir.join(format!("{id}.stderr.log"));

        std::fs::write(&input_path, serde_json::to_vec(&request)?)?;
        write_worker_progress(&progress_path, "queued")?;

        let stdout_file = std::fs::File::create(&stdout_path)?;
        let stderr_file = std::fs::File::create(&stderr_path)?;
        let mut child = Command::new(std::env::current_exe()?)
            .arg("tts-worker")
            .arg(&input_path)
            .arg(&output_path)
            .arg(&response_path)
            .arg(&progress_path)
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .spawn()?;

        log::info!(
            "[TTS] Spawned isolated TTS worker pid={:?}, memory_limit={} MB, idle_timeout={}s",
            child.id(),
            self.worker_memory_limit_mb,
            self.worker_idle_timeout.as_secs()
        );

        let status = wait_for_worker_with_limits(
            &mut child,
            &progress_path,
            self.worker_memory_limit_mb,
            self.worker_idle_timeout,
        )
        .await?;

        if !status.success() {
            let stderr = read_log_tail(&stderr_path, 4096);
            let stdout = read_log_tail(&stdout_path, 4096);
            cleanup_worker_files(&[&input_path, &output_path, &response_path, &progress_path]);
            anyhow::bail!(
                "TTS worker exited with status {}. stderr: {} stdout: {}",
                status,
                stderr,
                stdout
            );
        }

        let audio = std::fs::read(&output_path)?;
        let response: TtsWorkerResponse = serde_json::from_slice(&std::fs::read(&response_path)?)?;
        cleanup_worker_files(&[
            &input_path,
            &output_path,
            &response_path,
            &progress_path,
            &stdout_path,
            &stderr_path,
        ]);
        Ok((audio, response))
    }

    async fn speak_and_convert(
        &self,
        raw_text: &str,
        voice_override: Option<String>,
        prompt_override: Option<String>,
    ) -> Result<Vec<u8>> {
        let text = Self::bounded_text(raw_text);

        log::info!("Synthesizing long text through abstracted TTS library...");

        let mut engine_guard = self.engine.lock().await;
        if engine_guard.is_none() {
            log::info!("[TTS] Lazy-loading engine on first synthesis request...");
            let engine = EngineFactory::create(&self.config)?;
            engine_guard.replace(engine);
        }
        let engine = engine_guard
            .as_mut()
            .expect("TTS engine is initialized above");

        // If specific custom voices are requested outside of the default config, build dynamic cache prompt here
        if voice_override.is_some() || prompt_override.is_some() {
            let override_prompt = tts::VoicePrompt {
                text: prompt_override,
                wav_path: voice_override,
            };
            if let Err(e) = engine.cache_voice_prompt(&override_prompt) {
                log::warn!("Failed to apply temporary voice override cache: {}", e);
            }
        }

        let pcm_samples_result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            engine.synthesize_long_text(&text),
        )
        .await;

        let pcm_samples = match pcm_samples_result {
            Ok(res) => res?,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "TTS generation timed out after 60 seconds (preventing memory leak)"
                ))
            }
        };

        // Encode PCM float samples into standard 16-bit WAV
        let wav_bytes = self.create_wav_bytes(&pcm_samples, engine.sample_rate() as u32)?;
        Ok(wav_bytes)
    }

    async fn speak_and_encode_mp3(
        &self,
        raw_text: &str,
        voice_override: Option<String>,
        prompt_override: Option<String>,
    ) -> Result<(Vec<u8>, i64)> {
        let text = Self::bounded_text(raw_text);
        log::info!("Synthesizing long text directly into MP3...");

        tokio::time::timeout(Duration::from_secs(60), async {
            let mut engine_guard = self.engine.lock().await;
            if engine_guard.is_none() {
                log::info!("[TTS] Lazy-loading engine on first synthesis request...");
                let engine = EngineFactory::create(&self.config)?;
                engine_guard.replace(engine);
            }
            let engine = engine_guard
                .as_mut()
                .expect("TTS engine is initialized above");

            if voice_override.is_some() || prompt_override.is_some() {
                let override_prompt = tts::VoicePrompt {
                    text: prompt_override,
                    wav_path: voice_override,
                };
                if let Err(e) = engine.cache_voice_prompt(&override_prompt) {
                    log::warn!("Failed to apply temporary voice override cache: {}", e);
                }
            }

            let sample_rate = engine.sample_rate() as u32;
            let mut mp3_process = tts::Mp3StreamProcess::start_s16le(sample_rate)?;
            let chunks = tts::chunk_text(&text);
            let mut pcm_stream = tts::PcmStreamEncoder::new(sample_rate as usize, 0.05);

            for (idx, chunk) in chunks.iter().enumerate() {
                if chunk.trim().is_empty() {
                    continue;
                }
                log::info!(
                    "Generating MP3 audio chunk {}/{} ({} chars)...",
                    idx + 1,
                    chunks.len(),
                    chunk.chars().count()
                );

                let samples = engine.synthesize_chunk(chunk)?;
                let bytes = pcm_stream.push(&samples);
                if !bytes.is_empty() {
                    mp3_process.write_all(&bytes).await?;
                }
            }

            let tail = pcm_stream.finish();
            if !tail.is_empty() {
                mp3_process.write_all(&tail).await?;
            }

            let mp3_bytes = mp3_process.finish().await?;
            let duration = (pcm_stream.output_samples() as f64 / sample_rate as f64).ceil() as i64;
            Ok((mp3_bytes, duration))
        })
        .await
        .map_err(|_| anyhow::anyhow!("TTS MP3 generation timed out after 60 seconds"))?
    }

    /// Helper: Convert WAV bytes to MP3 using TTS library
    pub async fn convert_to_mp3(&self, wav_bytes: &[u8]) -> Result<Vec<u8>> {
        tts::convert_to_mp3(wav_bytes).await
    }

    fn bounded_text(raw_text: &str) -> String {
        if raw_text.chars().count() > MAX_TOTAL_CHARS {
            log::warn!(
                "[TTS] Text too long ({} chars > {} limit), truncating",
                raw_text.chars().count(),
                MAX_TOTAL_CHARS
            );
            let truncated: String = raw_text.chars().take(MAX_TOTAL_CHARS - 10).collect();
            format!("{}……（内容过长，已截断）", truncated)
        } else {
            raw_text.to_string()
        }
    }

    fn create_wav_bytes(&self, data: &[f32], sample_rate: u32) -> Result<Vec<u8>> {
        create_wav_bytes(data, sample_rate)
    }
}

pub async fn run_tts_worker(
    input_path: &Path,
    output_path: &Path,
    response_path: &Path,
    progress_path: Option<&Path>,
) -> Result<()> {
    let _ = env_logger::builder().is_test(false).try_init();
    if let Some(path) = progress_path {
        write_worker_progress(path, "starting")?;
    }
    let request: TtsWorkerRequest = serde_json::from_slice(&std::fs::read(input_path)?)?;
    let mut engine = EngineFactory::create(&request.config)?;
    if let Some(path) = progress_path {
        write_worker_progress(path, "engine_loaded")?;
    }

    if request.voice_override.is_some() || request.prompt_override.is_some() {
        let override_prompt = tts::VoicePrompt {
            text: request.prompt_override.clone(),
            wav_path: request.voice_override.clone(),
        };
        if let Err(e) = engine.cache_voice_prompt(&override_prompt) {
            log::warn!("Failed to apply worker voice override cache: {}", e);
        }
    }

    match request.mode {
        TtsWorkerMode::Wav => {
            let text = request
                .text
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("TTS worker missing text input"))?;
            let chunks = split_tts_text_for_config(text, &request.config);
            let samples = synthesize_chunks_to_pcm(engine.as_mut(), &chunks, progress_path).await?;
            let sample_rate = engine.sample_rate();
            let wav = create_wav_bytes(&samples, sample_rate as u32)?;
            std::fs::write(output_path, wav)?;
            write_worker_response(
                response_path,
                TtsWorkerResponse {
                    sample_rate,
                    duration_seconds: Some(
                        (samples.len() as f64 / sample_rate as f64).ceil() as i64
                    ),
                },
            )?;
        }
        TtsWorkerMode::Mp3 => {
            let text = request
                .text
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("TTS worker missing text input"))?;
            let chunks = split_tts_text_for_config(text, &request.config);
            let (mp3, duration) =
                synthesize_chunks_to_mp3(engine.as_mut(), &chunks, progress_path).await?;
            std::fs::write(output_path, mp3)?;
            write_worker_response(
                response_path,
                TtsWorkerResponse {
                    sample_rate: engine.sample_rate(),
                    duration_seconds: Some(duration),
                },
            )?;
        }
        TtsWorkerMode::Mp3Chunks => {
            let chunks = request
                .chunks
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("TTS worker missing chunk input"))?;
            let chunks = split_tts_safe_chunks_for_config(chunks, &request.config);
            let (mp3, duration) =
                synthesize_chunks_to_mp3(engine.as_mut(), &chunks, progress_path).await?;
            std::fs::write(output_path, mp3)?;
            write_worker_response(
                response_path,
                TtsWorkerResponse {
                    sample_rate: engine.sample_rate(),
                    duration_seconds: Some(duration),
                },
            )?;
        }
    }

    relieve_allocator_pressure();
    if let Some(path) = progress_path {
        write_worker_progress(path, "finished")?;
    }
    Ok(())
}

pub async fn run_tts_asr_loop_synthesize(
    config_path: &Path,
    output_dir: &Path,
    text_path: &Path,
) -> Result<()> {
    let _ = env_logger::builder().is_test(false).try_init();

    std::fs::create_dir_all(output_dir)?;
    let config_path_str = config_path.to_str().ok_or_else(|| {
        anyhow::anyhow!("config path is not valid UTF-8: {}", config_path.display())
    })?;
    let config = load_config(config_path_str)?;
    let client = TtsClient::new(config.tts.clone());
    let source_text = std::fs::read_to_string(text_path)?;
    let chunks = prepare_tts_asr_loop_chunks(&source_text, &client.config);
    if chunks.is_empty() {
        anyhow::bail!("TTS ASR loop text produced no chunks");
    }

    let mut engine = EngineFactory::create(&client.config)?;
    let sample_rate = engine.sample_rate();
    let mut manifest_chunks = Vec::with_capacity(chunks.len());

    for (idx, chunk) in chunks.iter().enumerate() {
        let chunk_index = idx + 1;
        log::info!(
            "[TTS ASR Loop] Generating chunk {}/{} ({} chars)",
            chunk_index,
            chunks.len(),
            chunk.chars().count()
        );
        let samples = synthesize_validated_chunk_with_heartbeat(
            engine.as_mut(),
            chunk,
            None,
            chunk_index,
            chunks.len(),
        )?;
        let stats = validate_tts_chunk_audio(chunk, &samples, sample_rate)
            .map_err(|failure| anyhow::anyhow!("validated chunk failed on recheck: {}", failure))?;
        let wav_file = format!("chunk-{chunk_index:03}.wav");
        let wav_path = output_dir.join(&wav_file);
        std::fs::write(&wav_path, create_wav_bytes(&samples, sample_rate as u32)?)?;

        manifest_chunks.push(TtsAsrLoopChunkManifest {
            index: chunk_index,
            text: chunk.clone(),
            chars: chunk.chars().count(),
            wav_file,
            wav_path: wav_path.to_string_lossy().into_owned(),
            duration_seconds: stats.duration_secs,
            audio_stats: stats,
        });
    }

    let manifest = TtsAsrLoopManifest {
        generated_at: chrono::Local::now().to_rfc3339(),
        config_path: config_path.to_string_lossy().into_owned(),
        text_path: text_path.to_string_lossy().into_owned(),
        engine: client.config.engine.clone(),
        device: client.config.device.clone(),
        prompt_text: tts_prompt_text(&client.config),
        prompt_wav_path: tts_prompt_wav_path(&client.config),
        sample_rate,
        source_chars: source_text.chars().count(),
        chunk_count: manifest_chunks.len(),
        chunks: manifest_chunks,
    };
    let manifest_path = output_dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    log::info!(
        "[TTS ASR Loop] Wrote {} chunks and manifest to {}",
        manifest.chunk_count,
        output_dir.display()
    );
    relieve_allocator_pressure();
    Ok(())
}

async fn synthesize_chunks_to_pcm(
    engine: &mut dyn TtsEngine,
    chunks: &[String],
    progress_path: Option<&Path>,
) -> Result<Vec<f32>> {
    let mut all_samples = Vec::new();
    let mut generated_any = false;

    for (idx, chunk) in chunks.iter().enumerate() {
        if chunk.trim().is_empty() {
            continue;
        }
        write_worker_chunk_progress(progress_path, idx + 1, chunks.len())?;
        log::info!(
            "[TTS Worker] Generating WAV audio chunk {}/{} ({} chars)",
            idx + 1,
            chunks.len(),
            chunk.chars().count()
        );
        let samples = synthesize_validated_chunk_with_heartbeat(
            engine,
            chunk,
            progress_path,
            idx + 1,
            chunks.len(),
        )?;
        generated_any = true;
        tts::append_with_crossfade(&mut all_samples, &samples, engine.sample_rate(), 0.05);
    }

    if !generated_any {
        anyhow::bail!("TTS worker produced no audio samples");
    }

    Ok(all_samples)
}

async fn synthesize_chunks_to_mp3(
    engine: &mut dyn TtsEngine,
    chunks: &[String],
    progress_path: Option<&Path>,
) -> Result<(Vec<u8>, i64)> {
    let sample_rate = engine.sample_rate() as u32;
    let mut mp3_process = tts::Mp3StreamProcess::start_s16le(sample_rate)?;
    let mut pcm_stream = tts::PcmStreamEncoder::new(sample_rate as usize, 0.05);
    let mut generated_any = false;

    for (idx, chunk) in chunks.iter().enumerate() {
        if chunk.trim().is_empty() {
            continue;
        }
        write_worker_chunk_progress(progress_path, idx + 1, chunks.len())?;
        log::info!(
            "[TTS Worker] Generating MP3 audio chunk {}/{} ({} chars)",
            idx + 1,
            chunks.len(),
            chunk.chars().count()
        );
        let samples = synthesize_validated_chunk_with_heartbeat(
            engine,
            chunk,
            progress_path,
            idx + 1,
            chunks.len(),
        )?;
        generated_any = true;
        let bytes = pcm_stream.push(&samples);
        if !bytes.is_empty() {
            mp3_process.write_all(&bytes).await?;
        }
    }

    if !generated_any {
        anyhow::bail!("TTS worker produced no audio samples");
    }

    let tail = pcm_stream.finish();
    if !tail.is_empty() {
        mp3_process.write_all(&tail).await?;
    }

    let mp3_bytes = mp3_process.finish().await?;
    let duration = (pcm_stream.output_samples() as f64 / sample_rate as f64).ceil() as i64;
    Ok((mp3_bytes, duration))
}

#[derive(Debug, Clone, Copy, Serialize)]
struct TtsChunkAudioStats {
    samples: usize,
    duration_secs: f64,
    rms: f32,
    peak: f32,
    clipped_ratio: f64,
    non_finite: usize,
    zero_crossing_rate: f64,
    high_freq_energy_ratio: f64,
    noisy_frame_ratio: f64,
    frame_rms_cv: f64,
}

#[derive(Debug, Serialize)]
struct TtsAsrLoopManifest {
    generated_at: String,
    config_path: String,
    text_path: String,
    engine: Option<String>,
    device: Option<String>,
    prompt_text: Option<String>,
    prompt_wav_path: Option<String>,
    sample_rate: usize,
    source_chars: usize,
    chunk_count: usize,
    chunks: Vec<TtsAsrLoopChunkManifest>,
}

#[derive(Debug, Serialize)]
struct TtsAsrLoopChunkManifest {
    index: usize,
    text: String,
    chars: usize,
    wav_file: String,
    wav_path: String,
    duration_seconds: f64,
    audio_stats: TtsChunkAudioStats,
}

impl TtsChunkAudioStats {
    fn log_summary(&self) -> String {
        format!(
            "samples={}, duration={:.2}s, rms={:.5}, peak={:.3}, clipped={:.2}%, non_finite={}, zcr={:.3}, hf_ratio={:.3}, noisy_frames={:.2}%, frame_rms_cv={:.3}",
            self.samples,
            self.duration_secs,
            self.rms,
            self.peak,
            self.clipped_ratio * 100.0,
            self.non_finite,
            self.zero_crossing_rate,
            self.high_freq_energy_ratio,
            self.noisy_frame_ratio * 100.0,
            self.frame_rms_cv
        )
    }
}

#[derive(Debug)]
struct TtsChunkQualityFailure {
    reason: String,
    stats: TtsChunkAudioStats,
}

impl std::fmt::Display for TtsChunkQualityFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.reason, self.stats.log_summary())
    }
}

fn validate_tts_chunk_audio(
    chunk: &str,
    samples: &[f32],
    sample_rate: usize,
) -> std::result::Result<TtsChunkAudioStats, TtsChunkQualityFailure> {
    let stats = analyze_tts_chunk_audio(samples, sample_rate);
    let failure = |reason: String| TtsChunkQualityFailure { reason, stats };

    if sample_rate == 0 {
        return Err(failure("sample rate is zero".to_string()));
    }
    if samples.is_empty() {
        return Err(failure("empty audio".to_string()));
    }
    if stats.non_finite > 0 {
        return Err(failure(format!(
            "contains {} non-finite samples",
            stats.non_finite
        )));
    }

    let chars = effective_tts_chunk_chars(chunk);
    let min_duration = expected_min_chunk_duration(chars);
    let max_duration = expected_max_chunk_duration(chars);

    if stats.duration_secs < min_duration {
        return Err(failure(format!(
            "too short for {} chars: {:.2}s < {:.2}s",
            chars, stats.duration_secs, min_duration
        )));
    }
    if stats.duration_secs > max_duration {
        return Err(failure(format!(
            "too long for {} chars: {:.2}s > {:.2}s",
            chars, stats.duration_secs, max_duration
        )));
    }
    if stats.rms < TTS_CHUNK_MIN_RMS || stats.peak < TTS_CHUNK_MIN_PEAK {
        return Err(failure(format!(
            "near-silent audio: rms {:.5}, peak {:.3}",
            stats.rms, stats.peak
        )));
    }
    if stats.peak > TTS_CHUNK_HARD_PEAK_LIMIT {
        return Err(failure(format!(
            "extreme sample peak {:.3} > {:.3}",
            stats.peak, TTS_CHUNK_HARD_PEAK_LIMIT
        )));
    }
    if stats.clipped_ratio > TTS_CHUNK_MAX_CLIPPED_RATIO {
        return Err(failure(format!(
            "too much clipping: {:.2}% > {:.2}%",
            stats.clipped_ratio * 100.0,
            TTS_CHUNK_MAX_CLIPPED_RATIO * 100.0
        )));
    }
    if stats.noisy_frame_ratio > TTS_CHUNK_MAX_NOISY_FRAME_RATIO {
        return Err(failure(format!(
            "sustained broadband noise: noisy frames {:.2}% > {:.2}%",
            stats.noisy_frame_ratio * 100.0,
            TTS_CHUNK_MAX_NOISY_FRAME_RATIO * 100.0
        )));
    }
    if stats.zero_crossing_rate > TTS_CHUNK_MAX_ZERO_CROSSING_RATE
        && stats.high_freq_energy_ratio > TTS_CHUNK_MAX_HIGH_FREQ_ENERGY_RATIO
    {
        return Err(failure(format!(
            "broadband/noisy synthesis: zcr {:.3} > {:.3} and hf_ratio {:.3} > {:.3}",
            stats.zero_crossing_rate,
            TTS_CHUNK_MAX_ZERO_CROSSING_RATE,
            stats.high_freq_energy_ratio,
            TTS_CHUNK_MAX_HIGH_FREQ_ENERGY_RATIO
        )));
    }

    Ok(stats)
}

fn analyze_tts_chunk_audio(samples: &[f32], sample_rate: usize) -> TtsChunkAudioStats {
    let duration_secs = if sample_rate == 0 {
        0.0
    } else {
        samples.len() as f64 / sample_rate as f64
    };
    let mut finite_count = 0_usize;
    let mut non_finite = 0_usize;
    let mut sum_squares = 0.0_f64;
    let mut peak = 0.0_f32;
    let mut clipped = 0_usize;
    let mut zero_crossings = 0_usize;
    let mut finite_pairs = 0_usize;
    let mut diff_squares = 0.0_f64;
    let mut prev_finite_sample = None;

    for &sample in samples {
        if !sample.is_finite() {
            non_finite += 1;
            prev_finite_sample = None;
            continue;
        }

        finite_count += 1;
        let abs = sample.abs();
        peak = peak.max(abs);
        sum_squares += (sample as f64) * (sample as f64);
        if abs >= 0.999 {
            clipped += 1;
        }
        if let Some(prev) = prev_finite_sample {
            if (prev >= 0.0) != (sample >= 0.0) {
                zero_crossings += 1;
            }
            let diff = sample - prev;
            diff_squares += (diff as f64) * (diff as f64);
            finite_pairs += 1;
        }
        prev_finite_sample = Some(sample);
    }

    let rms = if finite_count == 0 {
        0.0
    } else {
        (sum_squares / finite_count as f64).sqrt() as f32
    };
    let clipped_ratio = if finite_count == 0 {
        0.0
    } else {
        clipped as f64 / finite_count as f64
    };
    let zero_crossing_rate = if finite_pairs == 0 {
        0.0
    } else {
        zero_crossings as f64 / finite_pairs as f64
    };
    let high_freq_energy_ratio = if finite_pairs == 0 || sum_squares <= f64::EPSILON {
        0.0
    } else {
        (diff_squares / finite_pairs as f64) / (sum_squares / finite_count.max(1) as f64)
    };
    let frame_stats = analyze_tts_chunk_frames(samples, sample_rate);

    TtsChunkAudioStats {
        samples: samples.len(),
        duration_secs,
        rms,
        peak,
        clipped_ratio,
        non_finite,
        zero_crossing_rate,
        high_freq_energy_ratio,
        noisy_frame_ratio: frame_stats.noisy_frame_ratio,
        frame_rms_cv: frame_stats.frame_rms_cv,
    }
}

#[derive(Debug, Clone, Copy)]
struct TtsChunkFrameStats {
    noisy_frame_ratio: f64,
    frame_rms_cv: f64,
}

fn analyze_tts_chunk_frames(samples: &[f32], sample_rate: usize) -> TtsChunkFrameStats {
    let frame_len = (sample_rate / 50).max(160);
    if frame_len == 0 || samples.len() < frame_len {
        return TtsChunkFrameStats {
            noisy_frame_ratio: 0.0,
            frame_rms_cv: 0.0,
        };
    }

    let mut voiced_frames = 0_usize;
    let mut noisy_frames = 0_usize;
    let mut frame_rms_values = Vec::new();

    for frame in samples.chunks(frame_len) {
        if frame.len() < frame_len / 2 {
            continue;
        }

        let mut finite_count = 0_usize;
        let mut sum_squares = 0.0_f64;
        let mut diff_squares = 0.0_f64;
        let mut zero_crossings = 0_usize;
        let mut pairs = 0_usize;
        let mut prev_finite_sample = None;

        for &sample in frame {
            if !sample.is_finite() {
                prev_finite_sample = None;
                continue;
            }

            finite_count += 1;
            sum_squares += (sample as f64) * (sample as f64);
            if let Some(prev) = prev_finite_sample {
                if (prev >= 0.0) != (sample >= 0.0) {
                    zero_crossings += 1;
                }
                let diff = sample - prev;
                diff_squares += (diff as f64) * (diff as f64);
                pairs += 1;
            }
            prev_finite_sample = Some(sample);
        }

        if finite_count == 0 || pairs == 0 {
            continue;
        }

        let frame_energy = sum_squares / finite_count as f64;
        let frame_rms = frame_energy.sqrt();
        if frame_rms < TTS_CHUNK_MIN_RMS as f64 {
            continue;
        }

        voiced_frames += 1;
        frame_rms_values.push(frame_rms);
        let frame_zcr = zero_crossings as f64 / pairs as f64;
        let frame_hf_ratio = (diff_squares / pairs as f64) / frame_energy.max(f64::EPSILON);
        if frame_zcr > TTS_NOISY_FRAME_ZERO_CROSSING_RATE
            && frame_hf_ratio > TTS_NOISY_FRAME_HIGH_FREQ_ENERGY_RATIO
        {
            noisy_frames += 1;
        }
    }

    let noisy_frame_ratio = if voiced_frames == 0 {
        0.0
    } else {
        noisy_frames as f64 / voiced_frames as f64
    };
    let frame_rms_cv = coefficient_of_variation(&frame_rms_values);

    TtsChunkFrameStats {
        noisy_frame_ratio,
        frame_rms_cv,
    }
}

fn coefficient_of_variation(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if mean <= f64::EPSILON {
        return 0.0;
    }
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt() / mean
}

fn effective_tts_chunk_chars(chunk: &str) -> usize {
    chunk.chars().filter(|c| !c.is_whitespace()).count()
}

fn expected_min_chunk_duration(chars: usize) -> f64 {
    if chars <= 12 {
        0.35
    } else {
        (chars as f64 / 12.0).max(0.45)
    }
}

fn expected_max_chunk_duration(chars: usize) -> f64 {
    (chars as f64 / 3.0 + 6.0).max(8.0)
}

fn synthesize_validated_chunk_with_heartbeat(
    engine: &mut dyn TtsEngine,
    chunk: &str,
    progress_path: Option<&Path>,
    chunk_idx: usize,
    total_chunks: usize,
) -> Result<Vec<f32>> {
    let sample_rate = engine.sample_rate();
    let mut last_failure = None;

    for attempt in 1..=TTS_CHUNK_MAX_ATTEMPTS {
        if attempt > 1 {
            if let Some(path) = progress_path {
                let _ = write_worker_progress(
                    path,
                    &format!("chunk={chunk_idx}/{total_chunks} retry={attempt}"),
                );
            }
            log::warn!(
                "[TTS Worker] Retrying chunk {}/{} audio generation, attempt {}/{}",
                chunk_idx,
                total_chunks,
                attempt,
                TTS_CHUNK_MAX_ATTEMPTS
            );
        }

        let samples = match synthesize_chunk_with_heartbeat(
            engine,
            chunk,
            progress_path,
            chunk_idx,
            total_chunks,
        ) {
            Ok(samples) => samples,
            Err(e) => {
                let reason = format!("synthesis error: {}", e);
                log::warn!(
                    "[TTS Worker] Chunk {}/{} attempt {}/{} failed: {}",
                    chunk_idx,
                    total_chunks,
                    attempt,
                    TTS_CHUNK_MAX_ATTEMPTS,
                    reason
                );
                last_failure = Some(reason);
                continue;
            }
        };

        match validate_tts_chunk_audio(chunk, &samples, sample_rate) {
            Ok(stats) => {
                if attempt > 1 {
                    log::info!(
                        "[TTS Worker] Chunk {}/{} passed audio quality after retry: {}",
                        chunk_idx,
                        total_chunks,
                        stats.log_summary()
                    );
                } else {
                    log::info!(
                        "[TTS Worker] Chunk {}/{} audio quality: {}",
                        chunk_idx,
                        total_chunks,
                        stats.log_summary()
                    );
                }
                return Ok(samples);
            }
            Err(failure) => {
                let reason = failure.to_string();
                log::warn!(
                    "[TTS Worker] Chunk {}/{} attempt {}/{} failed audio quality gate: {}",
                    chunk_idx,
                    total_chunks,
                    attempt,
                    TTS_CHUNK_MAX_ATTEMPTS,
                    reason
                );
                last_failure = Some(reason);
            }
        }
    }

    anyhow::bail!(
        "TTS chunk {}/{} failed audio quality gate after {} attempts: {}",
        chunk_idx,
        total_chunks,
        TTS_CHUNK_MAX_ATTEMPTS,
        last_failure.unwrap_or_else(|| "unknown failure".to_string())
    )
}

fn synthesize_chunk_with_heartbeat(
    engine: &mut dyn TtsEngine,
    chunk: &str,
    progress_path: Option<&Path>,
    chunk_idx: usize,
    total_chunks: usize,
) -> Result<Vec<f32>> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let heartbeat = progress_path.map(|path| {
        let path = path.to_path_buf();
        std::thread::spawn(move || loop {
            match stop_rx.recv_timeout(Duration::from_secs(30)) {
                Ok(_) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    let _ = write_worker_progress(
                        &path,
                        &format!("chunk={chunk_idx}/{total_chunks} running"),
                    );
                }
            }
        })
    });

    let result = engine.synthesize_chunk(chunk);
    let _ = stop_tx.send(());

    if let Some(handle) = heartbeat {
        let _ = handle.join();
    }

    result
}

fn write_worker_response(path: &Path, response: TtsWorkerResponse) -> Result<()> {
    std::fs::write(path, serde_json::to_vec(&response)?)?;
    Ok(())
}

fn create_wav_bytes(data: &[f32], sample_rate: u32) -> Result<Vec<u8>> {
    use std::io::Cursor;
    let mut cursor = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
    for &sample in data {
        let sample_i16 = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        writer.write_sample(sample_i16)?;
    }
    writer.finalize()?;
    Ok(cursor.into_inner())
}

#[cfg(target_os = "macos")]
fn relieve_allocator_pressure() {
    use std::ffi::c_void;

    extern "C" {
        fn malloc_zone_pressure_relief(zone: *mut c_void, goal: usize) -> usize;
    }

    let released = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    if released > 0 {
        log::info!(
            "[TTS] macOS malloc pressure relief released {:.1} MB",
            released as f64 / 1024.0 / 1024.0
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn relieve_allocator_pressure() {}

fn experimental_tts_enabled() -> bool {
    let requested = std::env::var("FRESHLOOP_ALLOW_EXPERIMENTAL_TTS")
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    let installed_service = installed_cortex_service_context();
    let enabled = experimental_tts_enabled_for_context(requested, installed_service);
    if requested && !enabled {
        log::warn!("[TTS] Ignoring FRESHLOOP_ALLOW_EXPERIMENTAL_TTS for installed Cortex service");
    }
    enabled
}

fn experimental_tts_enabled_for_context(requested: bool, installed_service: bool) -> bool {
    requested && !installed_service
}

fn installed_cortex_service_context() -> bool {
    if std::env::var("XPC_SERVICE_NAME")
        .map(|value| value == "com.freshloop.cortex")
        .unwrap_or(false)
    {
        return true;
    }

    std::env::current_exe()
        .ok()
        .map(|path| {
            let path = path.to_string_lossy();
            path.ends_with("/.freshloop/bin/cortex")
        })
        .unwrap_or(false)
}

fn resolve_cortex_tts_runtime_policy<'a>(
    requested_engine: &'a str,
    requested_device: Option<&'a str>,
    allow_experimental: bool,
) -> (&'a str, &'a str) {
    let req = requested_engine.trim();
    let requested_device = requested_device
        .map(str::trim)
        .filter(|device| !device.is_empty());
    if allow_experimental {
        return (
            req,
            requested_device.unwrap_or_else(|| default_device_name_for_engine(req)),
        );
    }

    ("voxcpm_metal", "metal")
}

fn default_device_name_for_engine(engine: &str) -> &'static str {
    if engine.eq_ignore_ascii_case("voxcpm_metal") {
        "metal"
    } else {
        "cpu"
    }
}

fn production_voxcpm_max_len(configured: Option<usize>, allow_experimental: bool) -> Option<usize> {
    if allow_experimental {
        configured
    } else {
        Some(PRODUCTION_VOXCPM_MAX_LEN)
    }
}

fn production_voxcpm_inference_timesteps(
    configured: Option<usize>,
    allow_experimental: bool,
) -> Option<usize> {
    if allow_experimental {
        configured
    } else {
        Some(PRODUCTION_VOXCPM_INFERENCE_TIMESTEPS)
    }
}

fn production_voxcpm_cfg_value(configured: Option<f64>, allow_experimental: bool) -> Option<f64> {
    if allow_experimental {
        configured
    } else {
        Some(PRODUCTION_VOXCPM_CFG_VALUE)
    }
}

fn production_voxcpm_ratio_threshold(
    configured: Option<f64>,
    allow_experimental: bool,
) -> Option<f64> {
    if allow_experimental {
        configured
    } else {
        Some(PRODUCTION_VOXCPM_RATIO_THRESHOLD)
    }
}

fn production_voxcpm_control_instruction(
    configured: Option<String>,
    allow_experimental: bool,
) -> Option<String> {
    if allow_experimental {
        configured
    } else {
        None
    }
}

fn tts_prompt_text(config: &LibTtsConfig) -> Option<String> {
    match config.engine.as_deref().unwrap_or("voxcpm").trim() {
        "voxcpm" | "voxcpm_metal" => config.voxcpm.as_ref().and_then(|v| v.prompt_text.clone()),
        "qwen3" => config.qwen3.as_ref().and_then(|v| v.prompt_text.clone()),
        "magictts" => config.magictts.as_ref().and_then(|v| v.prompt_text.clone()),
        "moss" => config.moss.as_ref().and_then(|v| v.prompt_text.clone()),
        _ => None,
    }
}

fn tts_prompt_wav_path(config: &LibTtsConfig) -> Option<String> {
    match config.engine.as_deref().unwrap_or("voxcpm").trim() {
        "voxcpm" | "voxcpm_metal" => config
            .voxcpm
            .as_ref()
            .and_then(|v| v.prompt_wav_path.clone()),
        "qwen3" => config
            .qwen3
            .as_ref()
            .and_then(|v| v.prompt_wav_path.clone()),
        "magictts" => config
            .magictts
            .as_ref()
            .and_then(|v| v.prompt_wav_path.clone()),
        "moss" => config.moss.as_ref().and_then(|v| v.prompt_wav_path.clone()),
        _ => None,
    }
}

fn engine_uses_heavy_native_runtime(engine: &str) -> bool {
    matches!(
        engine.trim().to_ascii_lowercase().as_str(),
        "voxcpm" | "voxcpm_metal"
    )
}

fn push_bounded_chunk(chunks: &mut Vec<String>, text: &str, total_chars: &mut usize) -> bool {
    let split_chunks = split_tts_text_with_max_chars(text, TTS_MAX_STABLE_CHUNK_CHARS);

    for chunk in split_chunks {
        let char_count = chunk.chars().count();
        if *total_chars + char_count <= MAX_TOTAL_CHARS {
            *total_chars += char_count;
            chunks.push(chunk);
            continue;
        }

        if *total_chars >= MAX_TOTAL_CHARS {
            log::warn!(
                "[TTS] Reached {} char safety limit; dropping remaining chunks",
                MAX_TOTAL_CHARS
            );
            return false;
        }

        let remaining = MAX_TOTAL_CHARS - *total_chars;
        let keep = remaining.saturating_sub(10);
        let truncated: String = chunk.chars().take(keep).collect();
        chunks.push(format!("{}……（内容过长，已截断）", truncated));
        *total_chars = MAX_TOTAL_CHARS;
        log::warn!(
            "[TTS] Reached {} char safety limit; truncating final chunk",
            MAX_TOTAL_CHARS
        );
        return false;
    }

    true
}

fn split_tts_safe_chunks_for_config(chunks: &[String], config: &LibTtsConfig) -> Vec<String> {
    split_tts_safe_chunks(chunks, tts_stable_chunk_max_chars(config))
}

fn split_tts_safe_chunks(chunks: &[String], max_chars: usize) -> Vec<String> {
    let split = chunks
        .iter()
        .flat_map(|chunk| split_tts_text_with_max_chars(chunk, max_chars))
        .filter(|chunk| !chunk.trim().is_empty())
        .collect::<Vec<_>>();
    stabilize_tts_chunks(split, max_chars)
}

fn split_tts_text_for_config(text: &str, config: &LibTtsConfig) -> Vec<String> {
    split_tts_text_with_max_chars(text, tts_stable_chunk_max_chars(config))
}

fn prepare_tts_asr_loop_chunks(raw_text: &str, config: &LibTtsConfig) -> Vec<String> {
    let bounded = TtsClient::bounded_text(raw_text);
    split_tts_safe_chunks_for_config(&[bounded], config)
}

fn stabilize_tts_chunks(chunks: Vec<String>, max_chars: usize) -> Vec<String> {
    let mut stable = Vec::new();
    let mut current = String::new();
    let max_chars = max_chars.max(1);

    for chunk in chunks {
        let chunk = chunk.trim().to_string();
        if chunk.is_empty() {
            continue;
        }

        if current.is_empty() {
            current = chunk;
            continue;
        }

        let current_chars = current.chars().count();
        let chunk_chars = chunk.chars().count();
        let combined_chars = current_chars + chunk_chars;
        let should_merge = combined_chars <= max_chars;

        if should_merge {
            current.push('\n');
            current.push_str(&chunk);
        } else {
            stable.push(current);
            current = chunk;
        }
    }

    if !current.is_empty() {
        stable.push(current);
    }

    stable
}

fn split_tts_text_with_max_chars(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let chunks = tts::chunk_text(text);
    if chunks.is_empty() {
        split_tts_chunk_by_max_chars(text, max_chars)
    } else {
        chunks
            .into_iter()
            .flat_map(|chunk| split_tts_chunk_by_max_chars(&chunk, max_chars))
            .collect()
    }
}

fn split_tts_chunk_by_max_chars(chunk: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    if chunk.chars().count() <= max_chars {
        return vec![chunk.trim().to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in chunk.chars() {
        current.push(ch);
        if current.chars().count() >= max_chars {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                chunks.push(trimmed.to_string());
            }
            current.clear();
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        chunks.push(trimmed.to_string());
    }
    chunks
}

fn tts_stable_chunk_max_chars(config: &LibTtsConfig) -> usize {
    match config.engine.as_deref().unwrap_or("voxcpm").trim() {
        "moss" => config
            .moss
            .as_ref()
            .and_then(|m| m.chunk_max_chars)
            .unwrap_or(MOSS_TTS_MAX_STABLE_CHUNK_CHARS)
            .clamp(60, TTS_MAX_STABLE_CHUNK_CHARS),
        _ => TTS_MAX_STABLE_CHUNK_CHARS,
    }
}

fn tts_worker_temp_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".freshloop")
        .join("cache")
        .join("tts_worker")
}

fn tts_worker_semaphore() -> &'static Semaphore {
    static SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();
    SEMAPHORE.get_or_init(|| Semaphore::new(1))
}

async fn wait_for_worker_with_limits(
    child: &mut tokio::process::Child,
    progress_path: &Path,
    memory_limit_mb: u64,
    idle_timeout: Duration,
) -> Result<ExitStatus> {
    let mut last_progress = Instant::now();
    let mut last_progress_mtime = progress_mtime(progress_path);

    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }

        let current_mtime = progress_mtime(progress_path);
        if current_mtime.is_some() && current_mtime != last_progress_mtime {
            last_progress_mtime = current_mtime;
            last_progress = Instant::now();
        }

        if last_progress.elapsed() > idle_timeout {
            let _ = child.kill().await;
            anyhow::bail!(
                "TTS worker made no progress for {} seconds",
                idle_timeout.as_secs()
            );
        }

        if let Some(child_id) = child.id() {
            let memory_mb = process_memory_mb(child_id);
            if memory_mb > memory_limit_mb {
                let _ = child.kill().await;
                anyhow::bail!(
                    "TTS worker exceeded memory limit: {} MB > {} MB",
                    memory_mb,
                    memory_limit_mb
                );
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn progress_mtime(path: &Path) -> Option<std::time::SystemTime> {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn write_worker_chunk_progress(path: Option<&Path>, chunk: usize, total: usize) -> Result<()> {
    if let Some(path) = path {
        write_worker_progress(path, &format!("chunk={chunk}/{total}"))?;
    }
    Ok(())
}

fn write_worker_progress(path: &Path, status: &str) -> Result<()> {
    let now = chrono::Local::now().to_rfc3339();
    std::fs::write(path, format!("{now} {status}\n"))?;
    Ok(())
}

fn process_memory_mb(pid: u32) -> u64 {
    let mut system = System::new_all();
    system.refresh_all();
    system
        .process(Pid::from_u32(pid))
        .map(|process| process.memory() / 1024 / 1024)
        .unwrap_or(0)
}

fn read_log_tail(path: &Path, max_bytes: usize) -> String {
    std::fs::read(path)
        .map(|bytes| {
            let len = bytes.len();
            let start = len.saturating_sub(max_bytes);
            String::from_utf8_lossy(&bytes[start..]).into_owned()
        })
        .unwrap_or_else(|_| "Failed to read log".to_string())
}

fn cleanup_worker_files(paths: &[&Path]) {
    for path in paths {
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tts_config(engine: &str, process_isolation: Option<bool>) -> CortexTtsConfig {
        tts_config_with_device(engine, None, process_isolation)
    }

    fn tts_config_with_device(
        engine: &str,
        device: Option<&str>,
        process_isolation: Option<bool>,
    ) -> CortexTtsConfig {
        CortexTtsConfig {
            engine: Some(engine.to_string()),
            device: device.map(str::to_string),
            keep_engine_loaded: None,
            memory_pressure_relief: None,
            process_isolation,
            worker_memory_limit_mb: None,
            worker_idle_timeout_secs: None,
            worker_timeout_secs: None,
            voxcpm: None,
            qwen3: None,
            magictts: None,
            moss: None,
        }
    }

    fn voxcpm_lib_config() -> LibTtsConfig {
        LibTtsConfig {
            engine: Some("voxcpm_metal".to_string()),
            device: Some("metal".to_string()),
            voxcpm: None,
            qwen3: None,
            magictts: None,
            moss: None,
        }
    }

    #[test]
    fn voxcpm_uses_worker_isolation_by_default() {
        let client = TtsClient::new(tts_config("voxcpm", None));
        assert!(client.process_isolation);
    }

    #[test]
    fn cortex_tts_policy_forces_worker_isolation_by_default() {
        let client = TtsClient::new(tts_config("voxcpm", Some(false)));

        assert!(client.process_isolation);
    }

    #[test]
    fn cortex_tts_policy_uses_metal_worker_by_default() {
        let client = TtsClient::new(tts_config("voxcpm", None));

        assert_eq!(client.config.engine.as_deref(), Some("voxcpm_metal"));
        assert_eq!(client.config.device.as_deref(), Some("metal"));
        assert!(client.process_isolation);
    }

    #[test]
    fn cortex_tts_policy_ignores_cpu_device_in_production() {
        let client = TtsClient::new(tts_config_with_device("voxcpm", Some("cpu"), Some(false)));

        assert_eq!(client.config.engine.as_deref(), Some("voxcpm_metal"));
        assert_eq!(client.config.device.as_deref(), Some("metal"));
        assert!(client.process_isolation);
    }

    #[test]
    fn cortex_tts_policy_falls_back_from_experimental_engines() {
        let client = TtsClient::new(tts_config("qwen3", None));

        assert_eq!(client.config.engine.as_deref(), Some("voxcpm_metal"));
        assert_eq!(client.config.device.as_deref(), Some("metal"));
        assert!(client.process_isolation);
    }

    #[test]
    fn cortex_tts_policy_falls_back_from_moss() {
        let client = TtsClient::new(tts_config("moss", Some(false)));

        assert_eq!(client.config.engine.as_deref(), Some("voxcpm_metal"));
        assert_eq!(client.config.device.as_deref(), Some("metal"));
        assert!(client.process_isolation);
    }

    #[test]
    fn cortex_tts_policy_allows_experimental_engines_only_when_explicit() {
        let (engine, device) = resolve_cortex_tts_runtime_policy("qwen3", Some("metal"), true);

        assert_eq!(engine, "qwen3");
        assert_eq!(device, "metal");
    }

    #[test]
    fn installed_service_context_ignores_experimental_env_flag() {
        assert!(!experimental_tts_enabled_for_context(true, true));
        assert!(experimental_tts_enabled_for_context(true, false));
        assert!(!experimental_tts_enabled_for_context(false, false));
    }

    #[test]
    fn cortex_tts_policy_keeps_metal_acceleration_isolated() {
        let (engine, device) =
            resolve_cortex_tts_runtime_policy("voxcpm_metal", Some("metal"), false);

        assert_eq!(engine, "voxcpm_metal");
        assert_eq!(device, "metal");
    }

    #[test]
    fn cortex_tts_policy_restores_voxcpm_quality_defaults_in_production() {
        let mut config = tts_config("voxcpm_metal", None);
        config.voxcpm = Some(crate::core::config::VoxCPMConfig {
            model_path: "/tmp/voxcpm".to_string(),
            prompt_text: None,
            prompt_wav_path: None,
            control_instruction: Some("faster experimental delivery".to_string()),
            min_len: Some(2),
            max_len: Some(256),
            inference_timesteps: Some(6),
            cfg_value: Some(1.8),
            retry_badcase: Some(true),
            retry_badcase_ratio_threshold: Some(2.0),
        });

        let client = TtsClient::new(config);
        let voxcpm = client.config.voxcpm.as_ref().expect("voxcpm config");

        assert_eq!(voxcpm.max_len, Some(1024));
        assert_eq!(voxcpm.inference_timesteps, Some(10));
        assert_eq!(voxcpm.cfg_value, Some(2.0));
        assert_eq!(voxcpm.retry_badcase_ratio_threshold, Some(6.0));
        assert_eq!(voxcpm.control_instruction, None);
    }

    #[test]
    fn bounded_chunk_truncates_at_total_limit() {
        let mut chunks = Vec::new();
        let mut total = MAX_TOTAL_CHARS - 5;

        let keep_reading =
            push_bounded_chunk(&mut chunks, "abcdefghijklmnopqrstuvwxyz", &mut total);

        assert!(!keep_reading);
        assert_eq!(total, MAX_TOTAL_CHARS);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("已截断"));
    }

    #[test]
    fn bounded_chunk_splits_large_llm_segment_before_worker() {
        let mut chunks = Vec::new();
        let mut total = 0;
        let long_segment = "这是一段很长的语音合成输入".repeat(80);

        let keep_reading = push_bounded_chunk(&mut chunks, &long_segment, &mut total);

        assert!(keep_reading);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 220));
        assert_eq!(
            total,
            chunks
                .iter()
                .map(|chunk| chunk.chars().count())
                .sum::<usize>()
        );
    }

    #[test]
    fn worker_resplits_large_mp3_chunk_input() {
        let chunks =
            vec!["大家好，欢迎收听 FreshLoop 国际时政频道，今天我们直接看关键动向。".repeat(20)];

        let split = split_tts_safe_chunks(&chunks, TTS_MAX_STABLE_CHUNK_CHARS);

        assert!(split.len() > chunks.len());
        assert!(split.iter().all(|chunk| chunk.chars().count() <= 220));
    }

    #[test]
    fn tts_chunk_stabilizer_merges_short_neighbors() {
        let chunks = vec![
            "今天的主要新闻是，多个团队开始把研究智能体接入日常工作流，让资料整理和实验记录更连续。".to_string(),
            "系统不再只是回答一个问题，而是持续追踪来源。".to_string(),
            "接下来是技术产业，开发者工具正在从单点补全走向端到端协作。".to_string(),
        ];

        let stable = stabilize_tts_chunks(chunks, TTS_MAX_STABLE_CHUNK_CHARS);

        assert_eq!(stable.len(), 1);
        assert!(stable[0].contains("持续追踪来源"));
        assert!(stable[0].contains("技术产业"));
        assert!(stable.iter().all(|chunk| chunk.chars().count() <= 200));
    }

    #[test]
    fn tts_chunk_stabilizer_merges_safe_tail_chunk() {
        let chunks = vec![
            "这段测试的后半部分故意保持连续叙事，因为我们要捕获一种很隐蔽的问题：音频开头听起来正常，但模型在后续分块里逐渐退化，出现杂音、乱读、重复或和原文无关的声音。部署前必须把这种问题挡住，而不是上线后再让用户用耳朵发现。".to_string(),
            "如果整个闭环通过，说明当前生产配置至少能稳定读完一段较长文本，并且后半段没有明显掉线。如果失败，报告会指出具体 chunk、原文、识别文本和相似度，方便直接打开对应音频定位问题。".to_string(),
        ];

        let stable = stabilize_tts_chunks(chunks, TTS_MAX_STABLE_CHUNK_CHARS);

        assert_eq!(stable.len(), 1);
        assert!(stable[0].contains("定位问题"));
        assert!(stable[0].chars().count() <= 200);
    }

    #[test]
    fn moss_tts_policy_keeps_long_text_chunks_shorter_than_voxcpm() {
        let config = LibTtsConfig {
            engine: Some("moss".to_string()),
            device: Some("cpu".to_string()),
            voxcpm: None,
            qwen3: None,
            magictts: None,
            moss: Some(LibMossTtsConfig {
                model_dir: "/tmp/moss".to_string(),
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
                max_new_frames: Some(375),
                voice_clone_max_text_tokens: None,
                seed: Some(42),
                intra_threads: Some(4),
                inter_threads: Some(1),
                chunk_max_chars: None,
            }),
        };
        let chunks = vec![
            "国际时政方面，监管机构开始要求更清晰的模型使用披露，尤其是在新闻、教育、医疗和金融场景，系统必须说明生成内容的边界，也要保留可审计的证据链。".repeat(2),
            "最后回到产品体验，真正有用的个人信息流不应该让用户被动刷新，而应该把阅读、标注、评价和后续行动连在一起，用户可以发布自己的心得，也可以对系统推荐的内容写下个人判断。".to_string(),
        ];

        let split = split_tts_safe_chunks_for_config(&chunks, &config);

        assert!(split.len() >= 2);
        assert!(split.iter().all(|chunk| chunk.chars().count() <= 120));
    }

    #[test]
    fn tts_duration_bounds_reject_short_chunk_runaway_and_truncation() {
        let short_chunk = "系统不再只是回答一个问题，而是持续追踪来源。";
        let runaway = test_tone(24_000, 23.0);
        let truncated = test_tone(24_000, 1.0);

        let runaway_err = validate_tts_chunk_audio(short_chunk, &runaway, 24_000).unwrap_err();
        let truncated_err = validate_tts_chunk_audio(short_chunk, &truncated, 24_000).unwrap_err();

        assert!(runaway_err.to_string().contains("too long"));
        assert!(truncated_err.to_string().contains("too short"));
    }

    #[test]
    fn asr_loop_fixture_text_is_split_into_late_quality_chunks() {
        let text = "今天的闭环测试模拟一段真实的长新闻节目。我们先从人工智能行业说起，多个团队正在把研究智能体接入日常工作流，让信息筛选、资料整理和实验记录都变得更连续。接下来是技术产业，开发者工具正在从单点补全走向端到端协作，重点不再只是生成代码，而是理解上下文、保留决策记录，并在部署前主动暴露风险。再看商业财经，企业对自动化的期待已经从节省人力转向提升判断质量，管理者更关心系统能不能解释来源、能不能复盘误判、能不能持续学习偏好。国际时政方面，监管机构开始要求更清晰的模型使用披露，尤其是在新闻、教育、医疗和金融场景，系统必须说明生成内容的边界。最后回到产品体验，真正有用的个人信息流不应该让用户被动刷新，而应该把阅读、标注、评价和后续行动连在一起，形成稳定的反馈循环。".repeat(3);

        let chunks = prepare_tts_asr_loop_chunks(&text, &voxcpm_lib_config());

        assert!(chunks.len() >= 6);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 220));
        assert!(chunks.last().unwrap().contains("反馈循环"));
    }

    #[test]
    fn legacy_worker_timeout_config_is_idle_timeout_alias() {
        let mut config = tts_config("voxcpm", None);
        config.worker_timeout_secs = Some(42);

        let client = TtsClient::new(config);

        assert_eq!(client.worker_idle_timeout, Duration::from_secs(42));
    }

    struct FastEngine;

    impl TtsEngine for FastEngine {
        fn cache_voice_prompt(&mut self, _prompt: &tts::VoicePrompt) -> Result<()> {
            Ok(())
        }

        fn sample_rate(&self) -> usize {
            24_000
        }

        fn synthesize_chunk(&mut self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![0.0, 0.1, -0.1])
        }
    }

    struct ScriptedEngine {
        sample_rate: usize,
        calls: usize,
        outputs: std::collections::VecDeque<Vec<f32>>,
    }

    impl ScriptedEngine {
        fn new(outputs: Vec<Vec<f32>>) -> Self {
            Self {
                sample_rate: 24_000,
                calls: 0,
                outputs: outputs.into(),
            }
        }
    }

    impl TtsEngine for ScriptedEngine {
        fn cache_voice_prompt(&mut self, _prompt: &tts::VoicePrompt) -> Result<()> {
            Ok(())
        }

        fn sample_rate(&self) -> usize {
            self.sample_rate
        }

        fn synthesize_chunk(&mut self, _text: &str) -> Result<Vec<f32>> {
            self.calls += 1;
            self.outputs
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no scripted TTS output left"))
        }
    }

    fn test_tone(sample_rate: usize, seconds: f64) -> Vec<f32> {
        let samples = (sample_rate as f64 * seconds) as usize;
        (0..samples)
            .map(|idx| {
                let phase = idx as f32 / sample_rate as f32 * 440.0 * std::f32::consts::TAU;
                phase.sin() * 0.2
            })
            .collect()
    }

    fn broadband_noise(sample_rate: usize, seconds: f64) -> Vec<f32> {
        let samples = (sample_rate as f64 * seconds) as usize;
        let mut state = 0x1234_5678_u32;
        (0..samples)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let unit = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                unit * 0.2
            })
            .collect()
    }

    #[test]
    fn heartbeat_thread_does_not_hold_finished_chunks_for_interval() {
        let progress_path =
            std::env::temp_dir().join(format!("freshloop-tts-progress-{}.txt", Uuid::new_v4()));
        std::fs::write(&progress_path, "starting").unwrap();
        let mut engine = FastEngine;

        let start = Instant::now();
        let samples =
            synthesize_chunk_with_heartbeat(&mut engine, "hello", Some(&progress_path), 1, 1)
                .unwrap();

        let _ = std::fs::remove_file(progress_path);
        assert_eq!(samples.len(), 3);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "heartbeat join should not wait for the 30s heartbeat interval"
        );
    }

    #[tokio::test]
    async fn worker_retries_bad_chunk_audio_before_appending() {
        let bad_samples = vec![f32::NAN; 1_000];
        let good_samples = test_tone(24_000, 1.0);
        let mut engine = ScriptedEngine::new(vec![bad_samples, good_samples.clone()]);
        let chunks = vec!["今天很好。".to_string()];

        let samples = synthesize_chunks_to_pcm(&mut engine, &chunks, None)
            .await
            .unwrap();

        assert_eq!(engine.calls, 2);
        assert_eq!(samples.len(), good_samples.len());
    }

    #[tokio::test]
    async fn worker_retries_broadband_noise_before_appending() {
        let noisy_samples = broadband_noise(24_000, 1.0);
        let good_samples = test_tone(24_000, 1.0);
        let mut engine = ScriptedEngine::new(vec![noisy_samples, good_samples.clone()]);
        let chunks = vec!["今天很好。".to_string()];

        let samples = synthesize_chunks_to_pcm(&mut engine, &chunks, None)
            .await
            .unwrap();

        assert_eq!(engine.calls, 2);
        assert_eq!(samples.len(), good_samples.len());
    }

    #[tokio::test]
    async fn worker_fails_instead_of_publishing_repeated_bad_chunk_audio() {
        let silent_samples = vec![0.0; 24_000];
        let mut engine = ScriptedEngine::new(vec![
            silent_samples.clone(),
            silent_samples.clone(),
            silent_samples,
        ]);
        let chunks = vec!["今天很好。".to_string()];

        let err = synthesize_chunks_to_pcm(&mut engine, &chunks, None)
            .await
            .unwrap_err();

        assert_eq!(engine.calls, TTS_CHUNK_MAX_ATTEMPTS);
        assert!(err.to_string().contains("failed audio quality gate"));
    }

    #[tokio::test]
    async fn worker_fails_instead_of_publishing_repeated_broadband_noise() {
        let noisy_samples = broadband_noise(24_000, 1.0);
        let mut engine = ScriptedEngine::new(vec![
            noisy_samples.clone(),
            noisy_samples.clone(),
            noisy_samples,
        ]);
        let chunks = vec!["今天很好。".to_string()];

        let err = synthesize_chunks_to_pcm(&mut engine, &chunks, None)
            .await
            .unwrap_err();

        assert_eq!(engine.calls, TTS_CHUNK_MAX_ATTEMPTS);
        assert!(err.to_string().contains("broadband"));
    }
}
