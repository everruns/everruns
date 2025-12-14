// Thread CRUD HTTP routes

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_contracts::{Message, Thread};
use everruns_storage::{
    models::{CreateMessage, CreateThread},
    Database,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// App state
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
}

/// Request to create a thread
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateThreadRequest {}

/// Request to create a message
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMessageRequest {
    pub role: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

/// Create thread routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/threads", post(create_thread))
        .route("/v1/threads/:thread_id", get(get_thread))
        .route(
            "/v1/threads/:thread_id/messages",
            post(create_message).get(list_messages),
        )
        .with_state(state)
}

/// POST /v1/threads - Create a new thread
#[utoipa::path(
    post,
    path = "/v1/threads",
    request_body = CreateThreadRequest,
    responses(
        (status = 201, description = "Thread created successfully", body = Thread),
        (status = 500, description = "Internal server error")
    ),
    tag = "threads"
)]
pub async fn create_thread(
    State(state): State<AppState>,
    Json(_req): Json<CreateThreadRequest>,
) -> Result<(StatusCode, Json<Thread>), StatusCode> {
    let input = CreateThread {};

    let row = state.db.create_thread(input).await.map_err(|e| {
        tracing::error!("Failed to create thread: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let thread = Thread {
        id: row.id,
        created_at: row.created_at,
    };

    Ok((StatusCode::CREATED, Json(thread)))
}

/// GET /v1/threads/:thread_id
#[utoipa::path(
    get,
    path = "/v1/threads/{thread_id}",
    params(
        ("thread_id" = Uuid, Path, description = "Thread ID")
    ),
    responses(
        (status = 200, description = "Thread found", body = Thread),
        (status = 404, description = "Thread not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "threads"
)]
pub async fn get_thread(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Thread>, StatusCode> {
    let row = state
        .db
        .get_thread(thread_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get thread: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let thread = Thread {
        id: row.id,
        created_at: row.created_at,
    };

    Ok(Json(thread))
}

/// POST /v1/threads/:thread_id/messages - Add a message to the thread
#[utoipa::path(
    post,
    path = "/v1/threads/{thread_id}/messages",
    params(
        ("thread_id" = Uuid, Path, description = "Thread ID")
    ),
    request_body = CreateMessageRequest,
    responses(
        (status = 201, description = "Message created successfully", body = Message),
        (status = 500, description = "Internal server error")
    ),
    tag = "threads"
)]
pub async fn create_message(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(req): Json<CreateMessageRequest>,
) -> Result<(StatusCode, Json<Message>), StatusCode> {
    let input = CreateMessage {
        thread_id,
        role: req.role,
        content: req.content,
        metadata: req.metadata,
    };

    let row = state.db.create_message(input).await.map_err(|e| {
        tracing::error!("Failed to create message: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let message = Message {
        id: row.id,
        thread_id: row.thread_id,
        role: row.role,
        content: row.content,
        metadata: row.metadata,
        created_at: row.created_at,
    };

    Ok((StatusCode::CREATED, Json(message)))
}

/// GET /v1/threads/:thread_id/messages - List all messages in the thread
#[utoipa::path(
    get,
    path = "/v1/threads/{thread_id}/messages",
    params(
        ("thread_id" = Uuid, Path, description = "Thread ID")
    ),
    responses(
        (status = 200, description = "List of messages", body = Vec<Message>),
        (status = 500, description = "Internal server error")
    ),
    tag = "threads"
)]
pub async fn list_messages(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<Message>>, StatusCode> {
    let rows = state.db.list_messages(thread_id).await.map_err(|e| {
        tracing::error!("Failed to list messages: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let messages: Vec<Message> = rows
        .into_iter()
        .map(|row| Message {
            id: row.id,
            thread_id: row.thread_id,
            role: row.role,
            content: row.content,
            metadata: row.metadata,
            created_at: row.created_at,
        })
        .collect();

    Ok(Json(messages))
}
