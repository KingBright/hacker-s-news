//! Job-level fallback: a published program always uses one provider/model.
use super::{
    config::{Config, Host},
    tts::TtsClient,
};
use anyhow::{Context, Result};
use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

struct Route {
    id: String,
    cloud: bool,
    client: Arc<TtsClient>,
    fingerprint: String,
    config: super::config::TtsConfig,
}
#[derive(Default, Clone, Serialize, Deserialize)]
struct RoutingState {
    blocked_until: BTreeMap<String, u64>,
    last_event: Option<serde_json::Value>,
}
pub struct TtsRouter {
    routes: Vec<Route>,
    state: Mutex<RoutingState>,
    state_path: PathBuf,
}

pub fn state_path() -> PathBuf {
    let root = std::env::var_os("CORTEX_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join(".freshloop")
        });
    root.join("cache/tts-routing.json")
}
pub fn public_status() -> serde_json::Value {
    match std::fs::read(state_path()) {
        Ok(bytes) => serde_json::from_slice::<RoutingState>(&bytes)
            .map(|s| serde_json::json!({"blocked_until":s.blocked_until,"last_event":s.last_event}))
            .unwrap_or_else(|_| serde_json::json!({"error":"routing state unreadable"})),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            serde_json::json!({"last_event":null})
        }
        Err(_) => serde_json::json!({"error":"routing state unreadable"}),
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("routing state parent")?)?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temp, serde_json::to_vec(value)?)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}
#[derive(Debug, PartialEq)]
enum Failure {
    Budget,
    Quota,
    Auth,
    Transient,
    InvalidAudio,
    Stop,
}
fn classify(error: &str) -> Failure {
    if error.contains("GEMINI_TTS[budget]") {
        Failure::Budget
    } else if error.contains("GEMINI_TTS[quota]") {
        Failure::Quota
    } else if error.contains("GEMINI_TTS[auth]") {
        Failure::Auth
    } else if error.contains("GEMINI_TTS[transient]") || error.contains("COLAB_TTS[transient]") {
        Failure::Transient
    } else if error.contains("GEMINI_TTS[invalid_audio]") {
        Failure::InvalidAudio
    } else {
        Failure::Stop
    }
}
fn cooldown(failure: &Failure, error: &str) -> u64 {
    cooldown_at(failure, error, now())
}
fn cooldown_at(failure: &Failure, error: &str, at: u64) -> u64 {
    let base = match failure {
        Failure::Quota => 60,
        Failure::Auth => 24 * 3600,
        _ => 300,
    };
    let retry = error
        .split("retry_after=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    if *failure == Failure::Quota {
        if retry > 0 {
            return retry.min(86400);
        }
        if error.contains("quota_scope=daily") {
            // Official RPD reset is Pacific midnight, including DST changes.
            let zone = chrono_tz::America::Los_Angeles;
            if let Some(current) = zone.timestamp_opt(at as i64, 0).single() {
                if let Some(date) = current.date_naive().succ_opt() {
                    if let Some(midnight) = date
                        .and_hms_opt(0, 0, 0)
                        .and_then(|v| zone.from_local_datetime(&v).single())
                    {
                        return (midnight.timestamp() - at as i64).max(1) as u64;
                    }
                }
            }
        }
    }
    base.max(retry.min(86400))
}
fn available(state: &RoutingState, route: &Route, at: u64) -> bool {
    let route_ok = state.blocked_until.get(&route.id).copied().unwrap_or(0) <= at;
    if !route_ok {
        return false;
    }
    if route.id == "colab_auto" {
        return state.blocked_until.get("colab").copied().unwrap_or(0) <= at;
    }
    !route.cloud || state.blocked_until.get("google").copied().unwrap_or(0) <= at
}

pub fn select_colab_engine(job_id: &str, voice_kind: &str, china_hour: u64) -> &'static str {
    match voice_kind {
        "curated_weekly" | "weekly_digest" => "indextts25",
        "reading_article" => {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(job_id.as_bytes());
            if digest[0] % 2 == 0 {
                "voxcpm2"
            } else {
                "indextts25"
            }
        }
        _ => {
            if china_hour < 14 {
                "voxcpm2"
            } else {
                "indextts25"
            }
        }
    }
}

fn colab_client_script_path() -> PathBuf {
    if let Some(custom) = std::env::var_os("FRESHLOOP_COLAB_TTS_CLIENT") {
        return PathBuf::from(custom);
    }
    for candidate in [
        PathBuf::from("/Users/jinliang/workspace/hacker-s-news/skills/colab-operator/scripts/colab_tts_client.py"),
        PathBuf::from("skills/colab-operator/scripts/colab_tts_client.py"),
        PathBuf::from("scripts/colab_tts_client.py"),
    ] {
        if candidate.is_file() {
            return candidate;
        }
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".gemini/config/skills/colab-operator/scripts/colab_tts_client.py")
}

async fn render_via_colab(
    job_id: &str,
    engine: &str,
    voice_kind: &str,
    text: &str,
    hosts: &[Host],
    config: &super::config::TtsConfig,
    cache: &Path,
) -> Result<(Vec<u8>, i64)> {
    tokio::fs::create_dir_all(cache).await?;
    let req_path = cache.join("colab-request.json");
    let out_mp3 = cache.join("output.mp3");
    let out_meta = cache.join("output.meta.json");
    let seg_dir = cache.join("segments");

    let default_voice = config
        .voxcpm
        .as_ref()
        .and_then(|v| v.prompt_wav_path.clone())
        .unwrap_or_else(|| "file:///Users/jinliang/workspace/hacker-s-news/audio/vivian.wav".into());
    let default_prompt = config
        .voxcpm
        .as_ref()
        .and_then(|v| v.prompt_text.clone())
        .unwrap_or_else(|| {
            "Hello 大家好，这里是 Fresh Loop 新闻速度，我们将给你带来最新鲜的新闻资讯。".into()
        });

    let hosts_json: Vec<serde_json::Value> = hosts
        .iter()
        .map(|h| {
            serde_json::json!({
                "name": h.name,
                "voice": h.voice,
                "prompt_text": h.prompt_text,
                "categories": h.categories,
            })
        })
        .collect();

    let payload = serde_json::json!({
        "job_id": job_id,
        "engine": engine,
        "voice_kind": voice_kind,
        "text": text,
        "hosts": hosts_json,
        "default_voice": default_voice,
        "default_prompt_text": default_prompt,
        "output_mp3": out_mp3,
        "output_meta": out_meta,
        "segment_cache_dir": seg_dir,
    });
    tokio::fs::write(&req_path, serde_json::to_vec_pretty(&payload)?).await?;

    let script = colab_client_script_path();
    log::info!(
        "[TtsRouting] Dispatching job={} kind={} to Colab L4 engine=colab_{} via {}",
        job_id,
        voice_kind,
        engine,
        script.display()
    );

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(25 * 60),
        tokio::process::Command::new("python3")
            .arg(&script)
            .arg("--cortex-request")
            .arg(&req_path)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("COLAB_TTS[transient] colab_tts_client timed out after 25m"))?
    .map_err(|e| anyhow::anyhow!("COLAB_TTS[transient] failed to spawn colab_tts_client: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!(
            "COLAB_TTS[transient] colab_tts_client exited with {}: stderr={} stdout={}",
            output.status,
            stderr.chars().take(600).collect::<String>(),
            stdout.chars().take(400).collect::<String>()
        );
    }

    let mp3_bytes = tokio::fs::read(&out_mp3)
        .await
        .context("COLAB_TTS[transient] missing output.mp3 from colab_tts_client")?;
    let meta_bytes = tokio::fs::read(&out_meta)
        .await
        .context("COLAB_TTS[transient] missing output.meta.json from colab_tts_client")?;
    let meta: serde_json::Value = serde_json::from_slice(&meta_bytes)
        .context("COLAB_TTS[transient] invalid output.meta.json")?;
    let duration_sec = meta
        .get("duration_seconds")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if duration_sec <= 0 {
        anyhow::bail!("COLAB_TTS[transient] non-positive duration_seconds in output.meta.json");
    }
    Ok((mp3_bytes, duration_sec))
}

impl TtsRouter {
    pub fn new(config: &Config, primary: Arc<TtsClient>) -> Result<Self> {
        let mut config = config.clone();
        super::config::apply_host_personas(&mut config)?;
        let engine_mode = config.tts.engine.as_deref().unwrap_or("voxcpm_metal").trim();
        let mut routes = Vec::new();
        if engine_mode == "colab" {
            let mut colab_cfg = config.tts.clone();
            colab_cfg.engine = Some("colab".into());
            routes.push(Route {
                id: "colab_auto".into(),
                cloud: true,
                fingerprint: format!("colab_dual:{:?}", colab_cfg.voxcpm),
                config: colab_cfg,
                client: primary.clone(),
            });
            if let Some(gemini) = config.tts.gemini.as_ref() {
                let mut models = vec![gemini.model.clone()];
                for model in &gemini.fallback_models {
                    if !models.contains(model) {
                        models.push(model.clone());
                    }
                }
                for model in models.into_iter().take(3) {
                    let mut tts = config.tts.clone();
                    tts.engine = Some("gemini".into());
                    tts.device = Some("cpu".into());
                    tts.gemini.as_mut().unwrap().model = model.clone();
                    routes.push(Route {
                        id: model,
                        cloud: true,
                        fingerprint: format!("{:?}", tts),
                        config: tts.clone(),
                        client: Arc::new(TtsClient::new(tts)),
                    });
                }
            }
            if config.tts.voxcpm.is_some() {
                let mut local = config.tts.clone();
                local.engine = Some("voxcpm_metal".into());
                local.device = Some("metal".into());
                local.process_isolation = Some(true);
                local.worker_max_processes = Some(2);
                routes.push(Route {
                    id: "voxcpm_metal".into(),
                    cloud: false,
                    fingerprint: format!("{:?}", local),
                    config: local.clone(),
                    client: Arc::new(TtsClient::new(local)),
                });
            }
        } else if engine_mode == "gemini" {
            let gemini = config
                .tts
                .gemini
                .as_ref()
                .context("Gemini config missing")?;
            let mut models = vec![gemini.model.clone()];
            for model in &gemini.fallback_models {
                if !models.contains(model) {
                    models.push(model.clone());
                }
            }
            anyhow::ensure!(models.len() <= 3, "at most three cloud TTS models per job");
            for model in models {
                let mut tts = config.tts.clone();
                tts.gemini.as_mut().unwrap().model = model.clone();
                routes.push(Route {
                    id: model,
                    cloud: true,
                    fingerprint: format!("{:?}", tts),
                    config: tts.clone(),
                    client: Arc::new(TtsClient::new(tts)),
                });
            }
            if gemini.fallback_local {
                anyhow::ensure!(
                    config.tts.voxcpm.is_some(),
                    "local TTS fallback requires VoxCPM config"
                );
                let mut local = config.tts.clone();
                local.engine = Some("voxcpm_metal".into());
                local.device = Some("metal".into());
                local.process_isolation = Some(true);
                local.worker_max_processes = Some(2);
                routes.push(Route {
                    id: "voxcpm_metal".into(),
                    cloud: false,
                    fingerprint: format!("{:?}", local),
                    config: local.clone(),
                    client: Arc::new(TtsClient::new(local)),
                });
            }
        } else {
            routes.push(Route {
                id: config
                    .tts
                    .engine
                    .clone()
                    .unwrap_or_else(|| "voxcpm_metal".into()),
                cloud: false,
                fingerprint: format!("{:?}", config.tts),
                config: config.tts.clone(),
                client: primary,
            });
        }
        let path = state_path();
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .context("invalid persisted TTS routing state; refusing to erase cooldown")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => RoutingState::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            routes,
            state: Mutex::new(state),
            state_path: path,
        })
    }
    /// Deployment checks must exercise the route a new production job can use now.
    pub async fn selected_config(&self) -> Result<super::config::TtsConfig> {
        let state = self.state.lock().await;
        let route = self
            .routes
            .iter()
            .find(|route| available(&state, route, now()))
            .context("no TTS route currently available for acceptance")?;
        log::info!(
            "[TtsRouting] Acceptance uses available provider={}",
            route.id
        );
        Ok(route.config.clone())
    }
    pub async fn clear_completed_cache(&self, job_id: &str) {
        let configs: Vec<_> = self
            .routes
            .iter()
            .filter_map(|r| r.config.gemini.clone())
            .collect();
        let job_id = job_id.to_string();
        let result = tokio::task::spawn_blocking(move || -> Result<()> {
            for config in configs {
                tts::adapters::gemini_resources::clear_job(&config, &job_id)?;
            }
            Ok(())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            log::warn!("[TtsRouting] Completed-job cache cleanup failed; retained fragments expire after24h");
        }
    }
    pub async fn render(
        &self,
        job_id: &str,
        text: &str,
        voice_kind: &str,
        hosts: &[Host],
        cache: &Path,
        concurrency: usize,
    ) -> Result<(Vec<u8>, i64)> {
        let program = voice_kind == "radio_program";
        let scheduled = program || voice_kind == "curated_weekly";
        let text = text.to_string();
        let hosts = hosts.to_vec();
        let root = cache.to_path_buf();
        let job_id_owned = job_id.to_string();
        let voice_kind_owned = voice_kind.to_string();
        self.drive(job_id, cache, |index| {
            let route = &self.routes[index];
            let is_colab = route.id == "colab_auto" || route.config.engine.as_deref() == Some("colab");
            let china_hour = ((now() + 8 * 3600) / 3600) % 24;
            let colab_engine = select_colab_engine(&job_id_owned, &voice_kind_owned, china_hour);
            let route_config = route.config.clone();
            let client = if is_colab {
                route.client.clone()
            } else if route.cloud {
                let mut config = route.config.clone();
                if let Some(gemini) = config.gemini.as_mut() {
                    gemini.cache_scope = Some(job_id_owned.clone());
                    if scheduled {
                        gemini.reserved_requests = if china_hour < 12 {
                            gemini.morning_reserved_requests
                        } else {
                            0
                        };
                    }
                }
                Arc::new(TtsClient::new(config))
            } else {
                route.client.clone()
            };
            let fingerprint = route.fingerprint.clone();
            let text = text.clone();
            let hosts = hosts.clone();
            let job_id_inner = job_id_owned.clone();
            let voice_kind_inner = voice_kind_owned.clone();
            let cache = root.join(format!("provider-{index}"));
            Box::pin(async move {
                if is_colab {
                    render_via_colab(
                        &job_id_inner,
                        colab_engine,
                        &voice_kind_inner,
                        &text,
                        &hosts,
                        &route_config,
                        &cache,
                    )
                    .await
                } else if program {
                    super::radio_program::render(
                        &text,
                        &hosts,
                        client,
                        &cache,
                        &fingerprint,
                        concurrency,
                    )
                    .await
                } else {
                    client.speak_mp3(&text).await
                }
            })
        })
        .await
    }
    async fn drive<F>(&self, job_id: &str, cache: &Path, mut generate: F) -> Result<(Vec<u8>, i64)>
    where
        F: FnMut(usize) -> futures::future::BoxFuture<'static, Result<(Vec<u8>, i64)>>,
    {
        let pin_path = cache.join("route.json");
        let pin = match std::fs::read(&pin_path) {
            Ok(bytes) => Some(
                serde_json::from_slice::<String>(&bytes)
                    .context("invalid voice-job route checkpoint")?,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let start = match pin {
            Some(id) => self.routes.iter().position(|r| r.id == id).context(
                "pinned voice route missing from config; reconcile job before changing provider",
            )?,
            None => 0,
        };
        let mut last_error = None;
        let mut budget_exhausted = false;
        for (index, route) in self.routes.iter().enumerate().skip(start) {
            if (budget_exhausted && route.cloud && route.id != "colab_auto")
                || !available(&*self.state.lock().await, route, now())
            {
                continue;
            }
            write_json(&pin_path, &route.id)?;
            let result = generate(index).await;
            match result {
                Ok(audio) => {
                    let provider_label = if route.id == "colab_auto" {
                        let meta_path = cache.join(format!("provider-{index}/output.meta.json"));
                        std::fs::read(&meta_path)
                            .ok()
                            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                            .and_then(|v| v.get("provider").and_then(|p| p.as_str()).map(String::from))
                            .unwrap_or_else(|| "colab_auto".to_string())
                    } else {
                        route.id.clone()
                    };
                    let mut state = self.state.lock().await;
                    state.last_event = Some(
                        serde_json::json!({"job_id":job_id,"provider":provider_label,"degraded":index>0,"phase":"synthesized","timestamp":now()}),
                    );
                    write_json(&self.state_path, &*state)?;
                    return Ok(audio);
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    let failure = classify(&message);
                    if !route.cloud || failure == Failure::Stop {
                        return Err(error);
                    }
                    let mut state = self.state.lock().await;
                    if failure == Failure::Budget {
                        budget_exhausted = true;
                        state.last_event = Some(
                            serde_json::json!({"job_id":job_id,"provider":route.id,"phase":"fallback","reason":"LocalBudget","timestamp":now()}),
                        );
                        write_json(&self.state_path, &*state)?;
                        last_error = Some(error);
                        continue;
                    }
                    let shared = matches!(failure, Failure::Quota | Failure::Auth);
                    let key = if route.id == "colab_auto" {
                        "colab"
                    } else if shared {
                        "google"
                    } else {
                        &route.id
                    };
                    let until = now() + cooldown(&failure, &message);
                    state.blocked_until.insert(key.to_string(), until);
                    state.last_event = Some(
                        serde_json::json!({"job_id":job_id,"provider":route.id,"phase":"fallback","reason":format!("{failure:?}"),"quota_scope":if message.contains("quota_scope=daily") {"daily"} else if message.contains("quota_scope=minute") {"minute"} else {"unknown"},"cooldown_until":until,"timestamp":now()}),
                    );
                    write_json(&self.state_path, &*state)?;
                    log::warn!("[TtsRouting] job={} provider={} reason={:?} cooldown_until={} rerendering whole job",job_id,route.id,failure,until);
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!(
                "all eligible TTS providers are cooling down; preserve existing voice job"
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn router(root: &Path) -> TtsRouter {
        let config: Config = toml::from_str(
            r#"
[nexus]
api_url="http://localhost"
auth_key="test"
[llm]
model="unused"
api_url="http://localhost"
[tts]
engine="voxcpm_metal"
"#,
        )
        .unwrap();
        let client = Arc::new(TtsClient::new(config.tts.clone()));
        TtsRouter {
            routes: vec![
                Route {
                    id: "flash".into(),
                    cloud: true,
                    client: client.clone(),
                    fingerprint: "a".into(),
                    config: config.tts.clone(),
                },
                Route {
                    id: "lite".into(),
                    cloud: true,
                    client: client.clone(),
                    fingerprint: "b".into(),
                    config: config.tts.clone(),
                },
                Route {
                    id: "local".into(),
                    cloud: false,
                    client,
                    fingerprint: "c".into(),
                    config: config.tts.clone(),
                },
            ],
            state: Mutex::new(RoutingState::default()),
            state_path: root.join("state.json"),
        }
    }
    #[tokio::test]
    async fn deployment_gate_selects_actual_available_fallback() {
        let root = std::env::temp_dir().join(format!("tts-gate-{}", uuid::Uuid::new_v4()));
        let mut router = router(&root);
        for route in &mut router.routes {
            if route.cloud {
                route.config.engine = Some("gemini".into());
            }
        }
        assert_eq!(
            router.selected_config().await.unwrap().engine.as_deref(),
            Some("gemini")
        );
        router
            .state
            .lock()
            .await
            .blocked_until
            .insert("google".into(), now() + 3600);
        assert_eq!(
            router.selected_config().await.unwrap().engine.as_deref(),
            Some("voxcpm_metal")
        );
    }

    #[tokio::test]
    async fn quota_skips_both_google_models_and_pins_retry_to_local() {
        let root = std::env::temp_dir().join(format!("tts-route-{}", uuid::Uuid::new_v4()));
        let router = router(&root);
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = calls.clone();
        let result = router
            .drive("job", &root.join("job"), move |index| {
                seen.lock().unwrap().push(index);
                Box::pin(async move {
                    if index == 0 {
                        anyhow::bail!("GEMINI_TTS[quota] HTTP 429")
                    }
                    Ok((vec![1], 1))
                })
            })
            .await
            .unwrap();
        assert_eq!(result.0, vec![1]);
        assert_eq!(*calls.lock().unwrap(), vec![0, 2]);
        router.state.lock().await.blocked_until.clear();
        router
            .drive("job", &root.join("job"), |index| {
                Box::pin(async move {
                    assert_eq!(index, 2);
                    Ok((vec![2], 1))
                })
            })
            .await
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn transient_tries_next_model_once_and_bad_request_stops() {
        let root = std::env::temp_dir().join(format!("tts-route-{}", uuid::Uuid::new_v4()));
        let router = router(&root);
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = calls.clone();
        router
            .drive("job", &root.join("job"), move |index| {
                seen.lock().unwrap().push(index);
                Box::pin(async move {
                    if index == 0 {
                        anyhow::bail!("GEMINI_TTS[transient] HTTP 503")
                    }
                    Ok((vec![1], 1))
                })
            })
            .await
            .unwrap();
        assert_eq!(*calls.lock().unwrap(), vec![0, 1]);
        router.state.lock().await.blocked_until.clear();
        let failed = router
            .drive("bad", &root.join("bad"), |index| {
                Box::pin(async move {
                    assert_eq!(index, 0);
                    anyhow::bail!("GEMINI_TTS[request] HTTP 400")
                })
            })
            .await;
        assert!(failed.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn quota_is_shared_and_auth_does_not_retry_next_google_model() {
        assert_eq!(
            classify("worker stderr: GEMINI_TTS[quota] HTTP 429 retry_after=60"),
            Failure::Quota
        );
        assert_eq!(cooldown(&Failure::Quota, "retry_after=60"), 60);
        assert_eq!(cooldown(&Failure::Quota, ""), 60);
        assert_eq!(cooldown(&Failure::Quota, "retry_after=81514"), 81514);
        assert_eq!(cooldown(&Failure::Auth, ""), 86400);
    }
    #[test]
    fn input_errors_and_unknown_failures_are_not_hidden_by_fallback() {
        assert_eq!(classify("GEMINI_TTS[request] HTTP 400"), Failure::Stop);
        assert_eq!(classify("disk full"), Failure::Stop);
        assert_eq!(
            classify("GEMINI_TTS[transient] timeout"),
            Failure::Transient
        );
    }
    #[test]
    fn retry_after_is_bounded_and_respected() {
        assert_eq!(cooldown(&Failure::Transient, "retry_after=900"), 900);
        assert_eq!(cooldown(&Failure::Transient, "retry_after=999999"), 86400);
    }
    #[test]
    fn state_roundtrip_preserves_cooldowns_and_no_manuscripts() {
        let path =
            std::env::temp_dir().join(format!("freshloop-routing-{}.json", uuid::Uuid::new_v4()));
        let mut state = RoutingState::default();
        state.blocked_until.insert("google".into(), 12345);
        write_json(&path, &state).unwrap();
        let loaded: RoutingState = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded.blocked_until.get("google"), Some(&12345));
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test]
    async fn local_budget_skips_other_cloud_without_cooling_priority_jobs() {
        let root = std::env::temp_dir().join(format!("tts-budget-{}", uuid::Uuid::new_v4()));
        let router = router(&root);
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = calls.clone();
        router
            .drive("budget", &root.join("job"), move |i| {
                seen.lock().unwrap().push(i);
                Box::pin(async move {
                    if i == 0 {
                        anyhow::bail!("GEMINI_TTS[budget] exhausted");
                    }
                    Ok((vec![1], 1))
                })
            })
            .await
            .unwrap();
        assert_eq!(*calls.lock().unwrap(), vec![0, 2]);
        assert!(router.state.lock().await.blocked_until.is_empty());
        fs_remove(&root);
    }
    fn fs_remove(path: &Path) {
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn confirmed_daily_quota_uses_pacific_midnight_and_explicit_retry_takes_precedence() {
        let summer = chrono::Utc
            .with_ymd_and_hms(2026, 10, 9, 1, 0, 0)
            .unwrap()
            .timestamp() as u64;
        let winter = chrono::Utc
            .with_ymd_and_hms(2026, 12, 9, 1, 0, 0)
            .unwrap()
            .timestamp() as u64;
        assert_eq!(
            cooldown_at(&Failure::Quota, "quota_scope=daily", summer),
            6 * 3600
        );
        assert_eq!(
            cooldown_at(&Failure::Quota, "quota_scope=daily", winter),
            7 * 3600
        );
        assert_eq!(
            cooldown_at(&Failure::Quota, "quota_scope=minute", summer),
            60
        );
        assert_eq!(
            cooldown_at(&Failure::Quota, "retry_after=12 quota_scope=minute", summer),
            12
        );
        assert_eq!(
            cooldown_at(
                &Failure::Quota,
                "retry_after=81514 quota_scope=daily",
                summer
            ),
            81514
        );
    }
    #[test]
    fn colab_ab_routing_splits_morning_evening_weekly_and_reading() {
        assert_eq!(select_colab_engine("job-1", "radio_program", 7), "voxcpm2");
        assert_eq!(select_colab_engine("job-1", "radio_program", 18), "indextts25");
        assert_eq!(select_colab_engine("job-1", "curated_weekly", 8), "indextts25");
        let mut saw_voxcpm = false;
        let mut saw_indextts = false;
        for i in 0..16 {
            let engine = select_colab_engine(&format!("reading-job-{i}"), "reading_article", 9);
            if engine == "voxcpm2" {
                saw_voxcpm = true;
            } else if engine == "indextts25" {
                saw_indextts = true;
            }
        }
        assert!(saw_voxcpm && saw_indextts);
        assert_eq!(
            classify("COLAB_TTS[transient] colab_tts_client timed out"),
            Failure::Transient
        );
    }
}

