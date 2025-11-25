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
    mut multipart: Multipart,
) -> impl IntoResponse {
    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_string();
        if name == "file" {
            let file_name = field.file_name().unwrap_or("audio.mp3").to_string();
            // Sanitize filename to prevent directory traversal
            let sanitized_file_name = Path::new(&file_name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("audio.mp3");

            // Generate a unique filename
            let id = Uuid::new_v4();
            let new_filename = format!("{}-{}", id, sanitized_file_name);
            let filepath = Path::new(&state.audio_dir).join(&new_filename);

            let data = field.bytes().await.unwrap();

            if let Err(e) = fs::write(&filepath, data).await {
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save file: {}", e)).into_response();
            }

            return Json(json!({
                "url": format!("/audio/{}", new_filename),
                "filename": new_filename
            })).into_response();
        }
    }

    (StatusCode::BAD_REQUEST, "No file found").into_response()
}
