//! Gemini cloud speech. Credentials are read at runtime and never serialized.
use crate::{TtsEngine, VoicePrompt};
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Cursor, time::Duration};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeminiConfig {
    /// Local planning budget, not a claim about provider quotas. Zero disables cap.
    pub daily_request_budget: u32,
    pub reserved_requests: u32,
    pub morning_reserved_requests: u32,
    pub min_request_interval_secs: u64,
    pub resource_dir: Option<String>,
    /// Internal voice-job scope for temporary recovery fragments.
    pub cache_scope: Option<String>,
    pub model: String,
    pub voice: String,
    pub style: String,
    pub api_key_env: String,
    pub api_key_file: Option<String>,
    pub proxy: Option<String>,
    pub timeout_secs: u64,
    pub max_request_chars: usize,
    /// Explicit mapping of existing host reference paths to Gemini prebuilt voices.
    pub host_voices: BTreeMap<String, String>,
    pub host_styles: BTreeMap<String, String>,
    pub fallback_models: Vec<String>,
    pub fallback_local: bool,
}
impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            daily_request_budget: 0,
            reserved_requests: 0,
            morning_reserved_requests: 0,
            min_request_interval_secs: 20,
            resource_dir: None,
            cache_scope: None,
            model: "gemini-3.8-flash-tts".into(),
            voice: "Kore".into(),
            style: "用自然、清晰的普通话播报，忠实读出原稿，不增加内容。".into(),
            api_key_env: "GEMINI_API_KEY".into(),
            api_key_file: None,
            proxy: None,
            timeout_secs: 600,
            max_request_chars: 400,
            host_voices: BTreeMap::new(),
            host_styles: BTreeMap::new(),
            fallback_models: vec!["gemini-3.8-flash-lite-tts".into()],
            fallback_local: true,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Speaker {
    pub name: String,
    pub voice: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    pub speaker: String,
    pub text: String,
    #[serde(default)]
    pub style: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Dialogue {
    pub speakers: Vec<Speaker>,
    pub turns: Vec<Turn>,
}

pub struct GeminiAdapter {
    config: GeminiConfig,
    voice: String,
    style: String,
    rate: usize,
}
impl GeminiAdapter {
    pub fn new(config: GeminiConfig) -> Result<Self> {
        ensure!(
            !config.model.trim().is_empty() && !config.voice.trim().is_empty(),
            "Gemini model and voice required"
        );
        ensure!(
            (1..=600).contains(&config.timeout_secs),
            "Gemini timeout must be 1..600 seconds"
        );
        ensure!(
            (100..=8000).contains(&config.max_request_chars),
            "Gemini request limit must be 100..8000 characters"
        );
        Ok(Self {
            voice: config.voice.clone(),
            style: config.style.clone(),
            config,
            rate: 24000,
        })
    }
    fn request_body(&self, text: &str) -> Result<Value> {
        ensure!(!text.trim().is_empty(), "Speech text is empty");
        ensure!(
            text.chars().count() <= self.config.max_request_chars,
            "Text exceeds Gemini request character budget; use synthesize_text for splitting"
        );
        Ok(body(
            &self.config.model,
            json!([{"type":"text", "text":text,
            "annotations":[{"type":"speech_metadata", "style":self.style}]}]),
            json!([{"voice":self.voice}]),
        ))
    }
    pub fn synthesize_dialogue(&mut self, dialogue: &Dialogue) -> Result<Vec<f32>> {
        validate_dialogue(dialogue)?;
        if dialogue.speakers.len() == 2
            && dialogue
                .turns
                .iter()
                .map(|t| t.text.chars().count())
                .sum::<usize>()
                <= self.config.max_request_chars
        {
            return self.execute(dialogue_body(&self.config.model, dialogue)?);
        }
        // Native API supports two speakers. More voices (or long dialogue) use
        // explicit per-turn synthesis, preserving order and never mixing voices.
        log::info!(
            "Gemini dialogue: per-turn composition for {} speakers",
            dialogue.speakers.len()
        );
        let mut output = Vec::new();
        for (index, turn) in dialogue.turns.iter().enumerate() {
            let speaker = dialogue
                .speakers
                .iter()
                .find(|s| s.name == turn.speaker)
                .unwrap();
            let mut config = self.config.clone();
            config.voice = speaker.voice.clone();
            if !turn.style.is_empty() {
                config.style = turn.style.clone();
            }
            let samples = Self::new(config)?
                .synthesize_text(&turn.text)
                .with_context(|| {
                    format!(
                        "Gemini dialogue turn {} of {} failed",
                        index + 1,
                        dialogue.turns.len()
                    )
                })?;
            if !output.is_empty() {
                output.resize(output.len() + self.rate / 5, 0.0);
            }
            output.extend(samples);
        }
        Ok(output)
    }
    pub fn synthesize_text(&mut self, text: &str) -> Result<Vec<f32>> {
        ensure!(!text.trim().is_empty(), "Speech text is empty");
        let mut output = Vec::new();
        for chunk in speech_chunks(text, self.config.max_request_chars) {
            let samples = self.synthesize_chunk(&chunk)?;
            if !output.is_empty() {
                output.resize(output.len() + self.rate / 5, 0.0);
            }
            output.extend(samples);
        }
        Ok(output)
    }
    fn execute(&mut self, request: Value) -> Result<Vec<f32>> {
        // All call sites use the synchronous engine contract. Build/drop HTTP runtime
        // on a dedicated thread, including when called from a Tokio worker.
        let config = self.config.clone();
        let response = std::thread::spawn(move || -> Result<Value> {
            let key = read_api_key(&config)?;
            let mut resources = super::gemini_resources::Resources::open(&config, &request)?;
            if let Some(cached) = resources.cached()? { return Ok(cached); }
            resources.reserve(&config)?;
            let mut builder = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(config.timeout_secs))
                .connect_timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none());
            if let Some(proxy) = &config.proxy {
                builder = builder.proxy(
                    reqwest::Proxy::all(proxy)
                        .map_err(|_| anyhow::anyhow!("Invalid Gemini proxy"))?,
                );
            }
            let client = builder.build().context("Build Gemini HTTP client")?;
            let response = client
                .post("https://generativelanguage.googleapis.com/v1beta/interactions")
                .header("x-goog-api-key", key)
                .json(&request)
                .send()
                .map_err(|_| {
                    anyhow::anyhow!(
                        "GEMINI_TTS[transient] transport failed or timed out; request may have been processed"
                    )
                })?;
            // Only status and bounded Retry-After are exposed; never provider bodies.
            let status = response.status().as_u16();
            if !response.status().is_success() {
                let header_delay = response.headers().get("retry-after").and_then(|v|v.to_str().ok()).and_then(|v|v.parse::<u64>().ok());
                let details: Value = response.json().unwrap_or(Value::Null);
                let (scope, body_delay) = quota_diagnostics(&details);
                let delay = header_delay.into_iter().chain(body_delay).max().unwrap_or(0).min(86400);
                bail!("GEMINI_TTS[{}] HTTP {} retry_after={} quota_scope={}", failure_kind(status), status, delay, scope);
            }
            let value: Value = response.json().context("GEMINI_TTS[invalid_audio] Invalid Gemini JSON response")?;
            let (samples, rate) = decode_response(&value).context("GEMINI_TTS[invalid_audio] response failed audio validation")?;
            resources.complete(&value, samples.len() as f64 / rate as f64)?;
            Ok(value)
        })
        .join()
        .map_err(|_| anyhow::anyhow!("Gemini HTTP thread failed"))??;
        let (samples, rate) = decode_response(&response)
            .context("GEMINI_TTS[invalid_audio] response failed audio validation")?;
        self.rate = rate;
        Ok(samples)
    }
}
// Extract only a bounded classification and retry duration, never provider messages/keys.
fn quota_diagnostics(value: &Value) -> (&'static str, Option<u64>) {
    let mut scope = "unknown";
    let mut delay = None;
    if let Some(details) = value["error"]["details"].as_array() {
        for detail in details {
            if detail["@type"]
                .as_str()
                .unwrap_or("")
                .ends_with("RetryInfo")
            {
                delay = detail["retryDelay"]
                    .as_str()
                    .and_then(|s| s.strip_suffix('s'))
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .map(|v| v.ceil().min(86400.0) as u64);
            }
            if let Some(violations) = detail["violations"].as_array() {
                for violation in violations {
                    let id = format!(
                        "{} {}",
                        violation["quotaMetric"].as_str().unwrap_or(""),
                        violation["quotaId"].as_str().unwrap_or("")
                    )
                    .to_ascii_lowercase();
                    if id.contains("perday") || id.contains("per_day") {
                        scope = "daily";
                    } else if scope != "daily"
                        && (id.contains("perminute") || id.contains("per_minute"))
                    {
                        scope = "minute";
                    }
                }
            }
        }
    }
    (scope, delay)
}

fn read_api_key(config: &GeminiConfig) -> Result<String> {
    let key = match std::env::var(&config.api_key_env)
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(key) => key,
        None => {
            let path = config
                .api_key_file
                .as_ref()
                .context("GEMINI_TTS[auth] API key is not configured")?;
            let metadata =
                std::fs::metadata(path).context("GEMINI_TTS[auth] API key file unavailable")?;
            ensure!(
                metadata.is_file(),
                "GEMINI_TTS[auth] API key path must be a file"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                ensure!(
                    metadata.permissions().mode() & 0o077 == 0,
                    "GEMINI_TTS[auth] API key file must be private (0600)"
                );
            }
            std::fs::read_to_string(path).context("GEMINI_TTS[auth] cannot read API key file")?
        }
    };
    let key = key.trim().to_string();
    ensure!(!key.is_empty(), "GEMINI_TTS[auth] API key is empty");
    Ok(key)
}

pub fn failure_kind(status: u16) -> &'static str {
    match status {
        401 | 403 => "auth",
        429 => "quota",
        408 | 500..=599 => "transient",
        _ => "request",
    }
}

fn body(model: &str, content: Value, speech: Value) -> Value {
    json!({"model":model, "input":[{"type":"user_input","content":content}],
        "response_format":{"type":"audio","mime_type":"audio/wav"},
        "generation_config":{"speech_config":speech}})
}
fn dialogue_body(model: &str, dialogue: &Dialogue) -> Result<Value> {
    ensure!(
        dialogue.speakers.len() == 2,
        "Gemini dialogue requires exactly two speakers"
    );
    validate_dialogue(dialogue)?;
    Ok(body(
        model,
        json!(dialogue
            .turns
            .iter()
            .map(|t| json!({"type":"text","text":t.text,
        "annotations":[{"type":"speech_metadata","speaker":t.speaker,"style":t.style}]}))
            .collect::<Vec<_>>()),
        json!({"mode":"conversational","speakers":dialogue.speakers.iter().map(|s| json!({"speaker":s.name,"voice":s.voice})).collect::<Vec<_>>()}),
    ))
}
fn validate_dialogue(dialogue: &Dialogue) -> Result<()> {
    ensure!(
        dialogue.speakers.len() >= 2,
        "Dialogue requires at least two speakers"
    );
    let mut names = std::collections::HashSet::new();
    let mut voices = std::collections::HashSet::new();
    for speaker in &dialogue.speakers {
        ensure!(
            !speaker.name.trim().is_empty() && names.insert(&speaker.name),
            "Speaker names must be unique and nonempty"
        );
        ensure!(
            !speaker.voice.trim().is_empty() && voices.insert(&speaker.voice),
            "Speaker voices must be distinct and nonempty"
        );
    }
    ensure!(!dialogue.turns.is_empty(), "Dialogue has no turns");
    for turn in &dialogue.turns {
        ensure!(names.contains(&turn.speaker), "Unknown dialogue speaker");
        ensure!(!turn.text.trim().is_empty(), "Empty dialogue turn");
    }
    Ok(())
}
/// Preserve every character; prefer a sentence boundary in the latter half.
pub fn speech_chunks(text: &str, limit: usize) -> Vec<String> {
    assert!(limit > 0);
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    let mut output = Vec::new();
    while start < chars.len() {
        let mut end = (start + limit).min(chars.len());
        if end < chars.len() {
            if let Some(index) = (start + limit / 2..end)
                .rev()
                .find(|i| "。！？.!?\n".contains(chars[*i]))
            {
                end = index + 1;
            }
        }
        output.push(chars[start..end].iter().collect());
        start = end;
    }
    output
}
pub(super) fn decode_response(response: &Value) -> Result<(Vec<f32>, usize)> {
    ensure!(
        response["status"] == "completed",
        "Gemini interaction did not complete"
    );
    let audio: Vec<_> = response["steps"]
        .as_array()
        .context("Missing Gemini steps")?
        .iter()
        .filter(|s| s["type"] == "model_output")
        .flat_map(|s| s["content"].as_array().into_iter().flatten())
        .filter(|c| c["type"] == "audio")
        .collect();
    ensure!(
        audio.len() == 1,
        "Expected one complete Gemini audio output"
    );
    ensure!(
        audio[0]["mime_type"] == "audio/wav",
        "Unexpected Gemini audio MIME; WAV required"
    );
    let bytes = STANDARD
        .decode(
            audio[0]["data"]
                .as_str()
                .context("Missing Gemini audio data")?,
        )
        .context("Invalid audio base64")?;
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).context("Invalid Gemini WAV")?;
    let spec = reader.spec();
    ensure!(
        spec.channels == 1
            && spec.bits_per_sample == 16
            && spec.sample_format == hound::SampleFormat::Int
            && spec.sample_rate == 24000,
        "Expected 24 kHz mono PCM16 WAV"
    );
    let samples = reader
        .samples::<i16>()
        .map(|s| s.map(|s| s as f32 / 32768.0))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(!samples.is_empty(), "Empty Gemini audio");
    Ok((samples, spec.sample_rate as usize))
}
#[async_trait::async_trait]
impl TtsEngine for GeminiAdapter {
    async fn synthesize_long_text(&mut self, text: &str) -> Result<Vec<f32>> {
        self.synthesize_text(text)
    }
    fn cache_voice_prompt(&mut self, prompt: &VoicePrompt) -> Result<()> {
        if let Some(path) = &prompt.wav_path {
            self.style = self
                .config
                .host_styles
                .get(path)
                .map(|style| format!("{} {}", self.config.style, style))
                .unwrap_or_else(|| self.config.style.clone());
            self.voice = self
                .config
                .host_voices
                .get(path)
                .cloned()
                .with_context(|| {
                    "Gemini cannot clone local WAV voices; configure gemini.host_voices explicitly"
                })?;
        } else if prompt.text.is_some() {
            bail!("Gemini does not support reference transcript voice cloning");
        }
        Ok(())
    }
    fn sample_rate(&self) -> usize {
        self.rate
    }
    fn synthesize_chunk(&mut self, text: &str) -> Result<Vec<f32>> {
        self.execute(self.request_body(text)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dialogue() -> Dialogue {
        Dialogue {
            speakers: vec![
                Speaker {
                    name: "甲".into(),
                    voice: "Kore".into(),
                },
                Speaker {
                    name: "乙".into(),
                    voice: "Puck".into(),
                },
            ],
            turns: vec![Turn {
                speaker: "甲".into(),
                text: "你好".into(),
                style: "温柔".into(),
            }],
        }
    }
    #[test]
    fn dialogue_separates_direction_from_spoken_text() {
        let v = dialogue_body("model", &dialogue()).unwrap();
        assert_eq!(v["input"][0]["content"][0]["text"], "你好");
        assert_eq!(
            v["input"][0]["content"][0]["annotations"][0]["style"],
            "温柔"
        );
        assert_eq!(
            v["generation_config"]["speech_config"]["mode"],
            "conversational"
        );
    }
    #[test]
    fn long_text_preserves_all_characters_and_short_text_is_one_request() {
        let text = "第一句。第二句更长一些！最后一句。";
        let chunks = speech_chunks(text, 8);
        assert_eq!(chunks.concat(), text);
        assert!(chunks.iter().all(|c| c.chars().count() <= 8));
        assert_eq!(speech_chunks(text, 100), vec![text]);
    }
    #[test]
    fn three_speakers_validate_but_cannot_use_native_request() {
        let mut d = dialogue();
        d.speakers.push(Speaker {
            name: "丙".into(),
            voice: "Charon".into(),
        });
        assert!(validate_dialogue(&d).is_ok());
        assert!(dialogue_body("m", &d).is_err());
    }
    #[test]
    fn host_personality_is_metadata_and_reference_text_is_not_read() {
        let mut config = GeminiConfig::default();
        config.host_voices.insert("host.wav".into(), "Aoede".into());
        config
            .host_styles
            .insert("host.wav".into(), "轻快但不喊叫".into());
        let mut engine = GeminiAdapter::new(config).unwrap();
        engine
            .cache_voice_prompt(&VoicePrompt {
                wav_path: Some("host.wav".into()),
                text: Some("reference only".into()),
            })
            .unwrap();
        let body = engine.request_body("这是新闻正文").unwrap();
        assert_eq!(
            body["generation_config"]["speech_config"][0]["voice"],
            "Aoede"
        );
        assert!(body["input"][0]["content"][0]["annotations"][0]["style"]
            .as_str()
            .unwrap()
            .contains("轻快但不喊叫"));
        assert_eq!(body["input"][0]["content"][0]["text"], "这是新闻正文");
        assert!(!body.to_string().contains("reference only"));
    }
    #[test]
    fn error_status_classes_do_not_hide_bad_requests() {
        assert_eq!(failure_kind(429), "quota");
        assert_eq!(failure_kind(403), "auth");
        assert_eq!(failure_kind(503), "transient");
        assert_eq!(failure_kind(400), "request");
    }
    #[test]
    fn unknown_and_duplicate_speakers_rejected() {
        let mut d = dialogue();
        d.turns[0].speaker = "丙".into();
        assert!(dialogue_body("m", &d).is_err());
        d = dialogue();
        d.speakers[1].name = "甲".into();
        assert!(dialogue_body("m", &d).is_err());
    }
    #[test]
    fn cloning_requires_explicit_mapping() {
        let mut e = GeminiAdapter::new(GeminiConfig::default()).unwrap();
        assert!(e
            .cache_voice_prompt(&VoicePrompt {
                text: None,
                wav_path: Some("local.wav".into())
            })
            .is_err());
    }
    #[test]
    fn decodes_wav_and_rejects_incomplete_or_wrong_mime() {
        let mut cursor = Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels: 1,
                sample_rate: 24000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        w.write_sample(16384i16).unwrap();
        w.finalize().unwrap();
        let mut v = json!({"status":"completed","steps":[{"type":"model_output","content":[{"type":"audio","mime_type":"audio/wav","data":STANDARD.encode(cursor.into_inner())}]}]});
        let (samples, rate) = decode_response(&v).unwrap();
        assert_eq!(rate, 24000);
        assert_eq!(samples, vec![0.5]);
        v["status"] = json!("failed");
        assert!(decode_response(&v).is_err());
        v["status"] = json!("completed");
        v["steps"][0]["content"][0]["mime_type"] = json!("audio/l16");
        assert!(decode_response(&v).is_err());
    }
    #[test]
    fn quota_details_distinguish_minute_day_and_retry_without_echoing_messages() {
        let v = json!({"error":{"message":"private provider text","details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"12.5s"},{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaId":"GenerateRequestsPerMinutePerProject"}]}]}});
        assert_eq!(quota_diagnostics(&v), ("minute", Some(13)));
        let v = json!({"error":{"details":[{"violations":[{"quotaMetric":"generate_requests_per_day"}]}]}});
        assert_eq!(quota_diagnostics(&v), ("daily", None));
        assert_eq!(quota_diagnostics(&Value::Null), ("unknown", None));
    }
}
