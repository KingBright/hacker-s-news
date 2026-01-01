use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde_json::json;
use std::path::Path;
use tokio::fs;
use uuid::Uuid;

use crate::AppState;

pub async fn upload_audio(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }
    loop {
        let field_res = multipart.next_field().await;
        match field_res {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or("").to_string();
                tracing::info!("Multipart field: {}", name);
                if name == "file" {
                    let file_name = field.file_name().unwrap_or("audio.mp3").to_string();
                    let sanitized_file_name = Path::new(&file_name)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("audio.mp3");

                    // let id = Uuid::new_v4();
                    // let new_filename = format!("{}-{}", id, sanitized_file_name);
                    let new_filename = sanitized_file_name.to_string();
                    let filepath = Path::new(&state.audio_dir).join(&new_filename);

                    let data_res = field.bytes().await;
                    match data_res {
                        Ok(data) => {
                             if let Err(e) = fs::write(&filepath, data).await {
                                return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save file: {}", e)).into_response();
                            }

                            return Json(json!({
                                "url": format!("/audio/{}", new_filename),
                                "filename": new_filename
                            })).into_response();
                        },
                        Err(e) => {
                            return (StatusCode::BAD_REQUEST, format!("Failed to read file data: {}", e)).into_response();
                        }
                    }
                }
            },
            Ok(None) => break, // No more fields
            Err(e) => {
                return (StatusCode::BAD_REQUEST, format!("Multipart error: {}", e)).into_response();
            }
        }
    }

    (StatusCode::BAD_REQUEST, "No file found").into_response()
}
