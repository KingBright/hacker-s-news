use crate::AppState;
use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use std::path::{Path, PathBuf};
use tokio::{fs, io::AsyncWriteExt};

pub async fn upload_audio(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    multipart: Multipart,
) -> Response {
    if headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok()) != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }
    save_upload(multipart, Path::new(&state.audio_dir)).await
}

// A cancelled or malformed upload must never become a publicly playable file.
struct TemporaryUpload(PathBuf);
impl Drop for TemporaryUpload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
async fn save_upload(mut multipart: Multipart, directory: &Path) -> Response {
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => return (StatusCode::BAD_REQUEST, "No file provided").into_response(),
            Err(_) => return (StatusCode::BAD_REQUEST, "Invalid multipart body").into_response(),
        };
        if field.name() != Some("file") {
            continue;
        }
        let name = Path::new(field.file_name().unwrap_or("audio.mp3"))
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("audio.mp3")
            .to_owned();
        let temp = TemporaryUpload(directory.join(format!(".upload-{}", uuid::Uuid::new_v4())));
        let mut file = match fs::File::create(&temp.0).await {
            Ok(file) => file,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Cannot create audio upload",
                )
                    .into_response()
            }
        };
        let mut field = field;
        let mut written = 0;
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if file.write_all(&chunk).await.is_err() {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Cannot write audio upload",
                        )
                            .into_response();
                    }
                    written += chunk.len();
                }
                Ok(None) => break,
                Err(_) => {
                    return (StatusCode::BAD_REQUEST, "Incomplete audio upload").into_response()
                }
            }
        }
        if written == 0 {
            return (StatusCode::BAD_REQUEST, "Empty audio upload").into_response();
        }
        if file.flush().await.is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot flush audio upload",
            )
                .into_response();
        }
        drop(file);
        if fs::rename(&temp.0, directory.join(&name)).await.is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot publish audio upload",
            )
                .into_response();
        }
        return Json(json!({"url":format!("/audio/{name}"),"filename":name})).into_response();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, extract::FromRequest, http::Request};
    #[tokio::test]
    async fn truncated_upload_is_rejected_without_replacing_existing_audio() {
        let directory =
            std::env::temp_dir().join(format!("freshloop-upload-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).await.unwrap();
        fs::write(directory.join("test.mp3"), b"original")
            .await
            .unwrap();
        for complete in [false, true] {
            let body=format!("--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.mp3\"\r\nContent-Type: audio/mpeg\r\n\r\nreplacement{}",if complete {"\r\n--boundary--\r\n"}else{""});
            let request = Request::builder()
                .header("content-type", "multipart/form-data; boundary=boundary")
                .body(Body::from(body))
                .unwrap();
            let multipart = Multipart::from_request(request, &()).await.unwrap();
            let response = save_upload(multipart, &directory).await;
            assert_eq!(
                response.status(),
                if complete {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_REQUEST
                }
            );
            assert_eq!(
                fs::read(directory.join("test.mp3")).await.unwrap(),
                if complete {
                    b"replacement".to_vec()
                } else {
                    b"original".to_vec()
                }
            );
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        }
        fs::remove_dir_all(directory).await.unwrap();
    }
}
