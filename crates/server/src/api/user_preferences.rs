// User Preferences API — a per-user key/value store for client/UI settings.
//
// Decision: User-scoped (not org-scoped). Preferences belong to the
// authenticated user and persist across orgs, devices, and browsers. Values are
// arbitrary JSON; the storage layer keeps them as TEXT so the schema stays
// agnostic about any individual preference's shape (e.g. a dismissed banner
// flag).

use crate::auth::middleware::{AuthState, AuthUser};
use crate::storage::{StorageBackend, backend::USER_PREFERENCE_LIMIT_EXCEEDED};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::impl_auth_state;
use crate::storage::models::UserPreferenceRow;

const MAX_PREFERENCES_PER_USER: usize = 100;
const MAX_PREFERENCE_VALUE_BYTES: usize = 4 * 1024;

fn validate_preference(key: &str, value: &str) -> Result<(), StatusCode> {
    if key.is_empty() || key.len() > 255 {
        return Err(StatusCode::BAD_REQUEST);
    }
    if value.len() > MAX_PREFERENCE_VALUE_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    Ok(())
}

/// App state for user preference routes.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }
}

impl_auth_state!(AppState);

/// A single stored preference. `value` is arbitrary JSON.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreferenceResponse {
    pub key: String,
    #[schema(value_type = Object)]
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

/// Request body for setting a preference value.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetPreferenceRequest {
    #[schema(value_type = Object)]
    pub value: serde_json::Value,
}

fn row_to_response(row: UserPreferenceRow) -> PreferenceResponse {
    // Stored as TEXT containing serialized JSON. Fall back to treating the raw
    // text as a string value if it was written as a non-JSON literal.
    let value = serde_json::from_str(&row.value)
        .unwrap_or_else(|_| serde_json::Value::String(row.value.clone()));
    PreferenceResponse {
        key: row.key,
        value,
        updated_at: row.updated_at,
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/user/preferences", get(list_preferences))
        .route(
            "/v1/user/preferences/{key}",
            get(get_preference)
                .put(set_preference)
                .delete(delete_preference),
        )
        .with_state(state)
}

/// GET /v1/user/preferences — list all of the user's preferences.
pub async fn list_preferences(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<PreferenceResponse>>, StatusCode> {
    let rows = state
        .db
        .list_user_preferences(auth.id, MAX_PREFERENCES_PER_USER)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list user preferences: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(rows.into_iter().map(row_to_response).collect()))
}

/// GET /v1/user/preferences/{key} — read a single preference by key.
pub async fn get_preference(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
) -> Result<Json<PreferenceResponse>, StatusCode> {
    let row = state
        .db
        .get_user_preference(auth.id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get user preference: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(row_to_response(row)))
}

/// PUT /v1/user/preferences/{key} — create or update a preference value.
pub async fn set_preference(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
    Json(body): Json<SetPreferenceRequest>,
) -> Result<Json<PreferenceResponse>, StatusCode> {
    let serialized = serde_json::to_string(&body.value).map_err(|e| {
        tracing::error!("Failed to serialize preference value: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    validate_preference(&key, &serialized)?;

    let row = state
        .db
        .set_user_preference(auth.id, &key, &serialized, MAX_PREFERENCES_PER_USER)
        .await
        .map_err(|e| {
            if e.to_string() == USER_PREFERENCE_LIMIT_EXCEEDED {
                return StatusCode::CONFLICT;
            }
            tracing::error!("Failed to set user preference: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(row_to_response(row)))
}

/// DELETE /v1/user/preferences/{key} — remove a preference.
pub async fn delete_preference(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state
        .db
        .delete_user_preference(auth.id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete user preference: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_preference_key_and_value_limits() {
        assert_eq!(
            validate_preference("", "true"),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            validate_preference(&"k".repeat(256), "true"),
            Err(StatusCode::BAD_REQUEST)
        );
        assert!(validate_preference("key", &"x".repeat(4096)).is_ok());
        assert_eq!(
            validate_preference("key", &"x".repeat(4097)),
            Err(StatusCode::PAYLOAD_TOO_LARGE)
        );
    }
}
