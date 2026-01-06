use axum::{
    extract::{State, Json},
    http::{StatusCode, HeaderMap},
};
use serde::{Deserialize, Serialize};
use crate::AppState;
use bcrypt::{hash, verify, DEFAULT_COST};
use rand::Rng;
use rand::distributions::Alphanumeric;
use sqlx::Row; // Import Row trait for get()

#[derive(Deserialize)]
pub struct CreateUserRequest {
    username: String,
    // Admin can specify a password OR we generate one
    password: Option<String>,
}

#[derive(Serialize)]
pub struct CreateUserResponse {
    id: String,
    username: String,
    password_generated: String, // Helper for admin to copy
}

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    id: String,
    username: String,
}

#[derive(Serialize)]
pub struct User {
    id: String,
    username: String,
    created_at: i64,
}

// Admin: Create User
pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateUserRequest>,
) ->  Result<Json<CreateUserResponse>, StatusCode> {
    // 1. Verify Admin Key
    let api_key = headers.get("x-api-key")
        .and_then(|v| v.to_str().ok());
    
    if api_key != Some(&state.api_key) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // 2. Generate Password if not provided
    let password_plain = payload.password.unwrap_or_else(|| {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect()
    });

    // 3. Hash Password
    let password_hash = hash(&password_plain, DEFAULT_COST)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let user_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    // 4. Insert into DB
    let res = sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)"
    )
    .bind(&user_id)
    .bind(&payload.username)
    .bind(&password_hash)
    .bind(now)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => Ok(Json(CreateUserResponse {
            id: user_id,
            username: payload.username,
            password_generated: password_plain,
        })),
        Err(_) => Err(StatusCode::CONFLICT), // Likely username exists
    }
}

// Public: Login
pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    let row = sqlx::query("SELECT id, username, password_hash FROM users WHERE username = ?")
        .bind(&payload.username)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Some(row) = row {
        let id: String = row.try_get("id").unwrap_or_default();
        let stored_hash: String = row.try_get("password_hash").unwrap_or_default();
        let username: String = row.try_get("username").unwrap_or_default();

        if verify(&payload.password, &stored_hash).unwrap_or(false) {
            return Ok(Json(LoginResponse {
                id,
                username,
            }));
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

// Admin: List Users
pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<User>>, StatusCode> {
     // 1. Verify Admin Key
     let api_key = headers.get("x-api-key")
         .and_then(|v| v.to_str().ok());
     
     if api_key != Some(&state.api_key) {
         return Err(StatusCode::UNAUTHORIZED);
     }

     let rows = sqlx::query("SELECT id, username, created_at FROM users ORDER BY created_at DESC")
         .fetch_all(&state.db)
         .await
         .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

     let users = rows.iter().map(|row| User {
         id: row.try_get("id").unwrap_or_default(),
         username: row.try_get("username").unwrap_or_default(),
         created_at: row.try_get("created_at").unwrap_or_default(),
     }).collect();

     Ok(Json(users))
}
