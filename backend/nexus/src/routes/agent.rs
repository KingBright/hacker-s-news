use crate::AppState;
#[path = "radio_program.rs"]
mod radio_program;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use loop_memory::{MemoryEntry, MemoryStore, MemoryType, Provenance};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, Sqlite, Transaction};
use std::collections::HashMap;
use uuid::Uuid;

const ALLOWED_AGENT_JOB_TYPES: &[&str] = &[
    "radio_program",
    "radio_episode",
    "reading_article",
    "weekly_digest",
    "loop_preference_extraction",
    "holiday_calendar_update",
];
const ALLOWED_AGENT_STATUSES: &[&str] = &["queued", "leased", "completed", "failed", "cancelled"];
const ALLOWED_VOICE_TARGET_TYPES: &[&str] = &["radio_item", "feed_item", "weekly_digest"];
const ALLOWED_VOICE_STATUSES: &[&str] = &["queued", "leased", "completed", "failed", "cancelled"];
const MAX_TEXT_LENGTH: usize = 80_000;
const MAX_TITLE_LENGTH: usize = 500;
const MAX_ERROR_LENGTH: usize = 1_000;
const DEFAULT_VOICE_REPAIR_LIMIT: i64 = 20;
const MAX_VOICE_REPAIR_LIMIT: i64 = 200;
const MAX_RECENT_FAILED_REPAIR_JOBS: i64 = 2;
const DEFAULT_CONTENT_VOICE_PRIORITY: i64 = 10;
const DEFAULT_LEASE_SECONDS: i64 = 30 * 60;
const MAX_LEASE_SECONDS: i64 = 6 * 60 * 60;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AgentJob {
    pub id: String,
    pub job_type: String,
    pub status: String,
    pub priority: Option<i64>,
    pub run_after: Option<i64>,
    pub lease_owner: Option<String>,
    pub lease_token: Option<String>,
    pub lease_expires_at: Option<i64>,
    pub attempt_count: Option<i64>,
    pub max_attempts: Option<i64>,
    pub context_json: Option<String>,
    pub input_json: Option<String>,
    pub artifact_json: Option<String>,
    pub result_ref: Option<String>,
    pub last_error: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct VoiceJob {
    pub id: String,
    pub target_type: String,
    pub target_id: String,
    pub product_line: String,
    pub voice_kind: String,
    pub text: String,
    pub file_prefix: String,
    pub status: String,
    pub priority: Option<i64>,
    pub run_after: Option<i64>,
    pub lease_owner: Option<String>,
    pub lease_token: Option<String>,
    pub lease_expires_at: Option<i64>,
    pub attempt_count: Option<i64>,
    pub max_attempts: Option<i64>,
    pub audio_url: Option<String>,
    pub duration_sec: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentJobRequest {
    pub id: Option<String>,
    pub job_type: String,
    pub priority: Option<i64>,
    pub run_after: Option<i64>,
    pub max_attempts: Option<i64>,
    pub context: Option<Value>,
    pub input: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct LeaseAgentJobRequest {
    pub job_id: Option<String>,
    pub agent_id: String,
    pub job_types: Option<Vec<String>>,
    pub lease_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub lease_token: String,
    pub lease_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitAgentJobRequest {
    pub lease_token: String,
    pub artifact: Value,
}

#[derive(Debug, Deserialize)]
pub struct FailJobRequest {
    pub lease_token: String,
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub struct LeaseVoiceJobRequest {
    pub worker_id: String,
    pub voice_kinds: Option<Vec<String>>,
    pub lease_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CompleteVoiceJobRequest {
    pub lease_token: String,
    pub audio_url: String,
    pub duration_sec: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct VoiceListQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RepairVoiceJobsRequest {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, Default)]
pub struct RepairVoiceJobsResponse {
    pub created: usize,
    pub backfilled_completed: usize,
    pub skipped_active: usize,
    pub skipped_recent_failures: usize,
    pub skipped_missing_text: usize,
    pub scanned: usize,
    pub voice_job_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SubmitAgentJobResponse {
    pub job_id: String,
    pub status: String,
    pub result_ref: Option<String>,
    pub voice_job_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SourceArtifact {
    url: String,
    title: Option<String>,
    summary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RadioEpisodeArtifact {
    title: String,
    category: String,
    script: String,
    audio_script: Option<String>,
    publish_time: Option<i64>,
    original_url: Option<String>,
    tags: Option<Vec<String>>,
    sources: Option<Vec<SourceArtifact>>,
}

#[derive(Debug, Deserialize)]
struct ReadingArticleArtifact {
    title: String,
    subtitle: Option<String>,
    source_name: Option<String>,
    source_url: Option<String>,
    original_url: String,
    canonical_url: Option<String>,
    publish_time: Option<i64>,
    reader_markdown: Option<String>,
    plain_text: Option<String>,
    compressed_markdown: Option<String>,
    audio_script: Option<String>,
    key_points: Option<Vec<String>>,
    reading_time_min: Option<i64>,
    quality_score: Option<i64>,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct WeeklyDigestArtifact {
    // Explicit repair of an existing silent digest; compare against reviewed text.
    replace_digest_id: Option<String>,
    expected_audio_script: Option<String>,
    title: String,
    week_start: i64,
    week_end: i64,
    digest_markdown: Option<String>,
    audio_script: Option<String>,
    included_item_ids: Option<Vec<String>>,
    themes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct LoopPreferenceArtifact {
    post_id: String,
    user_id: String,
    status: Option<String>,
    error: Option<String>,
    signals: Option<Vec<PreferenceSignalArtifact>>,
}

#[derive(Debug, Deserialize)]
struct PreferenceSignalArtifact {
    content: String,
    signal_type: Option<String>,
    polarity: Option<String>,
    confidence: Option<f32>,
    strength: Option<f32>,
    evidence: Option<String>,
}

fn has_internal_auth(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get("X-NEXUS-KEY")
        .and_then(|value| value.to_str().ok())
        == Some(state.api_key.as_str())
}

fn validate_label(value: &str, allowed: &[&str], field: &str) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!("invalid {field} '{value}'"))
    }
}

fn validate_agent_job_type(job_type: &str) -> Result<(), String> {
    validate_label(job_type, ALLOWED_AGENT_JOB_TYPES, "job_type")
}

fn trim_required(value: &str, field: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("{field} is required"))
    } else {
        Ok(trimmed.to_string())
    }
}

fn validate_text_length(value: &str, field: &str) -> Result<(), String> {
    if value.len() > MAX_TEXT_LENGTH {
        Err(format!("{field} exceeds {MAX_TEXT_LENGTH} bytes"))
    } else {
        Ok(())
    }
}

fn clamp_lease_seconds(value: Option<i64>) -> i64 {
    value
        .unwrap_or(DEFAULT_LEASE_SECONDS)
        .clamp(60, MAX_LEASE_SECONDS)
}

fn truncate_error(value: &str) -> String {
    value.chars().take(MAX_ERROR_LENGTH).collect()
}

fn encode_json(value: &Option<Value>) -> Option<String> {
    value.as_ref().map(Value::to_string)
}

fn encode_string_vec(values: Option<Vec<String>>) -> Option<String> {
    let values = values?
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        serde_json::to_string(&values).ok()
    }
}

fn validate_url(url: &str, field: &str) -> Result<(), String> {
    let parsed = url
        .trim()
        .parse::<axum::http::Uri>()
        .map_err(|_| format!("{field} must be an absolute HTTP(S) URL"))?;
    if !matches!(parsed.scheme_str(), Some("http" | "https"))
        || parsed.host().is_none_or(|h| h.is_empty())
        || parsed.authority().is_some_and(|a| a.as_str().contains('@'))
    {
        return Err(format!(
            "{field} must be an absolute HTTP(S) URL without credentials"
        ));
    }
    Ok(())
}

fn validate_audio_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("audio_url is required".to_string());
    }
    if trimmed.starts_with("/audio/")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        Ok(())
    } else {
        Err("audio_url must start with /audio/, http://, or https://".to_string())
    }
}

fn bad_request(message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message.into() })),
    )
        .into_response()
}

fn unauthorized() -> axum::response::Response {
    (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response()
}

async fn record_agent_event(
    state: &AppState,
    job_id: &str,
    event_type: &str,
    actor: Option<&str>,
    message: Option<&str>,
    payload: Option<Value>,
) {
    let now = chrono::Utc::now().timestamp();
    let payload_json = payload.map(|value| value.to_string());
    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO agent_job_events (
            id, job_id, event_type, actor, message, payload_json, created_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(job_id)
    .bind(event_type)
    .bind(actor)
    .bind(message)
    .bind(payload_json)
    .bind(now)
    .execute(&state.db)
    .await
    {
        tracing::warn!("failed to record agent job event {}: {}", job_id, e);
    }
}

pub async fn capabilities(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }

    Json(json!({
        "schema_version": 2,
        "features": ["exact_job_lease", "atomic_content_submit", "idempotent_submit"],
        "agent_job_types": ALLOWED_AGENT_JOB_TYPES,
        "agent_statuses": ALLOWED_AGENT_STATUSES,
        "voice_target_types": ALLOWED_VOICE_TARGET_TYPES,
        "voice_statuses": ALLOWED_VOICE_STATUSES,
        "artifacts": {
            "radio_program": {"required":["title","opening","closing","sections"],"slot":"morning-program or evening-program","sections":"full validated radio_episode artifacts; no repeated greeting or sign-off","multi_host":true},
            "radio_episode": {
                "required": ["title", "category", "script"],
                "optional": ["publish_time", "original_url", "tags", "sources", "audio_script"],
                "external_required": ["sources", "stories", "editorial"],
                "minimum_independent_stories": 1,
                "story_tiers": ["major", "brief"],
                "coverage_required": true,
                "external_policy": "Cover all qualifying same-category events after deduplication; major stories plus one-sentence briefs, with reviewed eligible-event coverage and full spoken coverage"
            },
            "reading_article": {
                "required": ["title", "original_url"],
                "optional": ["subtitle", "source_name", "source_url", "canonical_url", "reader_markdown", "plain_text", "compressed_markdown", "audio_script", "key_points", "quality_score", "tags"]
            },
            "weekly_digest": {
                "required": ["title", "week_start", "week_end"],
                "optional": ["digest_markdown", "audio_script", "included_item_ids", "themes"]
            },
            "loop_preference_extraction": {
                "required": ["post_id", "user_id"],
                "optional": ["status", "error", "signals"]
            },
            "holiday_calendar_update": {
                "contract": "submit structured JSON; stored as job artifact for the holiday sync worker"
            }
        }
    }))
    .into_response()
}

pub async fn create_agent_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateAgentJobRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    if let Err(e) = validate_agent_job_type(payload.job_type.trim()) {
        return bad_request(e);
    }

    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let now = chrono::Utc::now().timestamp();
    let max_attempts = payload.max_attempts.unwrap_or(3).clamp(1, 20);
    let result = sqlx::query(
        r#"
        INSERT INTO agent_jobs (
            id, job_type, status, priority, run_after, attempt_count, max_attempts,
            context_json, input_json, created_at, updated_at
        )
        VALUES (?, ?, 'queued', ?, ?, 0, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO NOTHING
        "#,
    )
    .bind(&id)
    .bind(payload.job_type.trim())
    .bind(payload.priority.unwrap_or(0))
    .bind(payload.run_after.unwrap_or(now))
    .bind(max_attempts)
    .bind(encode_json(&payload.context))
    .bind(encode_json(&payload.input))
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await;

    match result {
        Ok(result) if result.rows_affected() == 0 => {
            match sqlx::query_as::<_, AgentJob>("SELECT * FROM agent_jobs WHERE id = ?")
                .bind(&id)
                .fetch_one(&state.db)
                .await
            {
                Ok(job) if job.job_type == payload.job_type.trim() => {
                    Json(json!({"id":id,"status":job.status})).into_response()
                }
                Ok(_) => StatusCode::CONFLICT.into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }
        Ok(_) => {
            record_agent_event(&state, &id, "created", Some("internal"), None, None).await;
            (
                StatusCode::CREATED,
                Json(json!({ "id": id, "status": "queued" })),
            )
                .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn lease_agent_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LeaseAgentJobRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    let agent_id = match trim_required(&payload.agent_id, "agent_id") {
        Ok(value) => value,
        Err(e) => return bad_request(e),
    };
    if let Some(types) = &payload.job_types {
        for job_type in types {
            if let Err(e) = validate_agent_job_type(job_type.trim()) {
                return bad_request(e);
            }
        }
    }

    let now = chrono::Utc::now().timestamp();
    let lease_seconds = clamp_lease_seconds(payload.lease_seconds);
    let lease_expires_at = now + lease_seconds;
    let candidates = match sqlx::query_as::<_, AgentJob>(
        r#"
        SELECT * FROM agent_jobs
        WHERE (
            status = 'queued'
            OR (status = 'leased' AND COALESCE(lease_expires_at, 0) < ?)
        )
          AND COALESCE(run_after, 0) <= ?
          AND COALESCE(attempt_count, 0) < COALESCE(max_attempts, 3)
          AND (? IS NULL OR id = ?)
          AND (? IS NULL OR job_type IN (SELECT value FROM json_each(?)))
        ORDER BY priority DESC, created_at ASC
        LIMIT 1
        "#,
    )
    .bind(now)
    .bind(now)
    .bind(&payload.job_id)
    .bind(&payload.job_id)
    .bind(
        payload
            .job_types
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap()),
    )
    .bind(
        payload
            .job_types
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap()),
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let Some(job) = candidates.into_iter().next() else {
        return Json(json!({ "job": null })).into_response();
    };
    let lease_token = Uuid::new_v4().to_string();
    let result = sqlx::query(
        r#"
        UPDATE agent_jobs
        SET status = 'leased',
            lease_owner = ?,
            lease_token = ?,
            lease_expires_at = ?,
            attempt_count = COALESCE(attempt_count, 0) + 1,
            updated_at = ?
        WHERE id = ?
          AND (
              status = 'queued'
              OR (status = 'leased' AND COALESCE(lease_expires_at, 0) < ?)
          )
          AND COALESCE(run_after, 0) <= ?
          AND COALESCE(attempt_count, 0) < COALESCE(max_attempts, 3)
        "#,
    )
    .bind(&agent_id)
    .bind(&lease_token)
    .bind(lease_expires_at)
    .bind(now)
    .bind(&job.id)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await;

    match result {
        Ok(result) if result.rows_affected() > 0 => {
            record_agent_event(
                &state,
                &job.id,
                "leased",
                Some(&agent_id),
                None,
                Some(json!({ "lease_expires_at": lease_expires_at })),
            )
            .await;
            let mut leased = job;
            leased.status = "leased".to_string();
            leased.lease_owner = Some(agent_id);
            leased.lease_token = Some(lease_token);
            leased.lease_expires_at = Some(lease_expires_at);
            Json(json!({ "job": leased })).into_response()
        }
        Ok(_) => Json(json!({ "job": null })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn get_agent_job_context(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }

    let job = sqlx::query_as::<_, AgentJob>("SELECT * FROM agent_jobs WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await;

    match job {
        Ok(Some(job)) => Json(json!({
            "job": job,
            "context": parse_json(job.context_json.as_deref()),
            "input": parse_json(job.input_json.as_deref())
        }))
        .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn heartbeat_agent_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    let now = chrono::Utc::now().timestamp();
    let lease_expires_at = now + clamp_lease_seconds(payload.lease_seconds);
    let result = sqlx::query(
        r#"
        UPDATE agent_jobs
        SET lease_expires_at = ?, updated_at = ?
        WHERE id = ? AND lease_token = ? AND status = 'leased' AND lease_expires_at >= ?
        "#,
    )
    .bind(lease_expires_at)
    .bind(now)
    .bind(&id)
    .bind(&payload.lease_token)
    .bind(now)
    .execute(&state.db)
    .await;

    match result {
        Ok(result) if result.rows_affected() > 0 => {
            Json(json!({ "id": id, "lease_expires_at": lease_expires_at })).into_response()
        }
        Ok(_) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn validate_radio_digest(artifact: &Value) -> Result<(), String> {
    use std::collections::HashSet;
    fn compact(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }
    fn urls(value: &Value, field: Option<&str>) -> HashSet<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| {
                let raw = match field {
                    Some(f) => v[f].as_str(),
                    None => v.as_str(),
                }?;
                Some(raw.split('#').next().unwrap_or(raw).to_string())
            })
            .collect()
    }
    let stories = artifact["stories"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("Radio category digest requires all qualifying stories, at least one")?;
    let sources = urls(&artifact["sources"], Some("url"));
    let claims = urls(&artifact["editorial"]["claims"], Some("source_url"));
    let manuscript = compact(artifact["script"].as_str().unwrap_or(""));
    let spoken = compact(
        artifact["audio_script"]
            .as_str()
            .unwrap_or(artifact["script"].as_str().unwrap_or("")),
    );
    let (mut keys, mut titles, mut bodies) = (HashSet::new(), HashSet::new(), HashSet::new());
    for story in stories {
        let key = compact(story["event_key"].as_str().unwrap_or("")).to_lowercase();
        let title = compact(story["title"].as_str().unwrap_or("")).to_lowercase();
        let body = compact(story["script"].as_str().unwrap_or(""));
        let audio = compact(
            story["audio_script"]
                .as_str()
                .unwrap_or(story["script"].as_str().unwrap_or("")),
        );
        if key.is_empty()
            || title.is_empty()
            || !keys.insert(key)
            || !titles.insert(title)
            || !bodies.insert(body.clone())
        {
            return Err("Radio stories require distinct event_key, title and body".into());
        }
        let minimum = match story["tier"].as_str() {
            Some("major") => 60,
            Some("brief") => 10,
            _ => return Err("Radio story tier must be major or brief".into()),
        };
        if body.chars().count() < minimum
            || audio.chars().count() < minimum
            || !manuscript.contains(&body)
            || !spoken.contains(&audio)
        {
            return Err("each Radio story must meet its tier length and appear in displayed and spoken manuscripts".into());
        }
        let refs = story["source_urls"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or("each Radio story needs source_urls")?;
        for url in refs {
            validate_url(url.as_str().unwrap_or(""), "story.source_url")?;
        }
        let refs = urls(&story["source_urls"], None);
        if !refs.is_subset(&sources) || refs.is_disjoint(&claims) {
            return Err(
                "Radio story sources must be declared and backed by editorial claim evidence"
                    .into(),
            );
        }
    }
    let coverage = &artifact["editorial"]["radio_coverage"];
    let eligible = coverage["eligible_event_keys"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("Radio requires reviewed coverage with eligible_event_keys")?;
    let mut expected = HashSet::new();
    for key in eligible {
        let key = compact(key.as_str().unwrap_or("")).to_lowercase();
        if key.is_empty() || !expected.insert(key) {
            return Err("Radio coverage event keys must be unique".into());
        }
    }
    if coverage["reviewed"] != true || expected != keys {
        return Err("Radio coverage must match every eligible event exactly once".into());
    }
    Ok(())
}

fn validate_external_artifact(job: &AgentJob, artifact: &Value) -> Result<(), String> {
    if job.job_type == "radio_program" {
        return radio_program::validate(job, artifact);
    }
    if parse_json(job.context_json.as_deref())["production_mode"] != "external_agent"
        || job.job_type == "loop_preference_extraction"
    {
        return Ok(());
    }
    let review = &artifact["editorial"];
    if review["ready"] != true || review["dedup_checked"] != true || review["language"] != "zh-CN" {
        return Err(
            "external artifacts require a completed Chinese editorial review and duplicate check"
                .into(),
        );
    }
    let claims = review["claims"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("editorial.claims must contain source evidence")?;
    for claim in claims {
        trim_required(claim["claim"].as_str().unwrap_or(""), "claim")?;
        trim_required(claim["evidence"].as_str().unwrap_or(""), "evidence")?;
        validate_url(
            claim["source_url"].as_str().unwrap_or(""),
            "claim.source_url",
        )?;
    }
    let spoken =
        artifact[if job.job_type == "radio_episode" && artifact["audio_script"].is_string() {
            "audio_script"
        } else if job.job_type == "radio_episode" {
            "script"
        } else {
            "audio_script"
        }]
        .as_str()
        .unwrap_or("");
    let chars = spoken.chars().filter(|c| !c.is_whitespace()).count();
    let chinese = spoken
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    let minimum = if job.job_type == "radio_episode" {
        10
    } else {
        60
    };
    if chars < minimum || chars > 6500 || chinese * 5 < chars {
        return Err(
            "external audio must be substantive Chinese speech within the job-specific minimum and 6500-character maximum"
                .into(),
        );
    }
    if [
        "```", "&mdash;", "&nbsp;", "$1", "$2", "http://", "https://", "![",
    ]
    .iter()
    .any(|s| spoken.contains(s))
    {
        return Err("audio manuscript contains markup or extraction debris".into());
    }
    if job.job_type == "reading_article" {
        for field in ["reader_markdown", "compressed_markdown"] {
            trim_required(artifact[field].as_str().unwrap_or(""), field)?;
        }
    }
    if job.job_type == "radio_episode" {
        validate_radio_digest(artifact)?;
    }
    if let Some(media) = review.get("media") {
        for item in media.as_array().ok_or("editorial.media must be an array")? {
            validate_url(item["url"].as_str().unwrap_or(""), "media.url")?;
            let status = item["status"].as_str().unwrap_or("");
            validate_label(
                status,
                &["inspected", "transcript_only", "unavailable"],
                "media.status",
            )?;
            if status == "inspected" {
                trim_required(
                    item["observation"].as_str().unwrap_or(""),
                    "media.observation",
                )?;
            }
        }
    }
    Ok(())
}

pub async fn submit_agent_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<SubmitAgentJobRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    // Acquire SQLite's writer lock before reading ownership. Concurrent submits cannot
    // both publish; product rows, voice jobs and completion commit together.
    if let Err(e) = sqlx::query("UPDATE agent_jobs SET updated_at = updated_at WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let job = match sqlx::query_as::<_, AgentJob>("SELECT * FROM agent_jobs WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(job)) => job,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if job.lease_token.as_deref() != Some(payload.lease_token.as_str()) {
        return StatusCode::CONFLICT.into_response();
    }
    if job.status == "completed" {
        if parse_json(job.artifact_json.as_deref()) != payload.artifact {
            return StatusCode::CONFLICT.into_response();
        }
        let ids = match voice_ids_for_result(&mut tx, job.result_ref.as_deref()).await {
            Ok(ids) => ids,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        };
        return Json(SubmitAgentJobResponse {
            job_id: id,
            status: "completed".into(),
            result_ref: job.result_ref,
            voice_job_ids: ids,
        })
        .into_response();
    }
    if job.status != "leased" || job.lease_expires_at.unwrap_or(0) < chrono::Utc::now().timestamp()
    {
        return StatusCode::CONFLICT.into_response();
    }
    if let Err(e) = validate_external_artifact(&job, &payload.artifact) {
        return bad_request(e);
    }
    let priority = job
        .priority
        .unwrap_or(0)
        .max(DEFAULT_CONTENT_VOICE_PRIORITY);
    let published = match job.job_type.as_str() {
        "radio_program" => radio_program::publish(&mut tx, &job, &payload.artifact, priority).await,
        "radio_episode" => publish_radio_episode(&mut tx, &payload.artifact, priority).await,
        "reading_article" => publish_reading_article(&mut tx, &payload.artifact, priority).await,
        "weekly_digest" => publish_weekly_digest(&mut tx, &payload.artifact, priority).await,
        "loop_preference_extraction" => {
            apply_loop_preference_artifact(&state, &mut tx, &payload.artifact).await
        }
        "holiday_calendar_update" => Ok(PublishResult {
            result_ref: Some(format!("holiday_calendar_update:{}", job.id)),
            voice_job_ids: vec![],
        }),
        _ => Err("unsupported job type".into()),
    };
    let result = match published {
        Ok(result) => result,
        Err(e) => {
            // Roll back partial writes and preserve the live lease for a corrected draft.
            let _ = tx.rollback().await;
            return bad_request(e);
        }
    };
    let now = chrono::Utc::now().timestamp();
    let update=sqlx::query("UPDATE agent_jobs SET status='completed', artifact_json=?, result_ref=?, lease_owner=NULL, lease_expires_at=NULL, updated_at=?, completed_at=? WHERE id=?")
        .bind(payload.artifact.to_string()).bind(&result.result_ref).bind(now).bind(now).bind(&id).execute(&mut *tx).await;
    if let Err(e) = update {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    if let Err(e) = tx.commit().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    record_agent_event(
        &state,
        &id,
        "submitted",
        job.lease_owner.as_deref(),
        None,
        Some(json!({"result_ref":result.result_ref,"voice_job_ids":result.voice_job_ids})),
    )
    .await;
    Json(SubmitAgentJobResponse {
        job_id: id,
        status: "completed".into(),
        result_ref: result.result_ref,
        voice_job_ids: result.voice_job_ids,
    })
    .into_response()
}

async fn voice_ids_for_result(
    conn: &mut sqlx::SqliteConnection,
    result_ref: Option<&str>,
) -> Result<Vec<String>, String> {
    let Some((kind, id)) = result_ref.and_then(|s| s.split_once(':')) else {
        return Ok(vec![]);
    };
    sqlx::query_scalar(
        "SELECT id FROM voice_jobs WHERE target_type=? AND target_id=? ORDER BY created_at,id",
    )
    .bind(kind)
    .bind(id)
    .fetch_all(conn)
    .await
    .map_err(|e| e.to_string())
}

pub async fn fail_agent_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<FailJobRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    let err = truncate_error(&payload.error);
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        r#"
        UPDATE agent_jobs
        SET status = CASE
                WHEN COALESCE(attempt_count, 0) >= COALESCE(max_attempts, 3) THEN 'failed'
                ELSE 'queued'
            END,
            lease_token = NULL,
            lease_owner = NULL,
            lease_expires_at = NULL,
            last_error = ?,
            updated_at = ?
        WHERE id = ? AND lease_token = ? AND status = 'leased' AND lease_expires_at >= ?
        "#,
    )
    .bind(&err)
    .bind(now)
    .bind(&id)
    .bind(&payload.lease_token)
    .bind(now)
    .execute(&state.db)
    .await;

    match result {
        Ok(result) if result.rows_affected() > 0 => {
            record_agent_event(&state, &id, "failed", None, Some(&err), None).await;
            StatusCode::OK.into_response()
        }
        Ok(_) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

struct PublishResult {
    result_ref: Option<String>,
    voice_job_ids: Vec<String>,
}

async fn publish_radio_episode(
    conn: &mut sqlx::SqliteConnection,
    artifact: &Value,
    voice_priority: i64,
) -> Result<PublishResult, String> {
    let artifact: RadioEpisodeArtifact =
        serde_json::from_value(artifact.clone()).map_err(|e| e.to_string())?;
    let title = trim_required(&artifact.title, "title")?;
    if title.len() > MAX_TITLE_LENGTH {
        return Err(format!("title exceeds {MAX_TITLE_LENGTH} characters"));
    }
    let category = trim_required(&artifact.category, "category")?;
    let script = trim_required(&artifact.script, "script")?;
    validate_text_length(&script, "script")?;
    let spoken = artifact.audio_script.as_deref().unwrap_or(&script);
    trim_required(spoken, "audio_script")?;
    validate_text_length(spoken, "audio_script")?;
    if let Some(original_url) = artifact.original_url.as_deref() {
        validate_url(original_url, "original_url")?;
    }

    let item_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let publish_time = artifact.publish_time.unwrap_or(now);
    let original_url = artifact.original_url.clone().or_else(|| {
        artifact
            .sources
            .as_ref()
            .and_then(|sources| sources.first())
            .map(|source| source.url.clone())
    });
    let tags = encode_string_vec(artifact.tags);

    sqlx::query(
        r#"
        INSERT INTO items (
            id, title, summary, original_url, cover_image_url, audio_url,
            publish_time, created_at, duration_sec, status, category, tags
        )
        VALUES (?, ?, ?, ?, NULL, NULL, ?, ?, NULL, 'published', ?, ?)
        "#,
    )
    .bind(&item_id)
    .bind(&title)
    .bind(&script)
    .bind(&original_url)
    .bind(publish_time)
    .bind(now)
    .bind(&category)
    .bind(&tags)
    .execute(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;

    if let Some(sources) = artifact.sources {
        insert_item_sources(conn, &item_id, sources).await?;
    }

    let voice_job_id = insert_voice_job_on(
        conn,
        NewVoiceJob {
            target_type: "radio_item",
            target_id: &item_id,
            product_line: "radio",
            voice_kind: "radio_episode",
            text: spoken,
            file_prefix: "radio_agent",
            priority: voice_priority,
        },
    )
    .await?;

    Ok(PublishResult {
        result_ref: Some(format!("radio_item:{item_id}")),
        voice_job_ids: vec![voice_job_id],
    })
}

async fn insert_item_sources(
    conn: &mut sqlx::SqliteConnection,
    item_id: &str,
    sources: Vec<SourceArtifact>,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    for source in sources {
        validate_url(&source.url, "source.url")?;
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO item_sources (
                id, item_id, source_url, source_title, source_summary, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(item_id)
        .bind(source.url)
        .bind(source.title)
        .bind(source.summary)
        .bind(now)
        .execute(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn publish_reading_article(
    conn: &mut sqlx::SqliteConnection,
    artifact: &Value,
    voice_priority: i64,
) -> Result<PublishResult, String> {
    let artifact: ReadingArticleArtifact =
        serde_json::from_value(artifact.clone()).map_err(|e| e.to_string())?;
    let title = trim_required(&artifact.title, "title")?;
    let original_url = trim_required(&artifact.original_url, "original_url")?;
    validate_url(&original_url, "original_url")?;
    if let Some(source_url) = artifact.source_url.as_deref() {
        validate_url(source_url, "source_url")?;
    }
    if let Some(canonical_url) = artifact.canonical_url.as_deref() {
        validate_url(canonical_url, "canonical_url")?;
    }
    for (field, value) in [
        ("reader_markdown", artifact.reader_markdown.as_deref()),
        ("plain_text", artifact.plain_text.as_deref()),
        (
            "compressed_markdown",
            artifact.compressed_markdown.as_deref(),
        ),
        ("audio_script", artifact.audio_script.as_deref()),
    ] {
        if let Some(value) = value {
            validate_text_length(value, field)?;
        }
    }

    let item_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let publish_time = artifact.publish_time.unwrap_or(now);
    let tags = encode_string_vec(artifact.tags);
    let key_points_json = encode_string_vec(artifact.key_points);
    let quality_score = artifact.quality_score.unwrap_or(0).clamp(0, 10);
    let has_audio_text = artifact
        .audio_script
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());

    sqlx::query(
        r#"
        INSERT INTO feed_items (
            id, product_line, item_type, primary_mode, title, subtitle, source_name,
            source_url, original_url, canonical_url, content_hash, publish_time,
            created_at, updated_at, has_audio, audio_url, reading_time_min,
            duration_sec, quality_score, tags, status
        )
        VALUES (?, 'curated_feed', 'article', 'read', ?, ?, ?, ?, ?, ?, NULL,
            ?, ?, ?, 0, NULL, ?, NULL, ?, ?, 'published')
        "#,
    )
    .bind(&item_id)
    .bind(&title)
    .bind(&artifact.subtitle)
    .bind(&artifact.source_name)
    .bind(&artifact.source_url)
    .bind(&original_url)
    .bind(&artifact.canonical_url)
    .bind(publish_time)
    .bind(now)
    .bind(now)
    .bind(artifact.reading_time_min)
    .bind(quality_score)
    .bind(&tags)
    .execute(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        r#"
        INSERT OR REPLACE INTO feed_item_contents (
            item_id, original_html, reader_markdown, plain_text, compressed_markdown,
            audio_script, key_points_json, created_at, updated_at
        )
        VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&item_id)
    .bind(&artifact.reader_markdown)
    .bind(&artifact.plain_text)
    .bind(&artifact.compressed_markdown)
    .bind(&artifact.audio_script)
    .bind(&key_points_json)
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;

    let mut voice_job_ids = Vec::new();
    if has_audio_text {
        let audio_text = artifact.audio_script.as_deref().unwrap_or_default();
        let voice_job_id = insert_voice_job_on(
            conn,
            NewVoiceJob {
                target_type: "feed_item",
                target_id: &item_id,
                product_line: "curated_feed",
                voice_kind: "curated_article",
                text: audio_text,
                file_prefix: "curated_article",
                priority: voice_priority,
            },
        )
        .await?;
        voice_job_ids.push(voice_job_id);
    }

    Ok(PublishResult {
        result_ref: Some(format!("feed_item:{item_id}")),
        voice_job_ids,
    })
}

async fn publish_weekly_digest(
    conn: &mut sqlx::SqliteConnection,
    artifact: &Value,
    voice_priority: i64,
) -> Result<PublishResult, String> {
    let artifact: WeeklyDigestArtifact =
        serde_json::from_value(artifact.clone()).map_err(|e| e.to_string())?;
    let title = trim_required(&artifact.title, "title")?;
    if artifact.week_end < artifact.week_start {
        return Err("week_end must be greater than or equal to week_start".to_string());
    }
    for (field, value) in [
        ("digest_markdown", artifact.digest_markdown.as_deref()),
        ("audio_script", artifact.audio_script.as_deref()),
    ] {
        if let Some(value) = value {
            validate_text_length(value, field)?;
        }
    }

    if let Some(ids) = artifact.included_item_ids.as_ref() {
        let unique = ids.iter().collect::<std::collections::HashSet<_>>();
        if unique.len() != ids.len() {
            return Err("weekly references must be distinct".into());
        }
        for id in ids {
            let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM feed_items WHERE id=? AND item_type='article' AND status='published' AND publish_time BETWEEN ? AND ?")
                .bind(id).bind(artifact.week_start).bind(artifact.week_end).fetch_one(&mut *conn).await.map_err(|e|e.to_string())?;
            if count != 1 {
                return Err("weekly reference is missing or outside its date window".into());
            }
        }
    }
    if let Some(id) = artifact.replace_digest_id.as_deref() {
        let expected = artifact
            .expected_audio_script
            .as_deref()
            .ok_or("weekly repair requires expected_audio_script")?;
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT feed_item_id FROM weekly_digests WHERE id=? AND week_start=? AND week_end=? AND audio_script=? AND COALESCE(audio_url,'')='' AND status='published'"
        ).bind(id).bind(artifact.week_start).bind(artifact.week_end).bind(expected)
            .fetch_optional(&mut *conn).await.map_err(|e|e.to_string())?;
        let (feed_id,) = existing.ok_or(
            "weekly repair target changed, has audio, or does not match the reviewed window/script",
        )?;
        let script = artifact
            .audio_script
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or("weekly repair requires a replacement audio_script")?;
        // Fence the old worker before changing its target; all changes share the
        // submitting agent's transaction, so stale completions cannot win.
        sqlx::query("UPDATE voice_jobs SET status='cancelled',lease_token=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error='Superseded by reviewed weekly manuscript',updated_at=? WHERE target_type='weekly_digest' AND target_id=? AND status IN ('queued','leased')")
            .bind(chrono::Utc::now().timestamp()).bind(id).execute(&mut *conn).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE weekly_digests SET title=?,digest_markdown=?,audio_script=?,included_item_ids_json=?,themes_json=? WHERE id=?")
            .bind(&title).bind(&artifact.digest_markdown).bind(script)
            .bind(encode_string_vec(artifact.included_item_ids)).bind(encode_string_vec(artifact.themes))
            .bind(id).execute(&mut *conn).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE feed_items SET title=?,updated_at=? WHERE id=?")
            .bind(&title)
            .bind(chrono::Utc::now().timestamp())
            .bind(&feed_id)
            .execute(&mut *conn)
            .await
            .map_err(|e| e.to_string())?;
        let voice_id = insert_voice_job_on(
            conn,
            NewVoiceJob {
                target_type: "weekly_digest",
                target_id: id,
                product_line: "curated_feed",
                voice_kind: "curated_weekly",
                text: script,
                file_prefix: "curated_weekly",
                priority: voice_priority,
            },
        )
        .await?;
        return Ok(PublishResult {
            result_ref: Some(format!("weekly_digest:{id}")),
            voice_job_ids: vec![voice_id],
        });
    }
    let digest_id = Uuid::new_v4().to_string();
    let feed_item_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let included_json = encode_string_vec(artifact.included_item_ids);
    let themes_json = encode_string_vec(artifact.themes);

    sqlx::query(
        r#"
        INSERT INTO feed_items (
            id, product_line, item_type, primary_mode, title, subtitle, source_name,
            source_url, original_url, canonical_url, content_hash, publish_time,
            created_at, updated_at, has_audio, audio_url, reading_time_min,
            duration_sec, quality_score, tags, status
        )
        VALUES (?, 'curated_feed', 'weekly_digest', 'listen', ?, ?, 'FreshLoop',
            NULL, NULL, NULL, NULL, ?, ?, ?, 0, NULL, NULL, NULL, NULL, NULL, 'published')
        "#,
    )
    .bind(&feed_item_id)
    .bind(&title)
    .bind(Some(format!(
        "{} - {}",
        artifact.week_start, artifact.week_end
    )))
    .bind(artifact.week_end)
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        r#"
        INSERT INTO weekly_digests (
            id, feed_item_id, week_start, week_end, title, digest_markdown,
            audio_script, audio_url, duration_sec, included_item_ids_json,
            themes_json, created_at, status
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, 'published')
        "#,
    )
    .bind(&digest_id)
    .bind(&feed_item_id)
    .bind(artifact.week_start)
    .bind(artifact.week_end)
    .bind(&title)
    .bind(&artifact.digest_markdown)
    .bind(&artifact.audio_script)
    .bind(&included_json)
    .bind(&themes_json)
    .bind(now)
    .execute(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;

    let mut voice_job_ids = Vec::new();
    if artifact
        .audio_script
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
    {
        let voice_job_id = insert_voice_job_on(
            conn,
            NewVoiceJob {
                target_type: "weekly_digest",
                target_id: &digest_id,
                product_line: "curated_feed",
                voice_kind: "curated_weekly",
                text: artifact.audio_script.as_deref().unwrap_or_default(),
                file_prefix: "curated_weekly",
                priority: voice_priority,
            },
        )
        .await?;
        voice_job_ids.push(voice_job_id);
    }

    Ok(PublishResult {
        result_ref: Some(format!("weekly_digest:{digest_id}")),
        voice_job_ids,
    })
}

async fn apply_loop_preference_artifact(
    state: &AppState,
    conn: &mut sqlx::SqliteConnection,
    artifact: &Value,
) -> Result<PublishResult, String> {
    let artifact: LoopPreferenceArtifact =
        serde_json::from_value(artifact.clone()).map_err(|e| e.to_string())?;
    let post_id = trim_required(&artifact.post_id, "post_id")?;
    let user_id = trim_required(&artifact.user_id, "user_id")?;
    let signals = artifact.signals.unwrap_or_default();
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM loop_posts WHERE id=? AND user_id=?")
            .bind(&post_id)
            .bind(&user_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| e.to_string())?;
    if exists != 1 {
        return Err("Loop post/user not found".into());
    }
    let status = artifact.status.clone().unwrap_or_else(|| {
        if signals.is_empty() {
            "skipped"
        } else {
            "processed"
        }
        .into()
    });
    validate_label(
        &status,
        &["processed", "skipped", "failed"],
        "preference_status",
    )?;
    for signal in &signals {
        trim_required(&signal.content, "signal.content")?;
    }
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let namespace = format!("user:{user_id}");

    let mut written = 0usize;
    for (index, signal) in signals.into_iter().enumerate() {
        let content = trim_required(&signal.content, "signal.content")?;
        let mut metadata = HashMap::new();
        metadata.insert("post_id".to_string(), post_id.clone());
        if let Some(signal_type) = signal.signal_type {
            metadata.insert("signal_type".to_string(), signal_type);
        }
        if let Some(polarity) = signal.polarity {
            metadata.insert("polarity".to_string(), polarity);
        }
        if let Some(evidence) = signal.evidence {
            metadata.insert(
                "evidence".to_string(),
                evidence.chars().take(1_000).collect(),
            );
        }

        let mut entry = MemoryEntry::new(
            format!("loop-preference:{post_id}:{index}"),
            MemoryType::PreferenceSignal,
            content,
            now,
            None,
        );
        let strength = signal.strength.unwrap_or(1.0).clamp(0.1, 5.0);
        entry.base_strength = strength;
        entry.current_strength = strength;
        entry.namespace = Some(namespace.clone());
        entry.provenance = Provenance::LlmExtracted;
        entry.confidence = signal.confidence.unwrap_or(1.0).clamp(0.0, 1.0);
        entry.source_ref = Some(format!("loop_post:{post_id}"));
        entry.metadata = metadata;

        state
            .memory_store
            .store(entry)
            .await
            .map_err(|e| e.to_string())?;
        written += 1;
    }

    let extracted_at = if status == "processed" || status == "skipped" {
        Some(chrono::Utc::now().timestamp())
    } else {
        None
    };
    sqlx::query(
        r#"
        UPDATE loop_posts
        SET preference_status = ?, preference_extracted_at = ?, preference_error = ?, updated_at = ?
        WHERE id = ? AND user_id = ?
        "#,
    )
    .bind(&status)
    .bind(extracted_at)
    .bind(artifact.error.as_deref().map(truncate_error))
    .bind(chrono::Utc::now().timestamp())
    .bind(&post_id)
    .bind(&user_id)
    .execute(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;

    Ok(PublishResult {
        result_ref: Some(format!("loop_post:{post_id}:signals:{written}")),
        voice_job_ids: Vec::new(),
    })
}

struct NewVoiceJob<'a> {
    target_type: &'a str,
    target_id: &'a str,
    product_line: &'a str,
    voice_kind: &'a str,
    text: &'a str,
    file_prefix: &'a str,
    priority: i64,
}

async fn insert_voice_job(state: &AppState, job: NewVoiceJob<'_>) -> Result<String, String> {
    let mut conn = state.db.acquire().await.map_err(|e| e.to_string())?;
    insert_voice_job_on(&mut conn, job).await
}

async fn insert_voice_job_on(
    conn: &mut sqlx::SqliteConnection,
    job: NewVoiceJob<'_>,
) -> Result<String, String> {
    validate_label(job.target_type, ALLOWED_VOICE_TARGET_TYPES, "target_type")?;
    let text = trim_required(job.text, "voice text")?;
    validate_text_length(&text, "voice text")?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        r#"
        INSERT INTO voice_jobs (
            id, target_type, target_id, product_line, voice_kind, text, file_prefix,
            status, priority, run_after, attempt_count, max_attempts, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, 0, 5, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(job.target_type)
    .bind(job.target_id)
    .bind(job.product_line)
    .bind(job.voice_kind)
    .bind(text)
    .bind(job.file_prefix)
    .bind(job.priority)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub async fn lease_voice_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LeaseVoiceJobRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    let worker_id = match trim_required(&payload.worker_id, "worker_id") {
        Ok(value) => value,
        Err(e) => return bad_request(e),
    };
    let now = chrono::Utc::now().timestamp();
    let lease_expires_at = now + clamp_lease_seconds(payload.lease_seconds);
    // Select and claim in one SQLite write statement: concurrent workers must
    // not mistake a lost compare-and-swap for an empty queue.
    let kinds = payload
        .voice_kinds
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap());
    let result = sqlx::query_as::<_, VoiceJob>(
        r#"
        UPDATE voice_jobs
        SET status = 'leased', lease_owner = ?, lease_token = ?,
            lease_expires_at = ?, attempt_count = COALESCE(attempt_count, 0) + 1,
            updated_at = ?
        WHERE id = (
            SELECT id FROM voice_jobs
            WHERE (status = 'queued' OR
                (status = 'leased' AND COALESCE(lease_expires_at, 0) < ?))
              AND COALESCE(run_after, 0) <= ?
              AND COALESCE(attempt_count, 0) < COALESCE(max_attempts, 5)
              AND (? IS NULL OR voice_kind IN (SELECT value FROM json_each(?)))
            ORDER BY priority DESC, created_at ASC, id ASC
            LIMIT 1
        )
        RETURNING *
        "#,
    )
    .bind(&worker_id)
    .bind(Uuid::new_v4().to_string())
    .bind(lease_expires_at)
    .bind(now)
    .bind(now)
    .bind(now)
    .bind(&kinds)
    .bind(&kinds)
    .fetch_optional(&state.db)
    .await;
    match result {
        Ok(job) => Json(json!({ "job": job })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn list_voice_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<VoiceListQuery>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    let status = query.status.unwrap_or_else(|| "queued".to_string());
    if let Err(e) = validate_label(&status, ALLOWED_VOICE_STATUSES, "status") {
        return bad_request(e);
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query_as::<_, VoiceJob>(
        "SELECT * FROM voice_jobs WHERE status = ? ORDER BY created_at ASC LIMIT ?",
    )
    .bind(status)
    .bind(limit)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn get_voice_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }

    let job = sqlx::query_as::<_, VoiceJob>("SELECT * FROM voice_jobs WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await;

    match job {
        Ok(Some(job)) => Json(job).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn repair_missing_voice_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RepairVoiceJobsRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }

    match repair_missing_voice_jobs_inner(&state, payload.limit).await {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn heartbeat_voice_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    let now = chrono::Utc::now().timestamp();
    let lease_expires_at = now + clamp_lease_seconds(payload.lease_seconds);
    let result = sqlx::query(
        r#"
        UPDATE voice_jobs
        SET lease_expires_at = ?, updated_at = ?
        WHERE id = ? AND lease_token = ? AND status = 'leased' AND lease_expires_at >= ?
        "#,
    )
    .bind(lease_expires_at)
    .bind(now)
    .bind(&id)
    .bind(&payload.lease_token)
    .bind(now)
    .execute(&state.db)
    .await;

    match result {
        Ok(result) if result.rows_affected() > 0 => {
            Json(json!({ "id": id, "lease_expires_at": lease_expires_at })).into_response()
        }
        Ok(_) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn complete_voice_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<CompleteVoiceJobRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    if let Err(e) = validate_audio_url(&payload.audio_url) {
        return bad_request(e);
    }
    if payload.duration_sec.is_some_and(|duration| duration < 0) {
        return bad_request("duration_sec must be non-negative");
    }

    let job = match load_leased_voice_job(&state, &id, &payload.lease_token).await {
        Ok(job) => job,
        Err(response) => return response,
    };
    let now = chrono::Utc::now().timestamp();
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if let Err(e) =
        apply_voice_completion(&mut tx, &job, &payload.audio_url, payload.duration_sec).await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }

    let result = sqlx::query(
        r#"
        UPDATE voice_jobs
        SET status = 'completed',
            audio_url = ?,
            duration_sec = ?,
            lease_owner = NULL,
            lease_token = NULL,
            lease_expires_at = NULL,
            updated_at = ?,
            completed_at = ?
        WHERE id = ?
        "#,
    )
    .bind(&payload.audio_url)
    .bind(payload.duration_sec)
    .bind(now)
    .bind(now)
    .bind(&id)
    .execute(&mut *tx)
    .await;

    match result {
        Ok(result) if result.rows_affected() > 0 => match tx.commit().await {
            Ok(_) => Json(json!({ "id": id, "status": "completed" })).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Ok(_) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn fail_voice_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<FailJobRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return unauthorized();
    }
    let err = truncate_error(&payload.error);
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        r#"
        UPDATE voice_jobs
        SET status = CASE
                WHEN COALESCE(attempt_count, 0) >= COALESCE(max_attempts, 5) THEN 'failed'
                ELSE 'queued'
            END,
            lease_owner = NULL,
            lease_token = NULL,
            lease_expires_at = NULL,
            last_error = ?,
            run_after = ? + MIN(3600, 30 * (1 << MIN(7, MAX(0, COALESCE(attempt_count, 1) - 1)))),
            updated_at = ?
        WHERE id = ? AND lease_token = ? AND status = 'leased' AND lease_expires_at >= ?
        "#,
    )
    .bind(&err)
    .bind(now)
    .bind(now)
    .bind(&id)
    .bind(&payload.lease_token)
    .bind(now)
    .execute(&state.db)
    .await;

    match result {
        Ok(result) if result.rows_affected() > 0 => StatusCode::OK.into_response(),
        Ok(_) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Debug, FromRow)]
struct MissingVoiceTarget {
    target_type: String,
    target_id: String,
    product_line: String,
    voice_kind: String,
    text: String,
    file_prefix: String,
    priority: i64,
    _sort_time: Option<i64>,
    created_at: Option<i64>,
}

async fn repair_missing_voice_jobs_inner(
    state: &AppState,
    limit: Option<i64>,
) -> Result<RepairVoiceJobsResponse, String> {
    let limit = limit
        .unwrap_or(DEFAULT_VOICE_REPAIR_LIMIT)
        .clamp(1, MAX_VOICE_REPAIR_LIMIT);
    let mut stats = RepairVoiceJobsResponse::default();

    let mut targets = sqlx::query_as::<_, MissingVoiceTarget>(
        r#"
        SELECT
            'feed_item' AS target_type,
            fi.id AS target_id,
            fi.product_line AS product_line,
            'curated_article' AS voice_kind,
            fic.audio_script AS text,
            'curated_article' AS file_prefix,
            0 AS priority,
            fi.publish_time AS _sort_time,
            fi.created_at AS created_at
        FROM feed_items fi
        JOIN feed_item_contents fic ON fic.item_id = fi.id
        WHERE fi.product_line = 'curated_feed'
          AND fi.item_type = 'article'
          AND fi.status = 'published'
          AND TRIM(COALESCE(fic.audio_script, '')) != ''
          AND (
              COALESCE(fi.has_audio, 0) = 0
              OR fi.audio_url IS NULL
              OR TRIM(fi.audio_url) = ''
          )

        UNION ALL

        SELECT
            'weekly_digest' AS target_type,
            wd.id AS target_id,
            'curated_feed' AS product_line,
            'curated_weekly' AS voice_kind,
            wd.audio_script AS text,
            'curated_weekly' AS file_prefix,
            0 AS priority,
            wd.week_end AS _sort_time,
            wd.created_at AS created_at
        FROM weekly_digests wd
        WHERE wd.status = 'published'
          AND TRIM(COALESCE(wd.audio_script, '')) != ''
          AND (
              wd.audio_url IS NULL
              OR TRIM(wd.audio_url) = ''
          )
        ORDER BY _sort_time DESC, created_at DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    for target in targets.drain(..) {
        stats.scanned += 1;
        let text = target.text.trim();
        if text.is_empty() {
            stats.skipped_missing_text += 1;
            continue;
        }

        if let Some((audio_url, duration_sec)) =
            completed_voice_audio_for_target(state, &target.target_type, &target.target_id).await?
        {
            let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
            let job = VoiceJob {
                id: "repair-backfill".to_string(),
                target_type: target.target_type.clone(),
                target_id: target.target_id.clone(),
                product_line: target.product_line.clone(),
                voice_kind: target.voice_kind.clone(),
                text: target.text.clone(),
                file_prefix: target.file_prefix.clone(),
                status: "completed".to_string(),
                priority: Some(target.priority),
                run_after: None,
                lease_owner: None,
                lease_token: None,
                lease_expires_at: None,
                attempt_count: None,
                max_attempts: None,
                audio_url: Some(audio_url.clone()),
                duration_sec,
                last_error: None,
                created_at: target.created_at,
                updated_at: None,
                completed_at: None,
            };
            apply_voice_completion(&mut tx, &job, &audio_url, duration_sec).await?;
            tx.commit().await.map_err(|e| e.to_string())?;
            stats.backfilled_completed += 1;
            continue;
        }

        if active_voice_job_exists(state, &target.target_type, &target.target_id).await? {
            stats.skipped_active += 1;
            continue;
        }

        if recent_failed_voice_jobs(state, &target.target_type, &target.target_id).await?
            >= MAX_RECENT_FAILED_REPAIR_JOBS
        {
            stats.skipped_recent_failures += 1;
            continue;
        }

        let job_id = insert_voice_job(
            state,
            NewVoiceJob {
                target_type: &target.target_type,
                target_id: &target.target_id,
                product_line: &target.product_line,
                voice_kind: &target.voice_kind,
                text,
                file_prefix: &target.file_prefix,
                priority: target.priority,
            },
        )
        .await?;
        stats.created += 1;
        stats.voice_job_ids.push(job_id);
    }

    Ok(stats)
}

async fn active_voice_job_exists(
    state: &AppState,
    target_type: &str,
    target_id: &str,
) -> Result<bool, String> {
    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM voice_jobs
        WHERE target_type = ?
          AND target_id = ?
          AND status IN ('queued', 'leased')
        "#,
    )
    .bind(target_type)
    .bind(target_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

async fn recent_failed_voice_jobs(
    state: &AppState,
    target_type: &str,
    target_id: &str,
) -> Result<i64, String> {
    let cutoff = chrono::Utc::now().timestamp() - 24 * 60 * 60;
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM voice_jobs
        WHERE target_type = ?
          AND target_id = ?
          AND status IN ('failed', 'cancelled')
          AND COALESCE(updated_at, created_at, 0) >= ?
        "#,
    )
    .bind(target_type)
    .bind(target_id)
    .bind(cutoff)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())
}

async fn completed_voice_audio_for_target(
    state: &AppState,
    target_type: &str,
    target_id: &str,
) -> Result<Option<(String, Option<i64>)>, String> {
    sqlx::query_as::<_, VoiceJob>(
        r#"
        SELECT *
        FROM voice_jobs
        WHERE target_type = ?
          AND target_id = ?
          AND status = 'completed'
          AND audio_url IS NOT NULL
          AND TRIM(audio_url) != ''
        ORDER BY completed_at DESC, updated_at DESC
        LIMIT 1
        "#,
    )
    .bind(target_type)
    .bind(target_id)
    .fetch_optional(&state.db)
    .await
    .map(|job| {
        job.and_then(|job| {
            job.audio_url
                .filter(|url| !url.trim().is_empty())
                .map(|url| (url, job.duration_sec))
        })
    })
    .map_err(|e| e.to_string())
}

async fn load_leased_voice_job(
    state: &AppState,
    id: &str,
    lease_token: &str,
) -> Result<VoiceJob, axum::response::Response> {
    let now = chrono::Utc::now().timestamp();
    let job = sqlx::query_as::<_, VoiceJob>(
        "SELECT * FROM voice_jobs WHERE id = ? AND lease_token = ? AND status = 'leased' AND lease_expires_at >= ?",
    )
    .bind(id)
    .bind(lease_token)
    .bind(now)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())?;

    let Some(job) = job else {
        return Err(StatusCode::CONFLICT.into_response());
    };
    if job.lease_expires_at.unwrap_or_default() < now {
        return Err(StatusCode::CONFLICT.into_response());
    }
    Ok(job)
}

async fn apply_voice_completion(
    tx: &mut Transaction<'_, Sqlite>,
    job: &VoiceJob,
    audio_url: &str,
    duration_sec: Option<i64>,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    match job.target_type.as_str() {
        "radio_item" => {
            let result = sqlx::query(
                "UPDATE items SET audio_url = ?, duration_sec = ?, status = 'published' WHERE id = ?",
            )
            .bind(audio_url)
            .bind(duration_sec)
            .bind(&job.target_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
            if result.rows_affected() == 0 {
                return Err(format!("radio_item {} not found", job.target_id));
            }
        }
        "feed_item" => {
            let result = sqlx::query(
                r#"
                UPDATE feed_items
                SET has_audio = 1, audio_url = ?, duration_sec = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(audio_url)
            .bind(duration_sec)
            .bind(now)
            .bind(&job.target_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
            if result.rows_affected() == 0 {
                return Err(format!("feed_item {} not found", job.target_id));
            }
        }
        "weekly_digest" => {
            let feed_item_id = sqlx::query_scalar::<_, Option<String>>(
                "SELECT feed_item_id FROM weekly_digests WHERE id = ?",
            )
            .bind(&job.target_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| e.to_string())?
            .flatten();

            let result = sqlx::query(
                "UPDATE weekly_digests SET audio_url = ?, duration_sec = ? WHERE id = ?",
            )
            .bind(audio_url)
            .bind(duration_sec)
            .bind(&job.target_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
            if result.rows_affected() == 0 {
                return Err(format!("weekly_digest {} not found", job.target_id));
            }

            if let Some(feed_item_id) = feed_item_id {
                sqlx::query(
                    r#"
                    UPDATE feed_items
                    SET has_audio = 1, audio_url = ?, duration_sec = ?, updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(audio_url)
                .bind(duration_sec)
                .bind(now)
                .bind(feed_item_id)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
        other => return Err(format!("unsupported voice target_type {other}")),
    }
    Ok(())
}

fn parse_json(raw: Option<&str>) -> Value {
    raw.and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    #[test]
    fn lease_seconds_are_clamped() {
        assert_eq!(clamp_lease_seconds(Some(1)), 60);
        assert_eq!(
            clamp_lease_seconds(Some(MAX_LEASE_SECONDS + 1)),
            MAX_LEASE_SECONDS
        );
        assert_eq!(clamp_lease_seconds(None), DEFAULT_LEASE_SECONDS);
    }

    #[test]
    fn job_type_validation_rejects_unknown_tools() {
        assert!(validate_agent_job_type("radio_episode").is_ok());
        assert!(validate_agent_job_type("arbitrary_shell_task").is_err());
    }

    #[test]
    fn audio_url_accepts_only_served_or_remote_audio() {
        assert!(validate_audio_url("/audio/a.mp3").is_ok());
        assert!(validate_audio_url("https://example.com/a.mp3").is_ok());
        assert!(validate_audio_url("file:///tmp/a.mp3").is_err());
        assert!(validate_audio_url("/api/items").is_err());
    }

    #[test]
    fn radio_episode_artifact_requires_script() {
        let artifact = json!({
            "title": "AI News",
            "category": "AI",
            "script": "今天的内容。"
        });
        let parsed: RadioEpisodeArtifact = serde_json::from_value(artifact).unwrap();
        assert_eq!(parsed.category, "AI");
        assert!(trim_required(&parsed.script, "script").is_ok());
    }

    async fn test_state() -> AppState {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        sqlx::query(
            r#"
            CREATE TABLE items (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                summary TEXT,
                original_url TEXT UNIQUE,
                cover_image_url TEXT,
                audio_url TEXT,
                publish_time INTEGER,
                created_at INTEGER,
                rating INTEGER,
                tags TEXT,
                is_deleted BOOLEAN DEFAULT 0,
                duration_sec INTEGER,
                status TEXT DEFAULT 'published',
                category TEXT
            );
            CREATE TABLE item_sources (
                id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL,
                source_url TEXT NOT NULL,
                source_title TEXT,
                source_summary TEXT,
                created_at INTEGER
            );
            CREATE TABLE feed_items (
                id TEXT PRIMARY KEY,
                product_line TEXT NOT NULL,
                item_type TEXT NOT NULL,
                primary_mode TEXT NOT NULL,
                title TEXT NOT NULL,
                subtitle TEXT,
                source_name TEXT,
                source_url TEXT,
                original_url TEXT,
                canonical_url TEXT,
                content_hash TEXT,
                publish_time INTEGER,
                created_at INTEGER,
                updated_at INTEGER,
                has_audio BOOLEAN DEFAULT 0,
                audio_url TEXT,
                duration_sec INTEGER,
                reading_time_min INTEGER,
                quality_score INTEGER,
                tags TEXT,
                status TEXT DEFAULT 'published'
            );
            CREATE TABLE feed_item_contents (
                item_id TEXT PRIMARY KEY,
                original_html TEXT,
                reader_markdown TEXT,
                plain_text TEXT,
                compressed_markdown TEXT,
                audio_script TEXT,
                key_points_json TEXT,
                created_at INTEGER,
                updated_at INTEGER
            );
            CREATE TABLE weekly_digests (
                id TEXT PRIMARY KEY,
                feed_item_id TEXT,
                week_start INTEGER NOT NULL,
                week_end INTEGER NOT NULL,
                title TEXT NOT NULL,
                digest_markdown TEXT,
                audio_script TEXT,
                audio_url TEXT,
                duration_sec INTEGER,
                included_item_ids_json TEXT,
                themes_json TEXT,
                created_at INTEGER,
                status TEXT DEFAULT 'published'
            );
            CREATE TABLE loop_posts (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                post_type TEXT NOT NULL,
                feedback_mode TEXT DEFAULT 'balance',
                title TEXT,
                body TEXT NOT NULL,
                visibility TEXT NOT NULL DEFAULT 'private',
                source_ref TEXT,
                memory_entry_id TEXT,
                preference_status TEXT DEFAULT 'pending',
                preference_extracted_at INTEGER,
                preference_error TEXT,
                created_at INTEGER,
                updated_at INTEGER,
                status TEXT DEFAULT 'published'
            );
            CREATE TABLE agent_jobs (
                id TEXT PRIMARY KEY,
                job_type TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                priority INTEGER DEFAULT 0,
                run_after INTEGER,
                lease_owner TEXT,
                lease_token TEXT,
                lease_expires_at INTEGER,
                attempt_count INTEGER DEFAULT 0,
                max_attempts INTEGER DEFAULT 3,
                context_json TEXT,
                input_json TEXT,
                artifact_json TEXT,
                result_ref TEXT,
                last_error TEXT,
                created_at INTEGER,
                updated_at INTEGER,
                completed_at INTEGER
            );
            CREATE TABLE agent_job_events (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                actor TEXT,
                message TEXT,
                payload_json TEXT,
                created_at INTEGER
            );
            CREATE TABLE voice_jobs (
                id TEXT PRIMARY KEY,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                product_line TEXT NOT NULL,
                voice_kind TEXT NOT NULL,
                text TEXT NOT NULL,
                file_prefix TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                priority INTEGER DEFAULT 0,
                run_after INTEGER,
                lease_owner TEXT,
                lease_token TEXT,
                lease_expires_at INTEGER,
                attempt_count INTEGER DEFAULT 0,
                max_attempts INTEGER DEFAULT 5,
                audio_url TEXT,
                duration_sec INTEGER,
                last_error TEXT,
                created_at INTEGER,
                updated_at INTEGER,
                completed_at INTEGER
            );
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        let memory_path =
            std::env::temp_dir().join(format!("freshloop-agent-test-{}.redb", Uuid::new_v4()));
        let memory_store =
            loop_memory::RedbMemoryStore::new(memory_path.to_string_lossy().to_string()).unwrap();

        AppState {
            db: pool,
            api_key: "test-key".to_string(),
            audio_dir: "audio".to_string(),
            memory_store: Arc::new(memory_store),
        }
    }

    fn authed_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("X-NEXUS-KEY", "test-key".parse().unwrap());
        headers
    }

    #[tokio::test]
    async fn radio_spoken_copy_does_not_replace_display_manuscript() {
        let state = test_state().await;
        let mut tx = state.db.begin().await.unwrap();
        let artifact = json!({"title":"芯片", "category":"Tech", "script":"GPU 主频为 3.5GHz。", "audio_script":"图形处理器的主频为三点五吉赫兹。"});
        let result = publish_radio_episode(&mut tx, &artifact, 10).await.unwrap();
        let id = result
            .result_ref
            .unwrap()
            .strip_prefix("radio_item:")
            .unwrap()
            .to_string();
        let displayed: String = sqlx::query_scalar("SELECT summary FROM items WHERE id=?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        let spoken: String = sqlx::query_scalar("SELECT text FROM voice_jobs WHERE target_id=?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(displayed, artifact["script"].as_str().unwrap());
        assert_eq!(spoken, artifact["audio_script"].as_str().unwrap());
    }

    #[tokio::test]
    async fn capabilities_rejects_missing_internal_auth() {
        let state = test_state().await;
        let response = capabilities(State(state), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_voice_job_returns_single_job_with_internal_auth() {
        let state = test_state().await;
        sqlx::query(
            r#"
            INSERT INTO voice_jobs (
                id, target_type, target_id, product_line, voice_kind, text, file_prefix,
                status, priority, run_after, attempt_count, max_attempts, created_at, updated_at
            )
            VALUES (
                'voice-one', 'feed_item', 'feed-one', 'curated_feed', 'curated_article',
                'hello', 'curated_article', 'queued', 0, 1, 0, 5, 1, 1
            )
            "#,
        )
        .execute(&state.db)
        .await
        .unwrap();

        let response = get_voice_job(
            State(state.clone()),
            authed_headers(),
            Path("voice-one".to_string()),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let response = get_voice_job(
            State(state),
            HeaderMap::new(),
            Path("voice-one".to_string()),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn submitting_reading_article_publishes_content_and_voice_job() {
        let state = test_state().await;
        let now = chrono::Utc::now().timestamp();
        let job_id = "agent-job-reading";
        let lease_token = "lease-token";
        sqlx::query(
            r#"
            INSERT INTO agent_jobs (
                id, job_type, status, lease_owner, lease_token, lease_expires_at,
                attempt_count, max_attempts, created_at, updated_at
            )
            VALUES (?, 'reading_article', 'leased', 'codex-test', ?, ?, 1, 3, ?, ?)
            "#,
        )
        .bind(job_id)
        .bind(lease_token)
        .bind(now + 300)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();

        let response = submit_agent_job(
            State(state.clone()),
            authed_headers(),
            Path(job_id.to_string()),
            Json(SubmitAgentJobRequest {
                lease_token: lease_token.to_string(),
                artifact: json!({
                    "title": "A careful article",
                    "original_url": "https://example.com/article",
                    "source_name": "Example",
                    "reader_markdown": "# A careful article\n\n正文",
                    "compressed_markdown": "干货摘要",
                    "plain_text": "正文",
                    "audio_script": "适合语音播放的版本",
                    "key_points": ["观点一", "观点二"],
                    "quality_score": 8
                }),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);

        let job_status: String = sqlx::query_scalar("SELECT status FROM agent_jobs WHERE id = ?")
            .bind(job_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(job_status, "completed");

        let feed_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feed_items")
            .fetch_one(&state.db)
            .await
            .unwrap();
        let content_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feed_item_contents")
            .fetch_one(&state.db)
            .await
            .unwrap();
        let voice_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_jobs")
            .fetch_one(&state.db)
            .await
            .unwrap();
        let voice_priority: i64 = sqlx::query_scalar("SELECT priority FROM voice_jobs LIMIT 1")
            .fetch_one(&state.db)
            .await
            .unwrap();

        assert_eq!(feed_count, 1);
        assert_eq!(content_count, 1);
        assert_eq!(voice_count, 1);
        assert_eq!(voice_priority, DEFAULT_CONTENT_VOICE_PRIORITY);
    }

    #[tokio::test]
    async fn completing_voice_job_updates_reading_audio_fields() {
        let state = test_state().await;
        let mut conn = state.db.acquire().await.unwrap();
        let publish_result = publish_reading_article(
            &mut conn,
            &json!({
                "title": "Audio article",
                "original_url": "https://example.com/audio-article",
                "audio_script": "这是一段需要合成的语音文稿。"
            }),
            DEFAULT_CONTENT_VOICE_PRIORITY,
        )
        .await
        .unwrap();
        drop(conn);
        assert_eq!(publish_result.voice_job_ids.len(), 1);

        let voice_job_id = publish_result.voice_job_ids[0].clone();
        sqlx::query(
            r#"
            UPDATE voice_jobs
            SET status = 'leased',
                lease_owner = 'cortex-test',
                lease_token = 'voice-token',
                lease_expires_at = ?
            WHERE id = ?
            "#,
        )
        .bind(chrono::Utc::now().timestamp() + 300)
        .bind(&voice_job_id)
        .execute(&state.db)
        .await
        .unwrap();

        let response = complete_voice_job(
            State(state.clone()),
            authed_headers(),
            Path(voice_job_id.clone()),
            Json(CompleteVoiceJobRequest {
                lease_token: "voice-token".to_string(),
                audio_url: "/audio/generated.mp3".to_string(),
                duration_sec: Some(42),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);

        let audio_url: Option<String> =
            sqlx::query_scalar("SELECT audio_url FROM feed_items LIMIT 1")
                .fetch_one(&state.db)
                .await
                .unwrap();
        let has_audio: bool = sqlx::query_scalar("SELECT has_audio FROM feed_items LIMIT 1")
            .fetch_one(&state.db)
            .await
            .unwrap();
        let voice_status: String = sqlx::query_scalar("SELECT status FROM voice_jobs WHERE id = ?")
            .bind(&voice_job_id)
            .fetch_one(&state.db)
            .await
            .unwrap();

        assert_eq!(audio_url.as_deref(), Some("/audio/generated.mp3"));
        assert!(has_audio);
        assert_eq!(voice_status, "completed");
    }

    #[tokio::test]
    async fn repair_missing_voice_jobs_creates_article_audio_job() {
        let state = test_state().await;
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            r#"
            INSERT INTO feed_items (
                id, product_line, item_type, primary_mode, title, publish_time,
                created_at, updated_at, has_audio, audio_url, status
            )
            VALUES (
                'feed-missing-audio', 'curated_feed', 'article', 'read',
                'Needs audio', ?, ?, ?, 0, NULL, 'published'
            )
            "#,
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO feed_item_contents (
                item_id, audio_script, created_at, updated_at
            )
            VALUES ('feed-missing-audio', '这是一段需要补生成的精选文章语音稿。', ?, ?)
            "#,
        )
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();

        let stats = repair_missing_voice_jobs_inner(&state, Some(10))
            .await
            .unwrap();

        assert_eq!(stats.scanned, 1);
        assert_eq!(stats.created, 1);
        assert_eq!(stats.voice_job_ids.len(), 1);

        let voice_kind: String =
            sqlx::query_scalar("SELECT voice_kind FROM voice_jobs WHERE target_id = ?")
                .bind("feed-missing-audio")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(voice_kind, "curated_article");
    }

    #[tokio::test]
    async fn repair_missing_voice_jobs_backfills_completed_article_audio() {
        let state = test_state().await;
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            r#"
            INSERT INTO feed_items (
                id, product_line, item_type, primary_mode, title, publish_time,
                created_at, updated_at, has_audio, audio_url, status
            )
            VALUES (
                'feed-backfill-audio', 'curated_feed', 'article', 'read',
                'Backfill audio', ?, ?, ?, 0, NULL, 'published'
            )
            "#,
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO feed_item_contents (
                item_id, audio_script, created_at, updated_at
            )
            VALUES ('feed-backfill-audio', '这是一段已经完成但字段未回填的语音稿。', ?, ?)
            "#,
        )
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO voice_jobs (
                id, target_type, target_id, product_line, voice_kind, text, file_prefix,
                status, priority, run_after, attempt_count, max_attempts, audio_url,
                duration_sec, created_at, updated_at, completed_at
            )
            VALUES (
                'completed-audio-job', 'feed_item', 'feed-backfill-audio',
                'curated_feed', 'curated_article', 'text', 'curated_article',
                'completed', 0, ?, 1, 5, '/audio/existing.mp3', 33, ?, ?, ?
            )
            "#,
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();

        let stats = repair_missing_voice_jobs_inner(&state, Some(10))
            .await
            .unwrap();

        assert_eq!(stats.scanned, 1);
        assert_eq!(stats.backfilled_completed, 1);
        assert_eq!(stats.created, 0);

        let audio_url: Option<String> =
            sqlx::query_scalar("SELECT audio_url FROM feed_items WHERE id = ?")
                .bind("feed-backfill-audio")
                .fetch_one(&state.db)
                .await
                .unwrap();
        let has_audio: bool = sqlx::query_scalar("SELECT has_audio FROM feed_items WHERE id = ?")
            .bind("feed-backfill-audio")
            .fetch_one(&state.db)
            .await
            .unwrap();

        assert_eq!(audio_url.as_deref(), Some("/audio/existing.mp3"));
        assert!(has_audio);
    }

    #[tokio::test]
    async fn repair_missing_voice_jobs_creates_weekly_audio_job() {
        let state = test_state().await;
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            r#"
            INSERT INTO feed_items (
                id, product_line, item_type, primary_mode, title, publish_time,
                created_at, updated_at, has_audio, audio_url, status
            )
            VALUES (
                'weekly-feed-item', 'curated_feed', 'weekly_digest', 'listen',
                'Weekly needs audio', ?, ?, ?, 0, NULL, 'published'
            )
            "#,
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO weekly_digests (
                id, feed_item_id, week_start, week_end, title, digest_markdown,
                audio_script, audio_url, duration_sec, created_at, status
            )
            VALUES (
                'weekly-missing-audio', 'weekly-feed-item', ?, ?, 'Weekly needs audio',
                '周报正文', '这是一段需要补生成的周报语音稿。', NULL, NULL, ?, 'published'
            )
            "#,
        )
        .bind(now - 6 * 24 * 60 * 60)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();

        let stats = repair_missing_voice_jobs_inner(&state, Some(10))
            .await
            .unwrap();

        assert_eq!(stats.scanned, 1);
        assert_eq!(stats.created, 1);
        assert_eq!(stats.voice_job_ids.len(), 1);

        let (target_type, voice_kind): (String, String) =
            sqlx::query_as("SELECT target_type, voice_kind FROM voice_jobs WHERE target_id = ?")
                .bind("weekly-missing-audio")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(target_type, "weekly_digest");
        assert_eq!(voice_kind, "curated_weekly");
    }
    async fn seed_agent(state: &AppState, id: &str, kind: &str, expiry: i64) {
        sqlx::query("INSERT INTO agent_jobs (id,job_type,status,lease_token,lease_expires_at,attempt_count,max_attempts) VALUES (?,?,'leased','token',?,1,3)")
            .bind(id).bind(kind).bind(expiry).execute(&state.db).await.unwrap();
    }

    #[tokio::test]
    async fn invalid_source_rolls_back_radio_and_allows_correction_then_replay() {
        let state = test_state().await;
        seed_agent(
            &state,
            "atomic-radio",
            "radio_episode",
            chrono::Utc::now().timestamp() + 300,
        )
        .await;
        let mut artifact = json!({"title":"Report","category":"Tech","script":"Verified report", "sources":[{"url":"not-a-url"}]});
        let response = submit_agent_job(
            State(state.clone()),
            authed_headers(),
            Path("atomic-radio".into()),
            Json(SubmitAgentJobRequest {
                lease_token: "token".into(),
                artifact: artifact.clone(),
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        let status: String =
            sqlx::query_scalar("SELECT status FROM agent_jobs WHERE id='atomic-radio'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(status, "leased");
        artifact["sources"][0]["url"] = json!("https://example.org/report");
        for _ in 0..2 {
            let response = submit_agent_job(
                State(state.clone()),
                authed_headers(),
                Path("atomic-radio".into()),
                Json(SubmitAgentJobRequest {
                    lease_token: "token".into(),
                    artifact: artifact.clone(),
                }),
            )
            .await
            .into_response();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items")
            .fetch_one(&state.db)
            .await
            .unwrap();
        let voice: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_jobs")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!((count, voice), (1, 1));
        artifact["script"] = json!("Changed after completion");
        let response = submit_agent_job(
            State(state),
            authed_headers(),
            Path("atomic-radio".into()),
            Json(SubmitAgentJobRequest {
                lease_token: "token".into(),
                artifact,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn weekly_repair_preserves_identity_and_fences_old_audio() {
        let state = test_state().await;
        sqlx::query(
            "INSERT INTO feed_items (id,title,item_type,product_line,primary_mode) VALUES ('feed','old','weekly_digest','curated_feed','listen')",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO weekly_digests (id,feed_item_id,week_start,week_end,title,audio_script,status) VALUES ('digest','feed',10,20,'old','unreviewed','published')")
            .execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO voice_jobs (id,target_type,target_id,product_line,voice_kind,text,file_prefix,status,lease_token) VALUES ('old-voice','weekly_digest','digest','curated_feed','curated_weekly','unreviewed','weekly','leased','old-token')")
            .execute(&state.db).await.unwrap();
        let mut tx = state.db.begin().await.unwrap();
        let mut artifact = json!({"replace_digest_id":"digest","expected_audio_script":"wrong", "title":"reviewed", "week_start":10,"week_end":20,"audio_script":"reviewed script","digest_markdown":"reviewed digest"});
        assert!(publish_weekly_digest(&mut tx, &artifact, 10).await.is_err());
        artifact["expected_audio_script"] = json!("unreviewed");
        let result = publish_weekly_digest(&mut tx, &artifact, 10).await.unwrap();
        assert_eq!(result.result_ref.as_deref(), Some("weekly_digest:digest"));
        assert_eq!(result.voice_job_ids.len(), 1);
        tx.commit().await.unwrap();
        let row: (String, Option<String>) =
            sqlx::query_as("SELECT status,lease_token FROM voice_jobs WHERE id='old-voice'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(row, ("cancelled".into(), None));
        let row: (String, String) = sqlx::query_as(
            "SELECT feed_item_id,audio_script FROM weekly_digests WHERE id='digest'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(row, ("feed".into(), "reviewed script".into()));
        // Replaying a stale repair cannot erase a newer manuscript.
        let mut tx = state.db.begin().await.unwrap();
        assert!(publish_weekly_digest(&mut tx, &artifact, 10).await.is_err());
    }

    #[tokio::test]
    async fn voice_failure_backs_off_before_retry() {
        let state = test_state().await;
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO voice_jobs (id,target_type,target_id,product_line,voice_kind,text,file_prefix,status,lease_token,lease_expires_at,attempt_count) VALUES ('retry','feed_item','target','curated_feed','curated_article','text','article','leased','token',?,2)")
            .bind(now+300).execute(&state.db).await.unwrap();
        let response = fail_voice_job(
            State(state.clone()),
            authed_headers(),
            Path("retry".into()),
            Json(FailJobRequest {
                lease_token: "token".into(),
                error: "quality failure".into(),
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let row: (String, i64) =
            sqlx::query_as("SELECT status,run_after FROM voice_jobs WHERE id='retry'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(row.0, "queued");
        assert!(row.1 >= now + 60 && row.1 <= now + 65);
        let response = lease_voice_job(
            State(state),
            authed_headers(),
            Json(LeaseVoiceJobRequest {
                worker_id: "worker".into(),
                voice_kinds: None,
                lease_seconds: None,
            }),
        )
        .await
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(body["job"].is_null());
    }

    #[tokio::test]
    async fn concurrent_voice_leases_drain_distinct_jobs_and_respect_filters() {
        let state = test_state().await;
        for i in 0..5 {
            sqlx::query("INSERT INTO voice_jobs (id,target_type,target_id,product_line,voice_kind,text,file_prefix,priority) VALUES (?,'item','target','radio',?,'text','test',?)")
                .bind(format!("voice-{i}"))
                .bind(if i == 4 { "other" } else { "radio" })
                .bind(if i == 4 { 100 } else { 0 })
                .execute(&state.db).await.unwrap();
        }
        let mut workers = tokio::task::JoinSet::new();
        for i in 0..4 {
            let state = state.clone();
            workers.spawn(async move {
                let response = lease_voice_job(
                    State(state),
                    authed_headers(),
                    Json(LeaseVoiceJobRequest {
                        worker_id: format!("worker-{i}"),
                        voice_kinds: Some(vec!["radio".into()]),
                        lease_seconds: Some(300),
                    }),
                )
                .await
                .into_response();
                assert_eq!(response.status(), StatusCode::OK);
                let body = axum::body::to_bytes(response.into_body(), 65536)
                    .await
                    .unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(body["job"]["attempt_count"], 1);
                body["job"]["id"].as_str().unwrap().to_owned()
            });
        }
        let mut ids = std::collections::HashSet::new();
        while let Some(result) = workers.join_next().await {
            assert!(ids.insert(result.unwrap()));
        }
        assert_eq!(ids.len(), 4);
        let response = lease_voice_job(
            State(state),
            authed_headers(),
            Json(LeaseVoiceJobRequest {
                worker_id: "empty".into(),
                voice_kinds: Some(vec!["radio".into()]),
                lease_seconds: None,
            }),
        )
        .await
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(body["job"].is_null());
    }

    #[tokio::test]
    async fn expired_agent_cannot_heartbeat_or_publish() {
        let state = test_state().await;
        seed_agent(&state, "expired", "reading_article", 1).await;
        let response = heartbeat_agent_job(
            State(state.clone()),
            authed_headers(),
            Path("expired".into()),
            Json(HeartbeatRequest {
                lease_token: "token".into(),
                lease_seconds: Some(300),
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let response = submit_agent_job(
            State(state),
            authed_headers(),
            Path("expired".into()),
            Json(SubmitAgentJobRequest {
                lease_token: "token".into(),
                artifact: json!({}),
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn exact_lease_is_not_starved_by_other_types_and_create_is_idempotent() {
        let state = test_state().await;
        for i in 0..25 {
            sqlx::query("INSERT INTO agent_jobs (id,job_type,status,priority) VALUES (?,'radio_episode','queued',100)").bind(format!("other-{i}")).execute(&state.db).await.unwrap();
        }
        for _ in 0..2 {
            let response = create_agent_job(
                State(state.clone()),
                authed_headers(),
                Json(CreateAgentJobRequest {
                    id: Some("wanted".into()),
                    job_type: "reading_article".into(),
                    priority: None,
                    run_after: None,
                    max_attempts: None,
                    context: None,
                    input: None,
                }),
            )
            .await
            .into_response();
            assert!(response.status().is_success());
        }
        let response = lease_agent_job(
            State(state.clone()),
            authed_headers(),
            Json(LeaseAgentJobRequest {
                job_id: Some("wanted".into()),
                agent_id: "worker".into(),
                job_types: Some(vec!["reading_article".into()]),
                lease_seconds: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let status: String = sqlx::query_scalar("SELECT status FROM agent_jobs WHERE id='wanted'")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(status, "leased");
    }

    #[tokio::test]
    async fn external_gate_rejects_raw_english_fallback_without_publishing() {
        let state = test_state().await;
        seed_agent(
            &state,
            "editorial",
            "reading_article",
            chrono::Utc::now().timestamp() + 300,
        )
        .await;
        sqlx::query("UPDATE agent_jobs SET context_json=? WHERE id='editorial'")
            .bind(json!({"production_mode":"external_agent"}).to_string())
            .execute(&state.db)
            .await
            .unwrap();
        let response=submit_agent_job(State(state.clone()),authed_headers(),Path("editorial".into()),Json(SubmitAgentJobRequest{lease_token:"token".into(),artifact:json!({"title":"Raw copy","original_url":"https://example.org/raw","audio_script":"Here is the original English text, &mdash; including broken entities."})})).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feed_items")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    fn digest_fixture() -> Value {
        let stories: Vec<Value> = (0..3).map(|i| json!({"event_key":format!("event-{i}"),"tier":"major","title":format!("新闻{i}"),"script":format!("这是第{i}项独立新闻。已经核对原始报道，保留事实范围和限制。").repeat(4),"source_urls":[format!("https://example.org/{i}")]})).collect();
        let script = stories
            .iter()
            .map(|v| v["script"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let sources: Vec<Value> = stories
            .iter()
            .map(|v| json!({"url":v["source_urls"][0],"title":v["title"],"summary":"来源证据"}))
            .collect();
        let claims: Vec<Value> = stories.iter().map(|v| json!({"source_url":v["source_urls"][0],"claim":v["title"],"evidence":"原文引文"})).collect();
        json!({"title":"三项新闻汇总","category":"Tech","script":script,"stories":stories,"sources":sources,"editorial":{"ready":true,"dedup_checked":true,"language":"zh-CN","claims":claims,"radio_coverage":{"reviewed":true,"eligible_event_keys":["event-0","event-1","event-2"]}}})
    }

    #[test]
    fn radio_digest_checks_actual_spoken_coverage_and_evidence() {
        assert!(validate_radio_digest(&digest_fixture()).is_ok());
        let mut v = digest_fixture();
        v["stories"][1]["event_key"] = v["stories"][0]["event_key"].clone();
        assert!(validate_radio_digest(&v).is_err());
        let mut v = digest_fixture();
        v["audio_script"] = v["stories"][0]["script"].clone();
        assert!(validate_radio_digest(&v).is_err());
        let mut v = digest_fixture();
        v["stories"][1]["source_urls"] = json!(["https://example.org/missing"]);
        assert!(validate_radio_digest(&v).is_err());
        let mut v = digest_fixture();
        v["editorial"]["claims"] = json!([v["editorial"]["claims"][0]]);
        assert!(validate_radio_digest(&v).is_err());
    }

    #[test]
    fn radio_all_news_supports_briefs_and_genuinely_sparse_days() {
        let mut v = digest_fixture();
        v["stories"][1]["tier"] = json!("brief");
        v["stories"][1]["script"] = json!("另一家公司公布新的芯片，具体上市时间尚未确定。");
        v["script"] = json!(v["stories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["script"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n"));
        assert!(validate_radio_digest(&v).is_ok());
        v["stories"] = json!([v["stories"][0]]);
        v["script"] = v["stories"][0]["script"].clone();
        assert!(validate_radio_digest(&v).is_err());
        v["editorial"]["radio_coverage"]["eligible_event_keys"] = json!(["event-0"]);
        assert!(validate_radio_digest(&v).is_ok());
        v["stories"][0]["tier"] = json!("unknown");
        assert!(validate_radio_digest(&v).is_err());
    }

    #[tokio::test]
    async fn external_missing_eligible_stories_rejected_then_complete_digest_publishes() {
        let state = test_state().await;
        seed_agent(
            &state,
            "digest",
            "radio_episode",
            chrono::Utc::now().timestamp() + 300,
        )
        .await;
        sqlx::query("UPDATE agent_jobs SET context_json=? WHERE id='digest'")
            .bind(json!({"production_mode":"external_agent"}).to_string())
            .execute(&state.db)
            .await
            .unwrap();
        let mut single = digest_fixture();
        single["stories"] = json!([single["stories"][0]]);
        let response = submit_agent_job(
            State(state.clone()),
            authed_headers(),
            Path("digest".into()),
            Json(SubmitAgentJobRequest {
                lease_token: "token".into(),
                artifact: single,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        for table in ["items", "voice_jobs"] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&state.db)
                .await
                .unwrap();
            assert_eq!(count, 0);
        }
        let status: String = sqlx::query_scalar("SELECT status FROM agent_jobs WHERE id='digest'")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(status, "leased");
        let response = submit_agent_job(
            State(state.clone()),
            authed_headers(),
            Path("digest".into()),
            Json(SubmitAgentJobRequest {
                lease_token: "token".into(),
                artifact: digest_fixture(),
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        for table in ["items", "voice_jobs"] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&state.db)
                .await
                .unwrap();
            assert_eq!(count, 1);
        }
        let text: String = sqlx::query_scalar("SELECT text FROM voice_jobs")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(text, digest_fixture()["script"].as_str().unwrap());
    }
    #[tokio::test]
    async fn full_program_is_atomic_and_reuses_one_voice_job() {
        let state = test_state().await;
        seed_agent(
            &state,
            "program",
            "radio_program",
            chrono::Utc::now().timestamp() + 300,
        )
        .await;
        sqlx::query("UPDATE agent_jobs SET context_json=? WHERE id='program'").bind(json!({"production_mode":"external_agent","run_date":"2026-09-22","slot":"morning-program"}).to_string()).execute(&state.db).await.unwrap();
        let artifact = json!({"title":"九月二十二日早间新闻","opening":"这里是今天的早间新闻。","closing":"本期节目到这里，我们下次再见。","sections":[digest_fixture()]});
        let mut bad = artifact.clone();
        bad["sections"][0]["stories"] = json!([]);
        let response = submit_agent_job(
            State(state.clone()),
            authed_headers(),
            Path("program".into()),
            Json(SubmitAgentJobRequest {
                lease_token: "token".into(),
                artifact: bad,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        for _ in 0..2 {
            let response = submit_agent_job(
                State(state.clone()),
                authed_headers(),
                Path("program".into()),
                Json(SubmitAgentJobRequest {
                    lease_token: "token".into(),
                    artifact: artifact.clone(),
                }),
            )
            .await
            .into_response();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let jobs: Vec<(String, String)> = sqlx::query_as("SELECT voice_kind,text FROM voice_jobs")
            .fetch_all(&state.db)
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].0, "radio_program");
        let plan: Value = serde_json::from_str(&jobs[0].1).unwrap();
        assert_eq!(plan["segments"].as_array().unwrap().len(), 3);
        assert_eq!(plan["segments"][1]["category"], "Tech");
        let tags: String = sqlx::query_scalar("SELECT tags FROM items")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert!(tags.contains("radio:program"));
        let original_url: Option<String> = sqlx::query_scalar("SELECT original_url FROM items")
            .fetch_one(&state.db).await.unwrap();
        assert!(original_url.is_none(), "a compilation must not take ownership of a source article");
        let source_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM item_sources")
            .fetch_one(&state.db).await.unwrap();
        assert!(source_count > 0, "program citations must remain available");
    }
}
