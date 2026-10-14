//! Cross-worker request accounting and validated response reuse. No credentials in state.
use super::gemini::{decode_response, GeminiConfig};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
// Local editorial budget resets at Beijing midnight, independent of Google quota resets.
fn day(at: u64) -> u64 {
    (at + 8 * 3600) / 86400
}
#[derive(Default, Serialize, Deserialize)]
struct Usage {
    day: u64,
    attempts: u32,
    completed: u32,
    cache_hits: u32,
    audio_seconds: f64,
    last_request: u64,
}
pub fn root(config: &GeminiConfig) -> PathBuf {
    config
        .resource_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("CORTEX_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".freshloop")
                })
                .join("cache/gemini-requests")
        })
}
fn scope_prefix(scope: &str) -> String {
    format!("v2-{:x}", Sha256::digest(scope.as_bytes()))
}
/// Called only after Nexus acknowledges completion. Never remove another job's fragments.
pub fn clear_job(config: &GeminiConfig, job_id: &str) -> Result<()> {
    let root = root(config);
    if !root.exists() {
        return Ok(());
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("request.lock"))?;
    lock.lock()?;
    let prefix = format!("{}-", scope_prefix(job_id));
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
pub fn status(config: &GeminiConfig) -> Value {
    match fs::read(root(config).join("usage.json")) {
        Ok(bytes) => match serde_json::from_slice::<Usage>(&bytes) {
            Ok(mut usage) => {
                rollover(&mut usage, now());
                serde_json::to_value(usage).unwrap()
            }
            Err(_) => serde_json::json!({"error":"invalid request accounting"}),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            serde_json::json!({"attempts":0,"completed":0,"cache_hits":0,"audio_seconds":0})
        }
        Err(_) => serde_json::json!({"error":"request accounting unavailable"}),
    }
}
fn rollover(usage: &mut Usage, at: u64) {
    if usage.day != day(at) {
        *usage = Usage {
            day: day(at),
            last_request: usage.last_request,
            ..Default::default()
        };
    }
}
fn allowed(usage: &Usage, config: &GeminiConfig) -> bool {
    config.daily_request_budget == 0
        || usage.attempts
            < config
                .daily_request_budget
                .saturating_sub(config.reserved_requests)
}
pub struct Resources {
    root: PathBuf,
    cache: PathBuf,
    usage: Usage,
    _lock: File,
}
impl Resources {
    pub fn open(config: &GeminiConfig, request: &Value) -> Result<Self> {
        let root = root(config);
        fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("request.lock"))?;
        lock.lock().context("lock Gemini request accounting")?;
        let mut usage: Usage = match fs::read(root.join("usage.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .context("invalid Gemini request accounting; refusing to reset")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Usage::default(),
            Err(e) => return Err(e.into()),
        };
        rollover(&mut usage, now());
        // Only request payload participates: model, text, voice and style, never API key.
        let key = format!("{:x}", Sha256::digest(serde_json::to_vec(request)?));
        let cache = root.join(format!(
            "{}-{key}.json",
            scope_prefix(config.cache_scope.as_deref().unwrap_or("standalone"))
        ));
        Ok(Self {
            root,
            cache,
            usage,
            _lock: lock,
        })
    }
    fn save(&self) -> Result<()> {
        self.write(
            &self.root.join("usage.json"),
            &serde_json::to_vec(&self.usage)?,
        )
    }
    fn write(&self, path: &std::path::Path, bytes: &[u8]) -> Result<()> {
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes)?;
        fs::rename(temp, path)?;
        Ok(())
    }
    pub fn cached(&mut self) -> Result<Option<Value>> {
        // Recovery fragments only: discard after 24 hours, including legacy v1 entries.
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if (entry.file_name().to_string_lossy().starts_with("v1-")
                || entry.file_name().to_string_lossy().starts_with("v2-"))
                && entry.metadata()?.modified()?.elapsed().unwrap_or_default()
                    > Duration::from_secs(86400)
            {
                fs::remove_file(entry.path())?;
            }
        }
        if let Ok(bytes) = fs::read(&self.cache) {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                if decode_response(&value).is_ok() {
                    self.usage.cache_hits += 1;
                    self.save()?;
                    return Ok(Some(value));
                }
            }
        }
        Ok(None)
    }
    pub fn reserve(&mut self, config: &GeminiConfig) -> Result<()> {
        let delay = self
            .usage
            .last_request
            .saturating_add(config.min_request_interval_secs.min(60))
            .saturating_sub(now());
        if delay > 0 {
            std::thread::sleep(Duration::from_secs(delay));
        }
        rollover(&mut self.usage, now());
        anyhow::ensure!(
            allowed(&self.usage, config),
            "GEMINI_TTS[budget] local editorial request allowance exhausted"
        );
        // Reserve before transmission: failures/timeouts may consume provider capacity.
        self.usage.attempts += 1;
        self.usage.last_request = now();
        self.save()
    }
    pub fn complete(&mut self, response: &Value, seconds: f64) -> Result<()> {
        self.write(&self.cache, &serde_json::to_vec(response)?)?;
        self.usage.completed += 1;
        self.usage.audio_seconds += seconds;
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserve_preserves_priority_allowance() {
        let mut c = GeminiConfig::default();
        c.daily_request_budget = 60;
        c.reserved_requests = 40;
        let mut u = Usage {
            attempts: 19,
            ..Default::default()
        };
        assert!(allowed(&u, &c));
        u.attempts = 20;
        assert!(!allowed(&u, &c));
        c.reserved_requests = 0;
        assert!(allowed(&u, &c));
        u.attempts = 60;
        assert!(!allowed(&u, &c));
    }
    #[test]
    fn editorial_midnight_rolls_budget_without_resetting_spacing() {
        let boundary = 86400 - 8 * 3600;
        let mut u = Usage {
            day: day(boundary - 1),
            attempts: 60,
            last_request: boundary - 1,
            ..Default::default()
        };
        rollover(&mut u, boundary);
        assert_eq!(u.attempts, 0);
        assert_eq!(u.last_request, boundary - 1);
    }
    fn fixture() -> (GeminiConfig, Value) {
        let root = std::env::temp_dir().join(format!(
            "gemini-resource-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut c = GeminiConfig::default();
        c.resource_dir = Some(root.to_string_lossy().into());
        c.min_request_interval_secs = 0;
        c.daily_request_budget = 1;
        (
            c,
            serde_json::json!({"model":"test","text":"hello","voice":"Kore"}),
        )
    }
    #[test]
    fn restart_cannot_erase_attempts_and_corruption_fails_closed() {
        let (c, request) = fixture();
        {
            let mut r = Resources::open(&c, &request).unwrap();
            r.reserve(&c).unwrap();
        }
        {
            let mut r = Resources::open(&c, &request).unwrap();
            assert!(r
                .reserve(&c)
                .unwrap_err()
                .to_string()
                .contains("GEMINI_TTS[budget]"));
        }
        fs::write(root(&c).join("usage.json"), b"broken").unwrap();
        assert!(Resources::open(&c, &request).is_err());
        fs::remove_dir_all(root(&c)).unwrap();
    }
    #[test]
    fn completed_cache_survives_restart_without_spending_and_voice_changes_miss() {
        use base64::Engine;
        let (c, request) = fixture();
        let mut wav = std::io::Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(
                &mut wav,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 24000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for _ in 0..240 {
                w.write_sample(1_i16).unwrap();
            }
            w.finalize().unwrap();
        }
        let response = serde_json::json!({"status":"completed","steps":[{"type":"model_output","content":[{"type":"audio","mime_type":"audio/wav","data":base64::engine::general_purpose::STANDARD.encode(wav.into_inner())}]}]});
        {
            let mut r = Resources::open(&c, &request).unwrap();
            r.reserve(&c).unwrap();
            r.complete(&response, 0.01).unwrap();
        }
        {
            let mut r = Resources::open(&c, &request).unwrap();
            assert!(r.cached().unwrap().is_some());
            assert_eq!(r.usage.attempts, 1);
        }
        let mut changed = request.clone();
        changed["voice"] = serde_json::json!("Aoede");
        {
            let mut r = Resources::open(&c, &changed).unwrap();
            assert!(r.cached().unwrap().is_none());
            assert!(r.reserve(&c).is_err());
        }
        fs::remove_dir_all(root(&c)).unwrap();
    }
    #[test]
    fn concurrent_workers_share_one_allowance() {
        let (c, request) = fixture();
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let c = c.clone();
                let request = request.clone();
                std::thread::spawn(move || {
                    Resources::open(&c, &request).unwrap().reserve(&c).is_ok()
                })
            })
            .collect();
        assert_eq!(
            workers
                .into_iter()
                .filter_map(|w| w.join().ok())
                .filter(|ok| *ok)
                .count(),
            1
        );
        fs::remove_dir_all(root(&c)).unwrap();
    }
    #[test]
    fn completion_only_removes_its_own_job_cache_and_preserves_accounting() {
        let (mut c, request) = fixture();
        c.cache_scope = Some("job-a".into());
        let a = {
            let r = Resources::open(&c, &request).unwrap();
            fs::write(&r.cache, b"a").unwrap();
            r.cache.clone()
        };
        c.cache_scope = Some("job-b".into());
        let b = {
            let mut r = Resources::open(&c, &request).unwrap();
            r.reserve(&c).unwrap();
            fs::write(&r.cache, b"b").unwrap();
            r.cache.clone()
        };
        clear_job(&c, "job-a").unwrap();
        assert!(!a.exists());
        assert!(b.exists());
        assert!(root(&c).join("usage.json").exists());
        fs::remove_dir_all(root(&c)).unwrap();
    }
    #[test]
    fn fragments_expire_after_24_hours_including_legacy_entries() {
        let (c, request) = fixture();
        {
            let mut r = Resources::open(&c, &request).unwrap();
            for path in [&r.cache, &r.root.join("v1-legacy.json")] {
                fs::write(path, b"old").unwrap();
                File::options()
                    .write(true)
                    .open(path)
                    .unwrap()
                    .set_modified(SystemTime::now() - Duration::from_secs(86401))
                    .unwrap();
            }
            assert!(r.cached().unwrap().is_none());
            assert!(!r.cache.exists());
            assert!(!r.root.join("v1-legacy.json").exists());
        }
        fs::remove_dir_all(root(&c)).unwrap();
    }
}
