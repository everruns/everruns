// AG-UI app channel — anonymous, app-scoped streaming endpoint
//
// Design Decision: AG-UI ingress is app-scoped (POST /v1/apps/{app_id}/ag-ui)
// so the App controls the agent, harness, identity, and publication lifecycle.
//
// Design Decision: The endpoint is anonymous for the initial rollout. Requests
// are accepted without API auth when the app is published and an enabled AG-UI
// channel is present.
//
// Design Decision: The AG-UI stream is translated from Everruns session events
// instead of bypassing the durable runtime. This keeps app-channel behavior
// aligned with normal sessions and preserves streaming parity.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::Arc;

use ag_ui_core::event::{
    BaseEvent as AgUiBaseEvent, Event as AgUiEvent,
    MessagesSnapshotEvent as AgUiMessagesSnapshotEvent, RunErrorEvent as AgUiRunErrorEvent,
    RunFinishedEvent as AgUiRunFinishedEvent, RunStartedEvent as AgUiRunStartedEvent,
    TextMessageContentEvent as AgUiTextMessageContentEvent,
    TextMessageEndEvent as AgUiTextMessageEndEvent,
    TextMessageStartEvent as AgUiTextMessageStartEvent, ThinkingEndEvent as AgUiThinkingEndEvent,
    ThinkingStartEvent as AgUiThinkingStartEvent,
    ThinkingTextMessageContentEvent as AgUiThinkingTextMessageContentEvent,
    ThinkingTextMessageEndEvent as AgUiThinkingTextMessageEndEvent,
    ThinkingTextMessageStartEvent as AgUiThinkingTextMessageStartEvent,
    ToolCallArgsEvent as AgUiToolCallArgsEvent, ToolCallEndEvent as AgUiToolCallEndEvent,
    ToolCallResultEvent as AgUiToolCallResultEvent, ToolCallStartEvent as AgUiToolCallStartEvent,
};
use ag_ui_core::types::{
    ids::{
        MessageId as AgUiMessageId, RunId as AgUiRunId, ThreadId as AgUiThreadId,
        ToolCallId as AgUiToolCallId,
    },
    input::RunAgentInput as AgUiRunAgentInput,
    message::{Message as AgUiMessage, Role as AgUiRole},
    tool::ToolCall as AgUiToolCall,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::post,
};
use everruns_core::events::{
    OutputMessageCompletedData, OutputMessageDeltaData, ReasonThinkingCompletedData,
    ReasonThinkingDeltaData, ReasonThinkingStartedData, ToolCompletedData, ToolStartedData,
};
use everruns_core::message_retriever::InputMessage as StoredInputMessage;
use everruns_core::{App, AppStatus, Caller, ContentPart, ExternalActor, MessageRole};
use futures::{
    StreamExt,
    stream::{self, Stream},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::common::ErrorResponse;
use crate::api::messages::{
    CreateMessageRequest, InputContentPart, InputMessage, MessageRole as ApiMessageRole,
};
use crate::api::sessions::CreateSessionRequest;
use crate::api::sse::SseConnectionTracker;
use crate::execution_metadata;
use crate::services::{
    AppService, CreateMessageContext, EventService, MessageService, SessionService,
};
use crate::storage::{DbMessageRetriever, StorageBackend};

#[derive(Clone)]
pub struct AgUiState {
    pub db: Arc<StorageBackend>,
    pub app_service: Arc<AppService>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    pub sse_tracker: Arc<SseConnectionTracker>,
}

impl AgUiState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<crate::storage::EncryptionService>>,
        runner: Arc<dyn everruns_worker::AgentRunner>,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
        sse_tracker: Arc<SseConnectionTracker>,
    ) -> Self {
        Self {
            app_service: Arc::new(AppService::new(db.clone(), encryption)),
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner,
                notifications_enabled,
                event_delivery.clone(),
            )),
            event_service: Arc::new(EventService::new(db.clone(), event_delivery)),
            sse_tracker,
            db,
        }
    }
}

pub fn routes(state: AgUiState) -> Router {
    Router::new()
        .route("/v1/apps/{app_id}/ag-ui", post(run_agent))
        .with_state(state)
}

async fn run_agent(
    State(state): State<AgUiState>,
    Path(app_id): Path<String>,
    Json(req): Json<AgUiRunAgentInput>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, Json<ErrorResponse>)>
{
    let app = state
        .app_service
        .get_by_public_id_unscoped(&app_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;

    // THREAT[TM-AUTHZ-005]: Anonymous AG-UI requests must not reach draft or
    // private app configurations.
    // Mitigation: Require a published app, an enabled AG-UI channel, and
    // `anonymous=true` before accepting unauthenticated traffic.
    if app.status != AppStatus::Published {
        return Err(forbidden("App is not published"));
    }

    let channel = app
        .ag_ui_channel()
        .ok_or_else(|| bad_request("App does not have an enabled AG-UI channel"))?;
    let channel_config = channel
        .ag_ui_config()
        .ok_or_else(|| bad_request("Invalid AG-UI channel configuration"))?;
    if !channel_config.anonymous {
        return Err(forbidden("Anonymous AG-UI access is disabled"));
    }

    let trigger_message = req
        .messages
        .last()
        .ok_or_else(|| bad_request("messages must contain at least one user message"))?;
    let (trigger_content, trigger_name) = match trigger_message {
        AgUiMessage::User { content, name, .. } => (content.clone(), name.clone()),
        _ => return Err(bad_request("the final AG-UI message must have role=user")),
    };

    let thread_id = req.thread_id.clone();
    let run_id = req.run_id.clone();
    let thread_tag = thread_id.to_string();
    let run_tag = run_id.to_string();

    // THREAT[TM-TENANT-009]: Reusing the same AG-UI thread ID across apps must
    // not merge tenants or app sessions.
    // Mitigation: Scope the session lookup tags by both app public ID and
    // thread ID so thread collisions stay isolated per app.
    let routing_tags = vec![
        format!("ag_ui:app:{}", app.public_id),
        format!("ag_ui:thread:{}", thread_tag),
    ];
    let session = find_or_create_session(&state, &app, &routing_tags, &thread_tag, &req)
        .await
        .map_err(internal_error)?;

    // THREAT[TM-DOS-010]: Anonymous AG-UI streams must still respect server-wide
    // SSE connection limits.
    // Mitigation: Reuse the shared SSE tracker for per-org and per-session limits
    // before opening the stream.
    let sse_guard = state
        .sse_tracker
        .try_acquire(app.org_id, session.session.id.uuid())
        .map_err(|rejection| {
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse {
                    error: rejection.to_string(),
                }),
            )
        })?;

    // Seed prior history only on first use of the thread so a new AG-UI client can
    // carry conversation context into the durable session without triggering old runs.
    if session.is_new {
        seed_history(
            &state,
            session.session.id.uuid(),
            &req.messages[..req.messages.len() - 1],
        )
        .await
        .map_err(internal_error)?;
    }

    let subscription = state
        .event_service
        .event_delivery()
        .clone()
        .subscribe(session.session.id.uuid())
        .await
        .map_err(internal_error)?;

    let message = state
        .message_service
        .create(
            CreateMessageContext {
                org_id: app.org_id,
                user_id: None,
                harness_id: app.harness_id.uuid(),
                agent_id: Some(app.agent_id.uuid()),
                session_id: session.session.id.uuid(),
                event_metadata: Some(execution_metadata::app_message_metadata(
                    app.public_id,
                    app.agent_identity_id,
                )),
                request_id: None,
            },
            CreateMessageRequest {
                message: InputMessage {
                    role: ApiMessageRole::User,
                    content: vec![InputContentPart::text(trigger_content)],
                },
                controls: None,
                metadata: Some(
                    [
                        ("ag_ui_thread_id".to_string(), Value::String(thread_tag)),
                        ("ag_ui_run_id".to_string(), Value::String(run_tag)),
                    ]
                    .into_iter()
                    .collect(),
                ),
                tags: None,
                external_actor: build_external_actor(trigger_name.as_ref()),
            },
        )
        .await
        .map_err(internal_error)?;

    let snapshot_messages = state
        .message_service
        .list(session.session.id.uuid())
        .await
        .map_err(internal_error)?;

    let initial_events = vec![
        AgUiEvent::RunStarted(AgUiRunStartedEvent {
            base: agui_base_event(),
            thread_id: req.thread_id.clone(),
            run_id: req.run_id.clone(),
        }),
        AgUiEvent::MessagesSnapshot(AgUiMessagesSnapshotEvent {
            base: agui_base_event(),
            messages: snapshot_messages
                .iter()
                .map(to_ag_ui_message)
                .collect::<Vec<_>>(),
        }),
    ];

    let stream_state = AgUiStreamState {
        subscription: Box::new(subscription),
        session_id: session.session.id.uuid(),
        input_message_id: message.id.to_string(),
        thread_id,
        run_id,
        queue: VecDeque::new(),
        assistant_message_id: None,
        assistant_content_started: false,
        assistant_emitted_delta: false,
        thinking_started: false,
        thinking_text_started: false,
        tool_call_ids: HashMap::new(),
        finished: false,
    };

    let initial_stream = stream::iter(initial_events.into_iter().map(|event| Ok(agui_sse(&event))));
    let translated_stream = stream::unfold(stream_state, |mut state| async move {
        loop {
            if let Some(event) = state.queue.pop_front() {
                if state.finished && state.queue.is_empty() {
                    let done_state = state;
                    return Some((Ok(agui_sse(&event)), done_state));
                }
                return Some((Ok(agui_sse(&event)), state));
            }

            if state.finished {
                return None;
            }

            let Some(event) = state.subscription.recv().await else {
                state
                    .queue
                    .push_back(AgUiEvent::RunError(AgUiRunErrorEvent {
                        base: agui_base_event(),
                        message: "Event stream closed before the run finished".to_string(),
                        code: Some("stream_closed".to_string()),
                    }));
                state.finished = true;
                continue;
            };

            if event.session_id.uuid() != state.session_id {
                continue;
            }

            if event
                .context
                .input_message_id
                .as_ref()
                .map(|id| id.to_string())
                .as_deref()
                != Some(state.input_message_id.as_str())
            {
                continue;
            }

            translate_event(&mut state, &event);
        }
    });

    let stream = initial_stream.chain(translated_stream).map(move |event| {
        let _guard = &sse_guard;
        event
    });

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keepalive"),
    ))
}

struct SessionResolution {
    session: everruns_core::Session,
    is_new: bool,
}

async fn find_or_create_session(
    state: &AgUiState,
    app: &App,
    routing_tags: &[String],
    thread_id: &str,
    req: &AgUiRunAgentInput,
) -> anyhow::Result<SessionResolution> {
    let org_row = state
        .db
        .get_organization(app.org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Organization not found for app"))?;
    let org_public_id = org_row.public_id;

    match state
        .db
        .find_session_by_tags(app.org_id, routing_tags)
        .await?
    {
        Some(row) => Ok(SessionResolution {
            session: SessionService::row_to_session(row, &org_public_id),
            is_new: false,
        }),
        None => {
            let title = format!("AG-UI thread {}", thread_id);
            let session = state
                .session_service
                .create(
                    &Caller::internal(app.org_id),
                    app.harness_id.uuid(),
                    Some(app.agent_id.uuid()),
                    Some(app.agent_id),
                    CreateSessionRequest {
                        harness_id: Some(app.harness_id),
                        harness_name: None,
                        agent_id: Some(app.agent_id),
                        agent_identity_id: app.agent_identity_id,
                        title: Some(title),
                        locale: None,
                        tags: routing_tags.to_vec(),
                        model_id: None,
                        capabilities: vec![],
                        tools: vec![],
                        system_prompt: None,
                        initial_files: vec![],
                        hints: None,
                        network_access: None,
                        max_iterations: None,
                    },
                )
                .await?;

            tracing::info!(
                app_id = %app.public_id,
                session_id = %session.id,
                message_count = req.messages.len(),
                "Created AG-UI app session"
            );

            Ok(SessionResolution {
                session,
                is_new: true,
            })
        }
    }
}

async fn seed_history(
    state: &AgUiState,
    session_id: Uuid,
    messages: &[AgUiMessage],
) -> anyhow::Result<()> {
    if messages.is_empty() {
        return Ok(());
    }

    let retriever = DbMessageRetriever::new(state.db.clone());
    for message in messages {
        if let Some(stored) = to_stored_history_message(message) {
            retriever.add(session_id, stored).await?;
        }
    }
    Ok(())
}

fn to_stored_history_message(message: &AgUiMessage) -> Option<StoredInputMessage> {
    let (role, content, name) = match message {
        AgUiMessage::User { content, name, .. } => {
            (MessageRole::User, content.clone(), name.clone())
        }
        AgUiMessage::Assistant {
            content,
            name,
            tool_calls,
            ..
        } => (
            MessageRole::Agent,
            content
                .clone()
                .or_else(|| {
                    tool_calls
                        .as_ref()
                        .map(|calls| format_agui_tool_calls(calls))
                })
                .unwrap_or_default(),
            name.clone(),
        ),
        AgUiMessage::Tool {
            content,
            error,
            tool_call_id,
            ..
        } => (
            MessageRole::Agent,
            match error {
                Some(error) => format!("[Tool {} error: {}]\n{}", &**tool_call_id, error, content),
                None => format!("[Tool {} result]\n{}", &**tool_call_id, content),
            },
            None,
        ),
        AgUiMessage::System { content, name, .. }
        | AgUiMessage::Developer { content, name, .. } => {
            (MessageRole::System, content.clone(), name.clone())
        }
    };

    let mut stored = StoredInputMessage {
        role,
        content: vec![ContentPart::text(content)],
        controls: None,
        metadata: None,
        tags: vec![],
    };
    if let Some(name) = name {
        stored.metadata = Some(
            [("ag_ui_name".to_string(), Value::String(name))]
                .into_iter()
                .collect(),
        );
    }
    Some(stored)
}

fn build_external_actor(name: Option<&String>) -> Option<ExternalActor> {
    name.map(|name| ExternalActor {
        actor_id: name.clone(),
        actor_name: Some(name.clone()),
        source: "ag_ui".to_string(),
        metadata: None,
    })
}

fn to_ag_ui_message(message: &crate::api::messages::Message) -> AgUiMessage {
    let id = AgUiMessageId::from(message.id.uuid());
    let content = message
        .content
        .iter()
        .map(content_part_to_string)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    match message.role {
        ApiMessageRole::User => AgUiMessage::User {
            id,
            content,
            name: None,
        },
        ApiMessageRole::Agent => AgUiMessage::Assistant {
            id,
            content: (!content.is_empty()).then_some(content),
            name: None,
            tool_calls: None,
        },
    }
}

fn content_part_to_string(part: &ContentPart) -> String {
    match part {
        ContentPart::Text(text) => text.text.clone(),
        ContentPart::Image(image) => image.url.clone().unwrap_or_else(|| "[Image]".to_string()),
        ContentPart::ImageFile(image) => format!(
            "[Image file: {}]",
            image.filename.as_deref().unwrap_or("unnamed")
        ),
        ContentPart::ToolCall(tool_call) => format!(
            "[Tool call: {} {}]",
            tool_call.name,
            serde_json::to_string(&tool_call.arguments).unwrap_or_else(|_| "{}".to_string())
        ),
        ContentPart::ToolResult(tool_result) => format!(
            "[Tool result {}: {}]",
            tool_result.tool_call_id,
            tool_result
                .result
                .clone()
                .unwrap_or_else(|| Value::String(tool_result.error.clone().unwrap_or_default()))
        ),
    }
}

fn ensure_assistant_message_id(
    state: &mut AgUiStreamState,
    event: &everruns_core::Event,
) -> AgUiMessageId {
    state
        .assistant_message_id
        .get_or_insert_with(|| {
            event
                .context
                .turn_id
                .as_ref()
                .map(|id| AgUiMessageId::from(id.uuid()))
                .unwrap_or_else(AgUiMessageId::random)
        })
        .clone()
}

struct AgUiStreamState {
    subscription: Box<crate::event_delivery::EventSubscription>,
    session_id: Uuid,
    input_message_id: String,
    thread_id: AgUiThreadId,
    run_id: AgUiRunId,
    queue: VecDeque<AgUiEvent>,
    assistant_message_id: Option<AgUiMessageId>,
    assistant_content_started: bool,
    assistant_emitted_delta: bool,
    thinking_started: bool,
    thinking_text_started: bool,
    tool_call_ids: HashMap<String, AgUiToolCallId>,
    finished: bool,
}

fn translate_event(state: &mut AgUiStreamState, event: &everruns_core::Event) {
    match event.event_type.as_str() {
        "output.message.delta" => {
            if let Ok(data) = parse_event_data::<OutputMessageDeltaData>(event) {
                let message_id = ensure_assistant_message_id(state, event);
                if !state.assistant_content_started {
                    state
                        .queue
                        .push_back(AgUiEvent::TextMessageStart(AgUiTextMessageStartEvent {
                            base: agui_base_event(),
                            message_id: message_id.clone(),
                            role: AgUiRole::Assistant,
                        }));
                    state.assistant_content_started = true;
                }
                state.queue.push_back(AgUiEvent::TextMessageContent(
                    AgUiTextMessageContentEvent::new(message_id, data.delta).unwrap(),
                ));
                state.assistant_emitted_delta = true;
            }
        }
        "output.message.completed" => {
            if let Ok(data) = parse_event_data::<OutputMessageCompletedData>(event) {
                let message_id = ensure_assistant_message_id(state, event);
                if !state.assistant_content_started {
                    state
                        .queue
                        .push_back(AgUiEvent::TextMessageStart(AgUiTextMessageStartEvent {
                            base: agui_base_event(),
                            message_id: message_id.clone(),
                            role: AgUiRole::Assistant,
                        }));
                }
                if !state.assistant_emitted_delta {
                    let text = data.message.content_to_llm_string();
                    if !text.is_empty() {
                        state.queue.push_back(AgUiEvent::TextMessageContent(
                            AgUiTextMessageContentEvent::new(message_id.clone(), text).unwrap(),
                        ));
                    }
                }
                state
                    .queue
                    .push_back(AgUiEvent::TextMessageEnd(AgUiTextMessageEndEvent {
                        base: agui_base_event(),
                        message_id: message_id.clone(),
                    }));
                state
                    .queue
                    .push_back(AgUiEvent::RunFinished(AgUiRunFinishedEvent {
                        base: agui_base_event(),
                        thread_id: state.thread_id.clone(),
                        run_id: state.run_id.clone(),
                        result: None,
                    }));
                state.assistant_message_id = Some(message_id);
                state.assistant_content_started = false;
                state.assistant_emitted_delta = false;
                state.finished = true;
            }
        }
        "reason.thinking.started"
            if parse_event_data::<ReasonThinkingStartedData>(event).is_ok() =>
        {
            state
                .queue
                .push_back(AgUiEvent::ThinkingStart(AgUiThinkingStartEvent {
                    base: agui_base_event(),
                    title: None,
                }));
            state.thinking_started = true;
            state.thinking_text_started = false;
        }
        "reason.thinking.delta" => {
            if let Ok(data) = parse_event_data::<ReasonThinkingDeltaData>(event) {
                if !state.thinking_text_started {
                    state.queue.push_back(AgUiEvent::ThinkingTextMessageStart(
                        AgUiThinkingTextMessageStartEvent {
                            base: agui_base_event(),
                        },
                    ));
                    state.thinking_text_started = true;
                }
                state.queue.push_back(AgUiEvent::ThinkingTextMessageContent(
                    AgUiThinkingTextMessageContentEvent {
                        base: agui_base_event(),
                        delta: data.delta,
                    },
                ));
            }
        }
        "reason.thinking.completed"
            if parse_event_data::<ReasonThinkingCompletedData>(event).is_ok() =>
        {
            if state.thinking_text_started {
                state.queue.push_back(AgUiEvent::ThinkingTextMessageEnd(
                    AgUiThinkingTextMessageEndEvent {
                        base: agui_base_event(),
                    },
                ));
            }
            if state.thinking_started {
                state
                    .queue
                    .push_back(AgUiEvent::ThinkingEnd(AgUiThinkingEndEvent {
                        base: agui_base_event(),
                    }));
            }
            state.thinking_started = false;
            state.thinking_text_started = false;
        }
        "tool.started" => {
            if let Ok(data) = parse_event_data::<ToolStartedData>(event) {
                let message_id = ensure_assistant_message_id(state, event);
                let tool_call_id = state
                    .tool_call_ids
                    .entry(data.tool_call.id.clone())
                    .or_insert_with(AgUiToolCallId::random)
                    .clone();
                state
                    .queue
                    .push_back(AgUiEvent::ToolCallStart(AgUiToolCallStartEvent {
                        base: agui_base_event(),
                        tool_call_id: tool_call_id.clone(),
                        tool_call_name: data.tool_call.name,
                        parent_message_id: Some(message_id),
                    }));
                state
                    .queue
                    .push_back(AgUiEvent::ToolCallArgs(AgUiToolCallArgsEvent {
                        base: agui_base_event(),
                        tool_call_id: tool_call_id.clone(),
                        delta: serde_json::to_string(&data.tool_call.arguments)
                            .unwrap_or_else(|_| "{}".to_string()),
                    }));
                state
                    .queue
                    .push_back(AgUiEvent::ToolCallEnd(AgUiToolCallEndEvent {
                        base: agui_base_event(),
                        tool_call_id,
                    }));
            }
        }
        "tool.completed" => {
            if let Ok(data) = parse_event_data::<ToolCompletedData>(event) {
                let message_id = ensure_assistant_message_id(state, event);
                let tool_call_id = state
                    .tool_call_ids
                    .remove(&data.tool_call_id)
                    .unwrap_or_else(AgUiToolCallId::random);
                state
                    .queue
                    .push_back(AgUiEvent::ToolCallResult(AgUiToolCallResultEvent {
                        base: agui_base_event(),
                        message_id,
                        tool_call_id,
                        content: data
                            .result
                            .map(|parts| {
                                parts
                                    .iter()
                                    .map(content_part_to_string)
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_else(|| data.error.unwrap_or(data.status)),
                        role: AgUiRole::Tool,
                    }));
            }
        }
        "turn.completed" | "session.idled" if !state.finished => {
            state
                .queue
                .push_back(AgUiEvent::RunFinished(AgUiRunFinishedEvent {
                    base: agui_base_event(),
                    thread_id: state.thread_id.clone(),
                    run_id: state.run_id.clone(),
                    result: None,
                }));
            state.finished = true;
        }
        "turn.failed" | "turn.cancelled" => {
            let event_data = serde_json::to_value(&event.data).unwrap_or_default();
            let message = event_data
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| event_data.get("message").and_then(Value::as_str))
                .unwrap_or("Run failed");
            if !state.finished {
                state
                    .queue
                    .push_back(AgUiEvent::RunError(AgUiRunErrorEvent {
                        base: agui_base_event(),
                        message: message.to_string(),
                        code: None,
                    }));
                state.finished = true;
            }
        }
        _ => {}
    }
}

fn agui_base_event() -> AgUiBaseEvent {
    AgUiBaseEvent {
        timestamp: None,
        raw_event: None,
    }
}

fn agui_sse(event: &AgUiEvent) -> SseEvent {
    SseEvent::default().data(serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string()))
}

fn format_agui_tool_calls(tool_calls: &[AgUiToolCall]) -> String {
    tool_calls
        .iter()
        .map(|tool_call| {
            format!(
                "[Tool call: {} {}]",
                tool_call.function.name, tool_call.function.arguments
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_event_data<T: for<'de> Deserialize<'de>>(
    event: &everruns_core::Event,
) -> Result<T, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(&event.data)?)
}

fn internal_error(err: anyhow::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %err, "AG-UI route failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal server error".to_string(),
        }),
    )
}

fn bad_request(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
}

fn forbidden(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "App not found".to_string(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_ui_core::event::EventType as AgUiEventType;
    use everruns_core::{
        Event, EventContext, Message, MessageId, OutputMessageCompletedData,
        OutputMessageDeltaData, SessionId, ToolCall, ToolCompletedData, ToolStartedData, TurnId,
    };

    async fn test_stream_state() -> AgUiStreamState {
        let session_id = SessionId::new();
        let delivery = crate::event_delivery::EventDelivery::in_memory();
        let subscription = delivery.subscribe(session_id.uuid()).await.unwrap();
        AgUiStreamState {
            subscription: Box::new(subscription),
            session_id: session_id.uuid(),
            input_message_id: MessageId::new().to_string(),
            thread_id: AgUiThreadId::random(),
            run_id: AgUiRunId::random(),
            queue: VecDeque::new(),
            assistant_message_id: None,
            assistant_content_started: false,
            assistant_emitted_delta: false,
            thinking_started: false,
            thinking_text_started: false,
            tool_call_ids: HashMap::new(),
            finished: false,
        }
    }

    #[tokio::test]
    async fn test_translate_output_completion_to_ag_ui_run_finished() {
        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let event = Event::new(
            SessionId::from_uuid(state.session_id),
            EventContext::turn(turn_id, input_message_id),
            OutputMessageCompletedData::new(Message::assistant("Hello from AG-UI")),
        );

        translate_event(&mut state, &event);

        let event_types: Vec<AgUiEventType> =
            state.queue.iter().map(AgUiEvent::event_type).collect();
        assert_eq!(
            event_types,
            vec![
                AgUiEventType::TextMessageStart,
                AgUiEventType::TextMessageContent,
                AgUiEventType::TextMessageEnd,
                AgUiEventType::RunFinished,
            ]
        );
        assert!(state.finished);
    }

    #[tokio::test]
    async fn test_translate_streaming_delta_does_not_duplicate_final_content() {
        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let context = EventContext::turn(turn_id, input_message_id);
        let session_id = SessionId::from_uuid(state.session_id);

        let delta_event = Event::new(
            session_id,
            context.clone(),
            OutputMessageDeltaData {
                turn_id,
                delta: "Hello".to_string(),
                accumulated: "Hello".to_string(),
            },
        );
        translate_event(&mut state, &delta_event);

        let completed_event = Event::new(
            session_id,
            context,
            OutputMessageCompletedData::new(Message::assistant("Hello from AG-UI")),
        );
        translate_event(&mut state, &completed_event);

        let event_types: Vec<AgUiEventType> =
            state.queue.iter().map(AgUiEvent::event_type).collect();
        assert_eq!(
            event_types,
            vec![
                AgUiEventType::TextMessageStart,
                AgUiEventType::TextMessageContent,
                AgUiEventType::TextMessageEnd,
                AgUiEventType::RunFinished,
            ]
        );
        let content_events: Vec<&AgUiEvent> = state
            .queue
            .iter()
            .filter(|event| matches!(event, AgUiEvent::TextMessageContent(_)))
            .collect();
        assert_eq!(content_events.len(), 1);
        match content_events[0] {
            AgUiEvent::TextMessageContent(event) => assert_eq!(event.delta, "Hello"),
            _ => unreachable!(),
        }
        assert!(state.finished);
    }

    #[tokio::test]
    async fn test_tool_first_run_reuses_assistant_message_id() {
        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let context = EventContext::turn(turn_id, input_message_id);
        let session_id = SessionId::from_uuid(state.session_id);

        let tool_started = Event::new(
            session_id,
            context.clone(),
            ToolStartedData {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "web_search".to_string(),
                    arguments: serde_json::json!({"q": "hello"}),
                },
                display_name: None,
                narration: None,
            },
        );
        translate_event(&mut state, &tool_started);

        let tool_completed = Event::new(
            session_id,
            context.clone(),
            ToolCompletedData {
                tool_call_id: "call_1".to_string(),
                tool_name: "web_search".to_string(),
                display_name: None,
                success: true,
                status: "success".to_string(),
                result: Some(vec![ContentPart::text("result")]),
                error: None,
                duration_ms: Some(10),
                narration: None,
            },
        );
        translate_event(&mut state, &tool_completed);

        let output_completed = Event::new(
            session_id,
            context,
            OutputMessageCompletedData::new(Message::assistant("Hello after tool")),
        );
        translate_event(&mut state, &output_completed);

        let expected_message_id = AgUiMessageId::from(turn_id.uuid());

        let tool_call_id = match &state.queue[0] {
            AgUiEvent::ToolCallStart(event) => {
                assert_eq!(event.parent_message_id, Some(expected_message_id.clone()));
                event.tool_call_id.clone()
            }
            _ => panic!("expected tool start event"),
        };
        match &state.queue[1] {
            AgUiEvent::ToolCallArgs(_) => {}
            _ => panic!("expected tool args event"),
        }
        match &state.queue[2] {
            AgUiEvent::ToolCallEnd(event) => assert_eq!(event.tool_call_id, tool_call_id),
            _ => panic!("expected tool end event"),
        }
        match &state.queue[3] {
            AgUiEvent::ToolCallResult(event) => {
                assert_eq!(event.message_id, expected_message_id.clone());
                assert_eq!(event.tool_call_id, tool_call_id);
            }
            _ => panic!("expected tool result event"),
        }
        match &state.queue[4] {
            AgUiEvent::TextMessageStart(event) => assert_eq!(event.message_id, expected_message_id),
            _ => panic!("expected text start event"),
        }
    }
}
