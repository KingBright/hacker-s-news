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
                        .unwrap_or("audio.mp3")
                        .to_string();

                    let filepath = Path::new(&state.audio_dir).join(&sanitized_file_name);
                    
                    // Create file for streaming
                    let mut file = match fs::File::create(&filepath).await {
                        Ok(f) => f,
                        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create file: {}", e)).into_response(),
                    };

                    // Stream chunks directly to file
                    let mut field = field; // make mutable
                    while let Ok(Some(chunk)) = field.chunk().await {
                         // use tokio::io::AsyncWriteExt;
                         if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await {
                             return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write chunk: {}", e)).into_response();
                         }
                    }

                    return Json(json!({
                        "url": format!("/audio/{}", sanitized_file_name),
                        "filename": sanitized_file_name
                    })).into_response();
                }
            },
            Ok(None) => break, 
            Err(e) => return (StatusCode::BAD_REQUEST, format!("Multipart error: {}", e)).into_response(),
        }
    }

    (StatusCode::BAD_REQUEST, "No file found").into_response()
}
