// AG-UI Protocol runtime endpoint for CopilotKit integration
// Spec: https://docs.ag-ui.com

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::Response,
    routing::post,
    Json, Router,
};
use everruns_contracts::events::AgUiEvent;
use everruns_storage::{
    models::{CreateMessage, CreateThread},
    Database,
};
use everruns_worker::AgentRunner;
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};
use uuid::Uuid;

/// App state
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub runner: Arc<dyn AgentRunner>,
}

/// AG-UI query parameters
#[derive(Debug, Deserialize)]
pub struct AgUiParams {
    pub agent_id: Uuid,
    pub thread_id: Option<Uuid>,
}

/// AG-UI request body (messages from CopilotKit)
#[derive(Debug, Deserialize)]
pub struct AgUiRequest {
    pub messages: Vec<AgUiMessage>,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// AG-UI message format
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AgUiMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Create AG-UI routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/ag-ui", post(handle_ag_ui_request))
        .with_state(state)
}

/// POST /v1/ag-ui - Handle AG-UI runtime requests from CopilotKit
pub async fn handle_ag_ui_request(
    State(state): State<AppState>,
    Query(params): Query<AgUiParams>,
    Json(body): Json<AgUiRequest>,
) -> Result<Response, StatusCode> {
    tracing::info!(
        agent_id = %params.agent_id,
        message_count = body.messages.len(),
        "Received AG-UI request"
    );

    // Get or create thread
    let thread_id = if let Some(tid) = params
        .thread_id
        .or_else(|| body.thread_id.as_ref().and_then(|s| s.parse().ok()))
    {
        tid
    } else {
        // Create new thread
        let thread = state.db.create_thread(CreateThread {}).await.map_err(|e| {
            tracing::error!("Failed to create thread: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        thread.id
    };

    // Save messages to thread (skip if already saved)
    for msg in &body.messages {
        // Only save user messages (assistant messages are saved by the workflow)
        if msg.role == "user" {
            let create_msg = CreateMessage {
                thread_id,
                role: msg.role.clone(),
                content: msg.content.clone(),
                metadata: None,
            };

            // Ignore duplicate errors
            let _ = state.db.create_message(create_msg).await;
        }
    }

    // Create a new run
    let run = state
        .db
        .create_run(everruns_storage::models::CreateRun {
            agent_id: params.agent_id,
            thread_id,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create run: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let run_id = run.id;

    // Start run execution in background using the configured runner
    let runner = state.runner.clone();
    tokio::spawn(async move {
        if let Err(e) = runner.start_run(run_id, params.agent_id, thread_id).await {
            tracing::error!(run_id = %run_id, error = %e, "Run execution failed");
        }
    });

    // Create SSE stream that polls for events
    let db = state.db.clone();
    let stream = create_event_stream(db, run_id, thread_id);

    // Return SSE response
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header("X-Thread-Id", thread_id.to_string())
        .header("X-Run-Id", run_id.to_string())
        .body(Body::from_stream(stream))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}

/// Create an SSE stream that polls for AG-UI events
fn create_event_stream(
    db: Arc<Database>,
    run_id: Uuid,
    _thread_id: Uuid,
) -> impl Stream<Item = Result<String, Infallible>> {
    stream::unfold((0i64, false), move |(last_sequence, finished)| {
        let db = db.clone();
        async move {
            if finished {
                return None;
            }

            // Poll for new events
            match db.list_run_events(run_id, Some(last_sequence)).await {
                Ok(events) if !events.is_empty() => {
                    let new_sequence = events.last().unwrap().sequence_number;

                    let mut output = String::new();
                    let mut is_finished = false;

                    for event_row in events {
                        // Parse the event
                        if let Ok(ag_event) =
                            serde_json::from_value::<AgUiEvent>(event_row.event_data.clone())
                        {
                            // Check if run is finished
                            if matches!(
                                ag_event,
                                AgUiEvent::RunFinished(_) | AgUiEvent::RunError(_)
                            ) {
                                is_finished = true;
                            }

                            // Format as SSE
                            let json = serde_json::to_string(&ag_event).unwrap_or_default();
                            output.push_str(&format!("data: {}\n\n", json));
                        }
                    }

                    Some((Ok(output), (new_sequence, is_finished)))
                }
                Ok(_) => {
                    // No new events, wait before polling again
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    // Send empty keepalive
                    Some((Ok(": keepalive\n\n".to_string()), (last_sequence, finished)))
                }
                Err(e) => {
                    tracing::error!("Failed to fetch events: {}", e);
                    None
                }
            }
        }
    })
}
