// Run CRUD HTTP routes

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, patch},
    Json, Router,
};
use everruns_contracts::{events::AgUiEvent, Run, RunStatus};
use everruns_storage::{models::CreateRun, Database};
use everruns_worker::{WorkflowInput, WorkflowRunner};
use futures::{
    stream::{self, Stream},
    StreamExt,
};
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc, time::Duration};
use utoipa::ToSchema;
use uuid::Uuid;

/// App state
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub runner: Arc<dyn WorkflowRunner>,
}

/// Request to create a run
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRunRequest {
    pub agent_id: Uuid,
    pub thread_id: Uuid,
}

/// Query parameters for listing runs
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListRunsParams {
    pub status: Option<String>,
    pub agent_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    20
}

/// Create run routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/runs", get(list_runs).post(create_run))
        .route("/v1/runs/:run_id", get(get_run))
        .route("/v1/runs/:run_id/cancel", patch(cancel_run))
        .route("/v1/runs/:run_id/events", get(stream_run_events))
        .with_state(state)
}

/// GET /v1/runs - List runs
#[utoipa::path(
    get,
    path = "/v1/runs",
    params(
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("agent_id" = Option<Uuid>, Query, description = "Filter by agent ID"),
        ("limit" = Option<i64>, Query, description = "Max number of results (default 20)"),
        ("offset" = Option<i64>, Query, description = "Offset for pagination")
    ),
    responses(
        (status = 200, description = "List of runs", body = Vec<Run>),
        (status = 500, description = "Internal server error")
    ),
    tag = "runs"
)]
pub async fn list_runs(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<ListRunsParams>,
) -> Result<Json<Vec<Run>>, StatusCode> {
    let rows = state
        .db
        .list_runs(
            params.status.as_deref(),
            params.agent_id,
            params.limit,
            params.offset,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to list runs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let runs: Vec<Run> = rows
        .into_iter()
        .map(|row| Run {
            id: row.id,
            agent_id: row.agent_id,
            thread_id: row.thread_id,
            status: row.status.parse().unwrap_or(RunStatus::Pending),
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
        })
        .collect();

    Ok(Json(runs))
}

/// POST /v1/runs - Create a new run
#[utoipa::path(
    post,
    path = "/v1/runs",
    request_body = CreateRunRequest,
    responses(
        (status = 201, description = "Run created successfully", body = Run),
        (status = 500, description = "Internal server error")
    ),
    tag = "runs"
)]
pub async fn create_run(
    State(state): State<AppState>,
    Json(req): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<Run>), StatusCode> {
    let input = CreateRun {
        agent_id: req.agent_id,
        thread_id: req.thread_id,
    };

    let row = state.db.create_run(input).await.map_err(|e| {
        tracing::error!("Failed to create run: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let run = Run {
        id: row.id,
        agent_id: row.agent_id,
        thread_id: row.thread_id,
        status: row.status.parse().unwrap_or(RunStatus::Pending),
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
    };

    // Start the workflow execution
    let input = WorkflowInput {
        run_id: row.id,
        agent_id: row.agent_id,
        thread_id: row.thread_id,
    };
    state.runner.start_workflow(input).await.map_err(|e| {
        tracing::error!("Failed to start workflow: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::info!(run_id = %row.id, "Workflow started for run");

    Ok((StatusCode::CREATED, Json(run)))
}

/// GET /v1/runs/:run_id
#[utoipa::path(
    get,
    path = "/v1/runs/{run_id}",
    params(
        ("run_id" = Uuid, Path, description = "Run ID")
    ),
    responses(
        (status = 200, description = "Run found", body = Run),
        (status = 404, description = "Run not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "runs"
)]
pub async fn get_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<Run>, StatusCode> {
    let row = state
        .db
        .get_run(run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get run: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let run = Run {
        id: row.id,
        agent_id: row.agent_id,
        thread_id: row.thread_id,
        status: row.status.parse().unwrap_or(RunStatus::Pending),
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
    };

    Ok(Json(run))
}

/// PATCH /v1/runs/:run_id/cancel
#[utoipa::path(
    patch,
    path = "/v1/runs/{run_id}/cancel",
    params(
        ("run_id" = Uuid, Path, description = "Run ID")
    ),
    responses(
        (status = 200, description = "Run cancelled successfully"),
        (status = 404, description = "Run not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "runs"
)]
pub async fn cancel_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    // Verify run exists
    let _run = state
        .db
        .get_run(run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get run: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Cancel the workflow
    state.runner.cancel_workflow(run_id).await.map_err(|e| {
        tracing::error!("Failed to cancel workflow: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::info!(run_id = %run_id, "Workflow cancelled");

    Ok(StatusCode::OK)
}

/// GET /v1/runs/:run_id/events - Stream run events as Server-Sent Events
#[utoipa::path(
    get,
    path = "/v1/runs/{run_id}/events",
    params(
        ("run_id" = Uuid, Path, description = "Run ID")
    ),
    responses(
        (status = 200, description = "Event stream", content_type = "text/event-stream"),
        (status = 404, description = "Run not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "runs"
)]
pub async fn stream_run_events(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // Verify run exists
    let _run = state
        .db
        .get_run(run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get run: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    tracing::info!(run_id = %run_id, "Starting event stream");

    // Clone state for the stream
    let db = state.db.clone();

    // Create stream that replays all events from database
    let stream = stream::unfold(0i64, move |last_sequence| {
        let db = db.clone();
        async move {
            // Fetch events since last sequence
            match db.list_run_events(run_id, Some(last_sequence)).await {
                Ok(events) if !events.is_empty() => {
                    // Get the last sequence number for next iteration
                    let new_sequence = events.last().unwrap().sequence_number;

                    // Convert events to SSE format
                    let sse_events: Vec<Result<Event, Infallible>> = events
                        .into_iter()
                        .map(|event_row| {
                            // Deserialize event_data into AgUiEvent
                            match serde_json::from_value::<AgUiEvent>(event_row.event_data) {
                                Ok(ag_event) => {
                                    // Serialize to JSON for SSE
                                    let json = serde_json::to_string(&ag_event)
                                        .unwrap_or_else(|_| "{}".to_string());

                                    Ok(Event::default()
                                        .event(event_row.event_type)
                                        .data(json)
                                        .id(event_row.sequence_number.to_string()))
                                }
                                Err(e) => {
                                    tracing::error!("Failed to deserialize event: {}", e);
                                    Ok(Event::default()
                                        .event("error")
                                        .data(format!("Failed to deserialize event: {}", e)))
                                }
                            }
                        })
                        .collect();

                    // Emit all events from this batch
                    Some((stream::iter(sse_events), new_sequence))
                }
                Ok(_) => {
                    // No new events, wait a bit before polling again
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Some((stream::iter(vec![]), last_sequence))
                }
                Err(e) => {
                    tracing::error!("Failed to fetch events: {}", e);
                    None
                }
            }
        }
    })
    .flatten();

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
