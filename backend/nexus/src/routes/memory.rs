use crate::{personalization, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use loop_memory::{
    build_user_profile, format_profile_for_prompt_budgeted, MemoryEntry, MemoryQuery, MemoryStore,
    MemoryType, Provenance,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 200;

#[derive(Debug, Deserialize)]
pub struct CreateMemoryEntryRequest {
    pub content: String,
    pub memory_type: Option<MemoryType>,
    pub strength: Option<f32>,
    pub source_ref: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    pub is_static: Option<bool>,
    pub forget_after: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateInternalMemoryEntryRequest {
    pub user_id: String,
    pub content: String,
    pub memory_type: Option<MemoryType>,
    pub strength: Option<f32>,
    pub source_ref: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    pub provenance: Option<Provenance>,
    pub confidence: Option<f32>,
    pub is_static: Option<bool>,
    pub forget_after: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryListQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct MemorySearchQuery {
    pub q: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct MemoryProfileResponse {
    pub profile: loop_memory::UserProfile,
    pub prompt_context: String,
}

fn user_namespace(headers: &HeaderMap) -> Result<String, StatusCode> {
    let Some(user_id) = headers
        .get("x-user-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    Ok(format!("user:{}", user_id))
}

fn namespace_for_user_id(user_id: &str) -> Result<String, StatusCode> {
    let trimmed = user_id.trim();
    if trimmed.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(format!("user:{}", trimmed))
}

fn has_internal_auth(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get("X-NEXUS-KEY")
        .and_then(|value| value.to_str().ok())
        == Some(state.api_key.as_str())
}

fn sanitize_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT)
}

pub async fn create_memory_entry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateMemoryEntryRequest>,
) -> impl IntoResponse {
    let namespace = match user_namespace(&headers) {
        Ok(namespace) => namespace,
        Err(status) => return status.into_response(),
    };
    let content = payload.content.trim();
    if content.is_empty() {
        return (StatusCode::BAD_REQUEST, "content is required").into_response();
    }

    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let mut entry = MemoryEntry::new(
        Uuid::new_v4().to_string(),
        payload.memory_type.unwrap_or(MemoryType::UserExpression),
        content.to_string(),
        now,
        None,
    );
    let strength = payload.strength.unwrap_or(1.0).clamp(0.1, 5.0);
    entry.base_strength = strength;
    entry.current_strength = strength;
    entry.namespace = Some(namespace);
    entry.provenance = Provenance::UserExplicit;
    entry.source_ref = payload.source_ref;
    entry.metadata = payload.metadata.unwrap_or_default();
    entry.is_static = payload.is_static.unwrap_or(false);
    entry.forget_after = payload.forget_after;

    match state.memory_store.store(entry.clone()).await {
        Ok(_) => (StatusCode::CREATED, Json(entry)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn create_memory_entry_internal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateInternalMemoryEntryRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let namespace = match namespace_for_user_id(&payload.user_id) {
        Ok(namespace) => namespace,
        Err(status) => return status.into_response(),
    };
    let content = payload.content.trim();
    if content.is_empty() {
        return (StatusCode::BAD_REQUEST, "content is required").into_response();
    }

    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let mut entry = MemoryEntry::new(
        Uuid::new_v4().to_string(),
        payload.memory_type.unwrap_or(MemoryType::PreferenceSignal),
        content.to_string(),
        now,
        None,
    );
    let strength = payload.strength.unwrap_or(1.0).clamp(0.1, 5.0);
    entry.base_strength = strength;
    entry.current_strength = strength;
    entry.namespace = Some(namespace);
    entry.provenance = payload.provenance.unwrap_or(Provenance::LlmExtracted);
    entry.confidence = payload.confidence.unwrap_or(1.0).clamp(0.0, 1.0);
    entry.source_ref = payload.source_ref;
    entry.metadata = payload.metadata.unwrap_or_default();
    entry.is_static = payload.is_static.unwrap_or(false);
    entry.forget_after = payload.forget_after;

    match state.memory_store.store(entry.clone()).await {
        Ok(_) => (StatusCode::CREATED, Json(entry)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn list_memory_entries(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<MemoryListQuery>,
) -> impl IntoResponse {
    let namespace = match user_namespace(&headers) {
        Ok(namespace) => namespace,
        Err(status) => return status.into_response(),
    };
    let limit = sanitize_limit(query.limit);

    let mut entries = match state
        .memory_store
        .retrieve(MemoryQuery::TimeRange {
            start: 0,
            end: u64::MAX,
            namespace: Some(namespace),
        })
        .await
    {
        Ok(entries) => entries,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    entries.truncate(limit);
    Json(entries).into_response()
}

pub async fn search_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<MemorySearchQuery>,
) -> impl IntoResponse {
    let namespace = match user_namespace(&headers) {
        Ok(namespace) => namespace,
        Err(status) => return status.into_response(),
    };
    let search_text = query.q.trim();
    if search_text.is_empty() {
        return (StatusCode::BAD_REQUEST, "q is required").into_response();
    }

    let limit = sanitize_limit(query.limit);
    match state
        .memory_store
        .retrieve(MemoryQuery::SemanticSearch {
            query: search_text.to_string(),
            top_k: limit,
            namespace: Some(namespace),
        })
        .await
    {
        Ok(mut entries) => {
            entries.sort_by(|a, b| {
                b.similarity_score
                    .unwrap_or(0.0)
                    .partial_cmp(&a.similarity_score.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.created_at.cmp(&a.created_at))
            });
            entries.truncate(limit);
            Json(entries).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn get_memory_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let namespace = match user_namespace(&headers) {
        Ok(namespace) => namespace,
        Err(status) => return status.into_response(),
    };
    let profile = build_user_profile(&*state.memory_store, Some(&namespace)).await;
    let prompt_context = format_profile_for_prompt_budgeted(&profile, Some(3200));
    Json(MemoryProfileResponse {
        profile,
        prompt_context,
    })
    .into_response()
}

pub async fn get_focus_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(user_id) = headers
        .get("x-user-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    match personalization::get_focus_summary(&state, user_id).await {
        Ok(summary) => Json(summary).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn get_memory_profile_internal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let namespace = match namespace_for_user_id(&user_id) {
        Ok(namespace) => namespace,
        Err(status) => return status.into_response(),
    };
    let profile = build_user_profile(&*state.memory_store, Some(&namespace)).await;
    let prompt_context = format_profile_for_prompt_budgeted(&profile, Some(3200));
    Json(MemoryProfileResponse {
        profile,
        prompt_context,
    })
    .into_response()
}

pub async fn delete_memory_entry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let namespace = match user_namespace(&headers) {
        Ok(namespace) => namespace,
        Err(status) => return status.into_response(),
    };

    let entries = match state
        .memory_store
        .retrieve(MemoryQuery::TimeRange {
            start: 0,
            end: u64::MAX,
            namespace: Some(namespace),
        })
        .await
    {
        Ok(entries) => entries,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    if !entries.iter().any(|entry| entry.id == id) {
        return StatusCode::NOT_FOUND.into_response();
    }

    match state.memory_store.delete(&id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
