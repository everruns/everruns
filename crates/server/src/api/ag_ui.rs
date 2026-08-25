// AG-UI app channel — anonymous, app-scoped streaming endpoint
//
// Design Decision: AG-UI ingress is app-scoped (POST /v1/apps/{app_id}/ag-ui)
// so the App controls the agent, harness, identity, and publication lifecycle.
//
// Design Decision: The endpoint is public and app-scoped. Requests are accepted
// without user API auth when the app is published and an enabled AG-UI channel
// is present, but a channel may require its own shared bearer token.
//
// Design Decision: The AG-UI stream is translated from Everruns session events
// instead of bypassing the durable runtime. This keeps app-channel behavior
// aligned with normal sessions and preserves streaming parity.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use crate::kernel_imports::{
    Caller, ContentPart, ExternalActor, MessageRole, everruns_provider::typed_id::MessageId,
};
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
};
use ag_ui_core::types::{
    ids::{MessageId as AgUiMessageId, RunId as AgUiRunId, ThreadId as AgUiThreadId},
    input::RunAgentInput as AgUiRunAgentInput,
    message::{Message as AgUiMessage, Role as AgUiRole},
    tool::ToolCall as AgUiToolCall,
};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, FromRequest, Path, Request, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::post,
};
use axum_extra::extract::Multipart;
use everruns_core::events::{
    OutputMessageCompletedData, OutputMessageDeltaData, ReasonItemData,
    ReasonThinkingCompletedData, ReasonThinkingDeltaData, ReasonThinkingStartedData,
    ToolCompletedData, ToolStartedData, TurnFailedData,
};
use everruns_core::message_retriever::InputMessage as StoredInputMessage;
use everruns_platform::{AgUiChannelConfig, AgUiToolVisibility, App, AppStatus};
use everruns_provider::execution_phase::ExecutionPhase;
use everruns_provider::typed_id::ImageId;
#[cfg(test)]
use everruns_provider::user_facing_error::codes as user_facing_error_codes;
use futures::{
    StreamExt,
    stream::{self, Stream},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::app_endpoint_auth::{
    AppEndpointAuthError, AppEndpointAuthVerifier, LegacyEndpointAuth,
};
use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::api::common::ErrorResponse;
use crate::api::images::{
    ImageUploadResponse, generate_thumbnail, is_valid_content_type, validate_image_bytes,
};
use crate::api::messages::{
    CreateMessageRequest, InputContentPart, InputMessage, MessageRole as ApiMessageRole,
};
use crate::api::public::PublicError;
use crate::api::sessions::CreateSessionRequest;
use crate::api::sse::SseConnectionTracker;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::execution_metadata;
use crate::middleware::RequestId;
use crate::security::constant_time_eq;
use crate::services::EventService;
use crate::storage::{
    DbMessageRetriever, EncryptionService, StorageBackend, models::CreateImageRow,
};

const AG_UI_TOKEN_HEADER: &str = "x-everruns-ag-ui-token";
const MAX_AG_UI_IMAGES_PER_RUN: usize = 10;
const MAX_PUBLIC_AG_UI_IMAGE_SIZE: usize = 10 * 1024 * 1024;

#[derive(Clone)]
pub struct AgUiState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    pub sse_tracker: Arc<SseConnectionTracker>,
    pub rate_limiter: ChannelRateLimiter,
    pub auth_verifier: AppEndpointAuthVerifier,
}

impl AgUiState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_worker::AgentRunner>,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
        sse_tracker: Arc<SseConnectionTracker>,
        rate_limiter: ChannelRateLimiter,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner,
                notifications_enabled,
                event_delivery.clone(),
            )),
            event_service: Arc::new(EventService::new(db.clone(), event_delivery)),
            sse_tracker,
            rate_limiter,
            auth_verifier: AppEndpointAuthVerifier::new(),
            encryption,
            db,
        }
    }
}

pub fn routes(state: AgUiState) -> Router {
    Router::new()
        .route("/v1/apps/{app_id}/ag-ui", post(run_agent))
        .route(
            "/v1/apps/{app_id}/ag-ui/images",
            post(upload_image).layer(DefaultBodyLimit::max(
                MAX_PUBLIC_AG_UI_IMAGE_SIZE + 1024 * 1024,
            )),
        )
        .with_state(state)
}

struct AuthorizedAgUiRequest {
    app: App,
    channel_config: AgUiChannelConfig,
}

async fn authorize_ag_ui_request(
    state: &AgUiState,
    app_id: &str,
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
) -> Result<AuthorizedAgUiRequest, Response> {
    let app = crate::domains::apps::queries::get_by_public_id_unscoped(
        &state.db,
        state.encryption.as_ref(),
        app_id,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;

    // THREAT[TM-AUTHZ-005]: Anonymous AG-UI requests must not reach draft or
    // private app configurations.
    // Mitigation: Require a published app, an enabled AG-UI channel, and
    // `anonymous=true` before accepting unauthenticated traffic.
    //
    // THREAT[TM-TENANT-002]: An unauthenticated caller must not be able to tell
    // "app does not exist" apart from "app exists but is not published / has no
    // AG-UI channel / is misconfigured". Every such case collapses to a single
    // generic 404 (matching the FCP channel in `api/fcp.rs`); the real reason is
    // logged server-side only.
    if app.status != AppStatus::Published {
        tracing::debug!(app_id = %app.public_id, status = ?app.status, "AG-UI request rejected: app not published");
        return Err(not_found());
    }

    let Some(channel) = app.ag_ui_channel() else {
        tracing::debug!(app_id = %app.public_id, "AG-UI request rejected: no enabled AG-UI channel");
        return Err(not_found());
    };
    let Some(channel_config) = channel.ag_ui_config() else {
        tracing::error!(app_id = %app.public_id, "AG-UI channel config did not deserialize");
        return Err(not_found());
    };
    if let Some(auth) = channel_config.auth.as_ref() {
        state
            .auth_verifier
            .verify(
                auth,
                headers,
                LegacyEndpointAuth {
                    shared_secret: channel_config.token.as_deref(),
                    api_key: None,
                },
            )
            .await
            .map_err(ag_ui_auth_error_response)?;
    } else {
        if !channel_config.anonymous {
            // No auth provider configured and anonymous access disabled: the
            // channel is not reachable. Collapse to a generic 404 rather than a
            // 403 so callers cannot confirm the app exists (TM-TENANT-002).
            tracing::debug!(app_id = %app.public_id, "AG-UI request rejected: anonymous access disabled with no auth provider");
            return Err(not_found());
        }
        if let Some(expected_token) = channel_config.token.as_deref()
            && !expected_token.is_empty()
        {
            let provided_token = extract_ag_ui_token(headers).ok_or_else(unauthorized)?;
            if !constant_time_eq(provided_token.as_bytes(), expected_token.as_bytes()) {
                return Err(unauthorized());
            }
        }
    }

    // THREAT[TM-DOS-010]: Anonymous AG-UI traffic must respect a configurable
    // per-app, per-IP cap in addition to the global API limit. App owners
    // tune `rate_limit_per_minute` based on expected client traffic.
    if let Some(limit) = channel_config.rate_limit_per_minute
        && limit > 0
    {
        let client_ip = extract_client_ip_from_parts(peer_addr, headers);
        if state
            .rate_limiter
            .check(&app.public_id.to_string(), client_ip, limit)
            .await
            .is_err()
        {
            return Err(too_many_requests("AG-UI rate limit exceeded for this app"));
        }
    }

    Ok(AuthorizedAgUiRequest {
        app,
        channel_config,
    })
}

async fn upload_image(
    State(state): State<AgUiState>,
    Path(app_id): Path<String>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<ImageUploadResponse>), Response> {
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let AuthorizedAgUiRequest { app, .. } =
        authorize_ag_ui_request(&state, &app_id, &headers, peer_addr).await?;

    // THREAT[TM-DOS-010]: Public AG-UI image uploads are anonymous ingress.
    // Mitigation: reuse the per-app public AG-UI gate/rate limit above, cap the
    // multipart body at the router, and validate MIME type plus byte length
    // before writing image bytes to storage.
    let mut file_data: Option<(String, String, Vec<u8>)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| bad_request("invalid_request"))?
    {
        if field.name().unwrap_or("") != "file" {
            continue;
        }

        let filename = field.file_name().unwrap_or("upload").to_string();
        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        if !is_valid_content_type(&content_type) {
            return Err(bad_request("invalid_request"));
        }

        let data = field
            .bytes()
            .await
            .map_err(|_| bad_request("invalid_request"))?;
        if data.len() > MAX_PUBLIC_AG_UI_IMAGE_SIZE {
            return Err(bad_request("invalid_request"));
        }

        if !validate_image_bytes(&data, &content_type) {
            return Err(bad_request("invalid_request"));
        }

        file_data = Some((filename, content_type, data.to_vec()));
        break;
    }

    let (filename, content_type, data) =
        file_data.ok_or_else(|| bad_request("No 'file' field in request"))?;
    let (thumbnail_data, thumbnail_content_type) = generate_thumbnail(&data, &content_type)
        .map(|(data, content_type)| (Some(data), Some(content_type)))
        .unwrap_or((None, None));
    let size_bytes = data.len() as i64;
    let row = state
        .db
        .create_image(
            app.org_id,
            CreateImageRow {
                org_id: app.org_id,
                filename,
                content_type,
                size_bytes,
                data,
                thumbnail_data,
                thumbnail_content_type,
                metadata: ag_ui_image_metadata(&app),
            },
        )
        .await
        .map_err(internal_error)?;

    Ok((
        StatusCode::CREATED,
        Json(ImageUploadResponse {
            id: row.id,
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
        }),
    ))
}

async fn run_agent(
    State(state): State<AgUiState>,
    Path(app_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    request: Request,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, Response> {
    let request_id = req_id.map(|Extension(r)| r.0);
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let AuthorizedAgUiRequest {
        app,
        channel_config,
    } = authorize_ag_ui_request(&state, &app_id, &headers, peer_addr).await?;

    run_app_agent_stream(
        state,
        app,
        channel_config,
        "ag_ui",
        Vec::new(),
        request,
        request_id,
    )
    .await
}

/// Shared AG-UI ingress + streaming core, reused by both the AG-UI channel and
/// the Public Chat channel. `tag_prefix` scopes the session routing tags
/// (`{prefix}:app:{id}` / `{prefix}:thread:{id}`) so reusing a thread ID across
/// channels or apps can never merge tenants or sessions (TM-TENANT-009).
/// Callers with stronger visitor ownership requirements can pass additional
/// routing tags; they become part of the session lookup identity.
pub(crate) async fn run_app_agent_stream(
    state: AgUiState,
    app: App,
    channel_config: AgUiChannelConfig,
    tag_prefix: &str,
    extra_routing_tags: Vec<String>,
    request: Request,
    request_id: Option<String>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, Response> {
    // EVE-415: monotonic anchor for the ingress phase. Used to attribute
    // the gap between accepted request and `ReasonAtom: starting LLM call`
    // (~600-700ms baseline) to the AG-UI handler vs. the durable runtime.
    // The handler emits a single structured `ag_ui.ingress_complete` log just
    // before returning the SSE stream so downstream profilers can isolate the
    // ingress portion without grepping for adjacent timestamps.
    let ingress_start = Instant::now();

    let Json(req): Json<AgUiRunAgentInput> = Json::from_request(request, &state)
        .await
        .map_err(IntoResponse::into_response)?;

    // THREAT[TM-LLM-020]: Anonymous AG-UI clients must not be able to forge
    // privileged message roles (system/developer/tool) into the LLM context
    // that the server builds from the request body.
    // Mitigation: Reject any non-{user,assistant} role at the runtime trust
    // boundary, and reject duplicate message IDs. The error is a generic
    // `invalid_request` so we don't echo the offending role back.
    validate_input_messages(&req.messages).map_err(|err| *err)?;

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

    // THREAT[TM-TENANT-009]: Reusing the same thread ID across apps/channels must
    // not merge tenants or app sessions.
    // Mitigation: Scope the session lookup tags by channel prefix, app public
    // ID, and thread ID so thread collisions stay isolated per app and channel.
    let mut routing_tags = vec![
        format!("{tag_prefix}:app:{}", app.public_id),
        format!("{tag_prefix}:thread:{}", thread_tag),
    ];
    routing_tags.extend(extra_routing_tags);
    let session = find_or_create_session(
        &state,
        &app,
        &channel_config,
        &routing_tags,
        &thread_tag,
        &req,
    )
    .await
    .map_err(SessionError::into_response)?;
    // EVE-415: snapshot before SSE guard / history seed / subscribe so the
    // timing field measures only the session lookup-or-create work, matching
    // the contract documented in `knowledge/operations/load-testing.md`.
    let session_resolved_ms = ingress_start.elapsed().as_millis();

    // THREAT[TM-DOS-010]: Anonymous AG-UI streams must still respect server-wide
    // SSE connection limits.
    // Mitigation: Reuse the shared SSE tracker for per-org and per-session limits
    // before opening the stream.
    let sse_guard = state
        .sse_tracker
        .try_acquire(app.org_id, session.session.id.uuid())
        .map_err(|rejection| too_many_requests(&rejection.to_string()))?;

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

    let trigger_image_parts = ag_ui_image_content_parts(&state, &app, &req.forwarded_props)
        .await
        .map_err(|err| *err)?;
    let mut trigger_parts = vec![InputContentPart::text(trigger_content)];
    trigger_parts.extend(trigger_image_parts);

    let message = state
        .message_service
        .create(
            CreateMessageContext {
                org_id: app.org_id,
                user_id: None,
                harness_id: app.harness_id.uuid(),
                agent_id: app.agent_id.map(|agent_id| agent_id.uuid()),
                session_id: session.session.id.uuid(),
                event_metadata: Some(execution_metadata::app_message_metadata(
                    app.public_id,
                    app.owner_principal_id,
                    app.agent_identity_id,
                )),
                request_id,
            },
            CreateMessageRequest {
                message: InputMessage {
                    role: ApiMessageRole::User,
                    content: trigger_parts,
                },
                addressed_participant_id: None,
                controls: None,
                metadata: Some(ag_ui_message_metadata(&app, thread_tag, run_tag)),
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

    // EVE-415: structured AG-UI ingress timing. `ag_ui_handler_ms` is the
    // wall-clock cost of every server-side step the handler performs before
    // returning the SSE stream: validation, app/channel resolution, session
    // resolution, history seed, event subscription, and the synchronous part
    // of message creation. The durable turn workflow itself is scheduled by
    // `MessageService::create` via a detached `tokio::spawn`, so this log
    // intentionally does not claim the workflow has started — only that the
    // handler has finished its synchronous work and is about to hand off to
    // the SSE stream. Pair with the worker's `Turn workflow started` and
    // `ReasonAtom: starting LLM call` log lines (correlated by `session_id`)
    // to compute the full pre-LLM budget without joining unstructured logs.
    let ag_ui_handler_ms = ingress_start.elapsed().as_millis();
    tracing::info!(
        phase = "ag_ui.ingress_complete",
        app_id = %app.public_id,
        session_id = %session.session.id,
        message_id = %message.id,
        is_new_session = session.is_new,
        session_resolved_ms = session_resolved_ms as u64,
        ag_ui_handler_ms = ag_ui_handler_ms as u64,
        "AG-UI ingress complete; durable turn workflow start scheduled"
    );

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
        tool_visibility: channel_config.tool_visibility,
        generic_tool_text: channel_config.generic_tool_text.clone(),
        reasoning_summary_visible: channel_config.reasoning_summary_visible,
        active_tool_activity_count: 0,
        public_tool_activity_started: false,
        public_tool_activity_opened_thinking: false,
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
                    .push_back(public_run_error_event(PublicError::fallback()));
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

fn ag_ui_message_metadata(
    app: &App,
    thread_tag: String,
    run_tag: String,
) -> HashMap<String, Value> {
    [
        (
            "_app_id".to_string(),
            Value::String(app.public_id.to_string()),
        ),
        ("ag_ui_thread_id".to_string(), Value::String(thread_tag)),
        ("ag_ui_run_id".to_string(), Value::String(run_tag)),
    ]
    .into_iter()
    .collect()
}

fn ag_ui_image_metadata(app: &App) -> Value {
    serde_json::json!({
        "_app_id": app.public_id.to_string(),
        "source": "ag_ui",
    })
}

fn is_ag_ui_app_image(metadata: &Value, app_public_id: &str) -> bool {
    metadata
        .get("_app_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id == app_public_id)
        && metadata.get("source").and_then(Value::as_str) == Some("ag_ui")
}

async fn ag_ui_image_content_parts(
    state: &AgUiState,
    app: &App,
    forwarded_props: &Value,
) -> Result<Vec<InputContentPart>, Box<Response>> {
    let image_ids = forwarded_ag_ui_image_ids(forwarded_props)?;
    if image_ids.is_empty() {
        return Ok(vec![]);
    }

    // THREAT[TM-TENANT-009]: Public clients must not attach arbitrary org
    // images to an app run. Mitigation: resolve by org and require AG-UI image
    // metadata for the same public app before constructing ImageFile parts.
    let app_public_id = app.public_id.to_string();
    let mut parts = Vec::with_capacity(image_ids.len());
    for image_id in image_ids {
        let row = state
            .db
            .get_image(app.org_id, image_id.uuid())
            .await
            .map_err(|err| Box::new(internal_error(err)))?
            .ok_or_else(|| Box::new(bad_request("invalid_request")))?;
        if !is_ag_ui_app_image(&row.metadata, &app_public_id) {
            return Err(Box::new(bad_request("invalid_request")));
        }
        parts.push(InputContentPart::ImageFile(
            everruns_core::ImageFileContentPart::with_filename(row.id, row.filename),
        ));
    }
    Ok(parts)
}

fn forwarded_ag_ui_image_ids(forwarded_props: &Value) -> Result<Vec<ImageId>, Box<Response>> {
    let Some(raw_images) = forwarded_props
        .get("imageIds")
        .or_else(|| forwarded_props.get("image_ids"))
        .or_else(|| forwarded_props.get("agUiImageIds"))
        .or_else(|| forwarded_props.get("ag_ui_image_ids"))
    else {
        return Ok(vec![]);
    };

    let images = raw_images
        .as_array()
        .ok_or_else(|| Box::new(bad_request("invalid_request")))?;
    if images.len() > MAX_AG_UI_IMAGES_PER_RUN {
        return Err(Box::new(bad_request("invalid_request")));
    }

    images
        .iter()
        .map(|value| {
            let id = value
                .as_str()
                .ok_or_else(|| Box::new(bad_request("invalid_request")))?;
            id.parse::<ImageId>()
                .map_err(|_| Box::new(bad_request("invalid_request")))
        })
        .collect()
}

struct SessionResolution {
    session: everruns_platform::Session,
    is_new: bool,
}

/// Errors that can come back from `find_or_create_session`. Distinguishes
/// expected client failures (expired threads) from internal errors so the
/// caller can return the right HTTP status.
enum SessionError {
    Expired { age_seconds: i64, max_seconds: u32 },
    Internal(anyhow::Error),
}

impl SessionError {
    fn into_response(self) -> Response {
        match self {
            SessionError::Expired {
                age_seconds,
                max_seconds,
            } => ErrorResponse::new(format!(
                        "AG-UI thread expired after {age_seconds}s (limit {max_seconds}s); start a new thread"
                    ))
                .into_response(StatusCode::GONE)
                .into_response(),
            SessionError::Internal(err) => internal_error(err),
        }
    }
}

impl From<anyhow::Error> for SessionError {
    fn from(err: anyhow::Error) -> Self {
        SessionError::Internal(err)
    }
}

async fn find_or_create_session(
    state: &AgUiState,
    app: &App,
    config: &AgUiChannelConfig,
    routing_tags: &[String],
    thread_id: &str,
    req: &AgUiRunAgentInput,
) -> Result<SessionResolution, SessionError> {
    let org_row = state
        .db
        .get_organization(app.org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Organization not found for app"))?;
    let org_public_id = org_row.public_id;

    match state
        .db
        .find_app_session_by_tags(app.org_id, app.internal_id, routing_tags)
        .await?
    {
        Some(row) => {
            // THREAT[TM-AUTHZ-005]: Public AG-UI threads must not be resumable
            // forever. After the configured expiration the thread_id can no
            // longer carry forward an existing conversation.
            if let Some(age) = expired_age_seconds(
                row.created_at,
                config.session_expiration_seconds,
                chrono::Utc::now(),
            ) {
                tracing::info!(
                    app_id = %app.public_id,
                    session_id = %row.id,
                    age_seconds = age,
                    max_seconds = config.session_expiration_seconds,
                    "Rejecting AG-UI resume on expired thread"
                );
                return Err(SessionError::Expired {
                    age_seconds: age,
                    max_seconds: config.session_expiration_seconds,
                });
            }

            let fallback = if row.harness_id.is_none() {
                Some(crate::org_init::base_harness_id(&state.db, app.org_id).await?)
            } else {
                None
            };
            Ok(SessionResolution {
                session: SessionService::row_to_session(row, &org_public_id, fallback),
                is_new: false,
            })
        }
        None => {
            let title = format!("AG-UI thread {}", thread_id);
            let session = state
                .session_service
                .create_from_app(
                    &Caller::internal(app.org_id),
                    app.harness_id.uuid(),
                    app.agent_id.map(|agent_id| agent_id.uuid()),
                    app.agent_id,
                    app.internal_id,
                    app.owner_principal_id,
                    app.resolved_owner_user_id,
                    everruns_platform::SessionSource::AgUi,
                    CreateSessionRequest {
                        source: None,
                        workspace_id: None,
                        harness_id: Some(app.harness_id),
                        harness_name: None,
                        agent_id: app.agent_id,
                        agent_name: None,
                        agent_identity_id: app.agent_identity_id,
                        title: Some(title),
                        goal: None,
                        locale: None,
                        tags: routing_tags.to_vec(),
                        model_id: None,
                        capabilities: vec![],
                        tools: vec![],
                        mcp_servers: Default::default(),
                        system_prompt: None,
                        initial_files: vec![],
                        hints: None,
                        network_access: None,
                        max_iterations: None,
                        parallel_tool_calls: None,
                        parent_session_id: None,
                        forked_from_session_id: None,
                        budget_root_session_id: None,
                        seed: everruns_core::SessionSeedMode::Fresh,
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
    let content = public_content_parts_to_string(&message.content);

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

fn public_content_parts_to_string(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(public_content_part_to_string)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn public_content_part_to_string(part: &ContentPart) -> Option<String> {
    match part {
        ContentPart::Text(text) => Some(text.text.clone()),
        ContentPart::Image(image) => {
            Some(image.url.clone().unwrap_or_else(|| "[Image]".to_string()))
        }
        ContentPart::ImageFile(image) => Some(format!(
            "[Image file: {}]",
            image.filename.as_deref().unwrap_or("unnamed")
        )),
        ContentPart::ToolCall(_) | ContentPart::ToolResult(_) => None,
        // Reasoning belongs to the reasoning channel and is projected there by
        // the reason.* handlers. Rendering it as assistant text would move
        // content across channels, which the projection contract forbids.
        ContentPart::Reasoning(_) => None,
    }
}

fn is_terminal_public_output_message(
    message: &everruns_core::Message,
    assistant_emitted_delta: bool,
) -> bool {
    if matches!(message.phase, Some(ExecutionPhase::Commentary)) {
        return false;
    }
    if message
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::ToolCall(_) | ContentPart::ToolResult(_)))
    {
        return false;
    }
    assistant_emitted_delta || !public_content_parts_to_string(&message.content).is_empty()
}

/// Projects the canonical streamed message id into AG-UI's message id space.
///
/// A new lifecycle id closes any still-open assistant text before switching
/// scopes. Normal streams close through `output.message.completed`; this guard
/// also keeps replay/reconnect projections correct when a terminal event is
/// missing.
fn ensure_assistant_message_id(
    state: &mut AgUiStreamState,
    streamed_message_id: MessageId,
) -> AgUiMessageId {
    let projected = AgUiMessageId::from(streamed_message_id.uuid());
    if state.assistant_message_id.as_ref() != Some(&projected) {
        close_assistant_text_without_finishing(state);
        state.assistant_message_id = Some(projected.clone());
    }
    projected
}

fn close_assistant_text_without_finishing(state: &mut AgUiStreamState) {
    // Commentary text can stream before tools. Close that AG-UI text message so
    // the later final-answer completion can emit its own public content without
    // ending the run early.
    if state.assistant_content_started
        && let Some(message_id) = state.assistant_message_id.clone()
    {
        state
            .queue
            .push_back(AgUiEvent::TextMessageEnd(AgUiTextMessageEndEvent {
                base: agui_base_event(),
                message_id,
            }));
    }
    state.assistant_message_id = None;
    state.assistant_content_started = false;
    state.assistant_emitted_delta = false;
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
    tool_visibility: AgUiToolVisibility,
    generic_tool_text: String,
    reasoning_summary_visible: bool,
    active_tool_activity_count: usize,
    public_tool_activity_started: bool,
    public_tool_activity_opened_thinking: bool,
    finished: bool,
}

impl AgUiStreamState {
    /// Returns the channel-configured generic tool text, trimmed and with a fallback to the
    /// platform default if the value is empty or whitespace. Used for both `Generic` and
    /// `Narrated` visibility so the public stream always opens with non-empty safe text.
    fn safe_public_tool_text(&self) -> String {
        let trimmed = self.generic_tool_text.trim();
        if trimmed.is_empty() {
            everruns_platform::app::DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string()
        } else {
            trimmed.to_string()
        }
    }
}

fn push_public_tool_activity_start(state: &mut AgUiStreamState, text: String) {
    if !state.public_tool_activity_started {
        if !state.thinking_started {
            state
                .queue
                .push_back(AgUiEvent::ThinkingStart(AgUiThinkingStartEvent {
                    base: agui_base_event(),
                    title: None,
                }));
            state.public_tool_activity_opened_thinking = true;
        }
        state.public_tool_activity_started = true;
    }
    if state.thinking_started && state.thinking_text_started {
        state.queue.push_back(AgUiEvent::ThinkingTextMessageContent(
            AgUiThinkingTextMessageContentEvent {
                base: agui_base_event(),
                delta: format!("\n{text}"),
            },
        ));
        return;
    }
    state.queue.push_back(AgUiEvent::ThinkingTextMessageStart(
        AgUiThinkingTextMessageStartEvent {
            base: agui_base_event(),
        },
    ));
    state.queue.push_back(AgUiEvent::ThinkingTextMessageContent(
        AgUiThinkingTextMessageContentEvent {
            base: agui_base_event(),
            delta: text,
        },
    ));
    state.queue.push_back(AgUiEvent::ThinkingTextMessageEnd(
        AgUiThinkingTextMessageEndEvent {
            base: agui_base_event(),
        },
    ));
}

fn push_public_tool_activity_end(state: &mut AgUiStreamState) {
    if state.public_tool_activity_started && state.active_tool_activity_count == 0 {
        if state.public_tool_activity_opened_thinking && !state.thinking_started {
            state
                .queue
                .push_back(AgUiEvent::ThinkingEnd(AgUiThinkingEndEvent {
                    base: agui_base_event(),
                }));
        }
        state.public_tool_activity_started = false;
        state.public_tool_activity_opened_thinking = false;
    }
}

/// Project a provider `reason.item` summary onto the AG-UI reasoning (thinking)
/// channel. Per the EVE-768 design note the provider-authored summary is a
/// reasoning artifact: it must render on the reasoning channel and is never
/// relabeled as an assistant answer. Only the curated `summary` segments are
/// surfaced here — opaque/encrypted reasoning content is never emitted.
///
/// If a reasoning block is already open (an active `reason.thinking` stream),
/// the summary is appended within it rather than opening a nested block;
/// otherwise a self-contained `THINKING_*` block is emitted for the summary.
fn push_reasoning_summary(state: &mut AgUiStreamState, summary: &[String]) {
    let text = summary
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return;
    }

    // Append to an already-open reasoning text message so the summary joins the
    // active thinking stream instead of opening a nested/duplicate block.
    if state.thinking_started && state.thinking_text_started {
        state.queue.push_back(AgUiEvent::ThinkingTextMessageContent(
            AgUiThinkingTextMessageContentEvent {
                base: agui_base_event(),
                delta: format!("\n{text}"),
            },
        ));
        return;
    }

    // Otherwise emit a self-contained reasoning block. Only open a fresh
    // `ThinkingStart` when no reasoning block (thinking or tool-activity) is
    // currently open, and close only what we opened.
    let thinking_block_open = state.thinking_started || state.public_tool_activity_opened_thinking;
    let opened_thinking = !thinking_block_open;
    if opened_thinking {
        state
            .queue
            .push_back(AgUiEvent::ThinkingStart(AgUiThinkingStartEvent {
                base: agui_base_event(),
                title: None,
            }));
    }
    state.queue.push_back(AgUiEvent::ThinkingTextMessageStart(
        AgUiThinkingTextMessageStartEvent {
            base: agui_base_event(),
        },
    ));
    state.queue.push_back(AgUiEvent::ThinkingTextMessageContent(
        AgUiThinkingTextMessageContentEvent {
            base: agui_base_event(),
            delta: text,
        },
    ));
    state.queue.push_back(AgUiEvent::ThinkingTextMessageEnd(
        AgUiThinkingTextMessageEndEvent {
            base: agui_base_event(),
        },
    ));
    if opened_thinking {
        state
            .queue
            .push_back(AgUiEvent::ThinkingEnd(AgUiThinkingEndEvent {
                base: agui_base_event(),
            }));
    }
}

fn translate_event(state: &mut AgUiStreamState, event: &everruns_core::Event) {
    match event.event_type.as_str() {
        "output.message.delta" => {
            if let Ok(data) = parse_event_data::<OutputMessageDeltaData>(event) {
                let message_id = ensure_assistant_message_id(state, data.message_id);
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
                let public_text = public_content_parts_to_string(&data.message.content);
                if !is_terminal_public_output_message(&data.message, state.assistant_emitted_delta)
                {
                    close_assistant_text_without_finishing(state);
                    return;
                }
                let message_id = ensure_assistant_message_id(state, data.message.id);
                if !state.assistant_content_started {
                    state
                        .queue
                        .push_back(AgUiEvent::TextMessageStart(AgUiTextMessageStartEvent {
                            base: agui_base_event(),
                            message_id: message_id.clone(),
                            role: AgUiRole::Assistant,
                        }));
                }
                if !state.assistant_emitted_delta && !public_text.is_empty() {
                    state.queue.push_back(AgUiEvent::TextMessageContent(
                        AgUiTextMessageContentEvent::new(message_id.clone(), public_text).unwrap(),
                    ));
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
            if !state.public_tool_activity_opened_thinking {
                state
                    .queue
                    .push_back(AgUiEvent::ThinkingStart(AgUiThinkingStartEvent {
                        base: agui_base_event(),
                        title: None,
                    }));
            }
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
            state.public_tool_activity_opened_thinking = false;
        }
        // EVE-775: a provider `reason.item` summary is a *reasoning artifact*
        // (EVE-768 design note), so when channel policy opts in it renders on
        // the AG-UI reasoning channel (`THINKING_*` / `REASONING_*`) — never on
        // the assistant-text channel. Opaque `encrypted_content` is never emitted.
        "reason.item" => {
            if state.reasoning_summary_visible
                && let Ok(data) = parse_event_data::<ReasonItemData>(event)
            {
                push_reasoning_summary(state, &data.summary);
            }
        }
        "tool.started" if parse_event_data::<ToolStartedData>(event).is_ok() => {
            state.active_tool_activity_count += 1;
            match state.tool_visibility {
                AgUiToolVisibility::None => {}
                // Both Generic and Narrated emit the safe channel-configured generic text.
                // Narrated must not forward backend/model-authored narration because narration
                // may derive from raw tool-call arguments. `safe_public_tool_text` falls back
                // to the platform default if the configured text is empty/whitespace so we
                // never push an empty delta on the public stream.
                AgUiToolVisibility::Generic | AgUiToolVisibility::Narrated => {
                    let text = state.safe_public_tool_text();
                    push_public_tool_activity_start(state, text);
                }
            }
        }
        "tool.completed" if parse_event_data::<ToolCompletedData>(event).is_ok() => {
            state.active_tool_activity_count = state.active_tool_activity_count.saturating_sub(1);
            push_public_tool_activity_end(state);
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
        // Cancellation is a deliberate terminal state (typically client-initiated),
        // not a server fault. Emit RUN_FINISHED so AG-UI clients see a clean end
        // rather than a misleading internal_error. The cancellation reason lives
        // in the internal session events for operators.
        "turn.cancelled" if !state.finished => {
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
        "turn.failed" => {
            // AG-UI is a public, app-scoped channel — see knowledge/execution/public-endpoints.md.
            // Sanitize via the shared `PublicError` so internal codes, provider
            // strings, model IDs, and quota state never reach the wire.
            let internal_code = parse_event_data::<TurnFailedData>(event)
                .ok()
                .and_then(|data| data.error_code);
            if !state.finished {
                let error = PublicError::from_internal_code(internal_code.as_deref());
                state.queue.push_back(public_run_error_event(error));
                state.finished = true;
            }
        }
        _ => {}
    }
}

/// Returns the age of a session (in seconds) when it has exceeded the
/// configured expiration. Returns `None` when expiration is disabled
/// (`max_seconds == 0`) or when the session is still within the window.
fn expired_age_seconds(
    created_at: chrono::DateTime<chrono::Utc>,
    max_seconds: u32,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<i64> {
    if max_seconds == 0 {
        return None;
    }
    let age = now.signed_duration_since(created_at).num_seconds().max(0);
    (age > max_seconds as i64).then_some(age)
}

fn agui_base_event() -> AgUiBaseEvent {
    AgUiBaseEvent {
        timestamp: None,
        raw_event: None,
    }
}

/// Adapt a sanitized `PublicError` into an AG-UI `RunError` event. All public
/// error emission on this endpoint must go through here so the contract from
/// `knowledge/execution/public-endpoints.md` is enforced in one place.
fn public_run_error_event(error: PublicError) -> AgUiEvent {
    AgUiEvent::RunError(AgUiRunErrorEvent {
        base: agui_base_event(),
        message: error.message.to_string(),
        code: Some(error.code.as_str().to_string()),
    })
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

fn internal_error(err: anyhow::Error) -> Response {
    tracing::error!(error = %err, "AG-UI route failed");
    ErrorResponse::new("Internal server error".to_string())
        .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        .into_response()
}

fn extract_ag_ui_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers.get(AG_UI_TOKEN_HEADER) {
        return value.to_str().ok();
    }

    let auth = headers.get(AUTHORIZATION)?.to_str().ok()?;
    auth.strip_prefix("Bearer ")
}

/// Reject AG-UI request bodies that smuggle non-{user,assistant} message
/// roles or reuse the same message id for multiple entries.
///
/// `Message` deserialises every variant the upstream protocol defines, so
/// without this gate a public AG-UI client could populate `system`,
/// `developer`, or `tool` messages and have them flow into the LLM context
/// alongside the agent's real system prompt. We refuse such requests with a
/// generic 400 `invalid_request` and never echo the offending role.
fn validate_input_messages(messages: &[AgUiMessage]) -> Result<(), Box<Response>> {
    use std::collections::HashSet;
    let mut seen_ids: HashSet<&AgUiMessageId> = HashSet::with_capacity(messages.len());
    for message in messages {
        match message {
            AgUiMessage::User { .. } | AgUiMessage::Assistant { .. } => {}
            AgUiMessage::System { .. }
            | AgUiMessage::Developer { .. }
            | AgUiMessage::Tool { .. } => {
                tracing::warn!("AG-UI request rejected: disallowed message role");
                return Err(Box::new(bad_request("invalid_request")));
            }
        }
        if !seen_ids.insert(message.id()) {
            tracing::warn!("AG-UI request rejected: duplicate message id");
            return Err(Box::new(bad_request("invalid_request")));
        }
    }
    Ok(())
}

fn bad_request(message: &str) -> Response {
    ErrorResponse::new(message.to_string())
        .into_response(StatusCode::BAD_REQUEST)
        .into_response()
}

fn forbidden(message: &str) -> Response {
    ErrorResponse::new(message.to_string())
        .into_response(StatusCode::FORBIDDEN)
        .into_response()
}

fn unauthorized() -> Response {
    ErrorResponse::new("Invalid or missing AG-UI token".to_string())
        .into_response(StatusCode::UNAUTHORIZED)
        .into_response()
}

fn service_unavailable(message: &str) -> Response {
    ErrorResponse::new(message.to_string())
        .into_response(StatusCode::SERVICE_UNAVAILABLE)
        .into_response()
}

fn ag_ui_auth_error_response(error: AppEndpointAuthError) -> Response {
    match error {
        AppEndpointAuthError::Unauthorized => unauthorized(),
        AppEndpointAuthError::Misconfigured => forbidden("AG-UI auth is misconfigured"),
        AppEndpointAuthError::ProviderUnavailable => {
            service_unavailable("AG-UI auth provider is unavailable")
        }
    }
}

fn not_found() -> Response {
    ErrorResponse::new("App not found".to_string())
        .into_response(StatusCode::NOT_FOUND)
        .into_response()
}

fn too_many_requests(message: &str) -> Response {
    let (status, json) =
        ErrorResponse::new(message.to_string()).into_response(StatusCode::TOO_MANY_REQUESTS);
    (status, [("retry-after", "60")], json).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::{
        Event, EventContext, Message, MessageId, OutputMessageCompletedData,
        OutputMessageDeltaData, SessionId, ToolCall, ToolCompletedData, ToolStartedData, TurnId,
    };
    use ag_ui_core::event::EventType as AgUiEventType;
    use chrono::Duration as ChronoDuration;
    use everruns_core::events::TurnFailedData;
    use everruns_provider::execution_phase::ExecutionPhase;

    #[test]
    fn test_expired_age_seconds_within_window() {
        let now = chrono::Utc::now();
        let created = now - ChronoDuration::seconds(100);
        assert_eq!(expired_age_seconds(created, 200, now), None);
    }

    #[test]
    fn test_expired_age_seconds_past_window() {
        let now = chrono::Utc::now();
        let created = now - ChronoDuration::seconds(300);
        assert_eq!(expired_age_seconds(created, 200, now), Some(300));
    }

    #[test]
    fn test_expired_age_seconds_zero_disables_expiration() {
        let now = chrono::Utc::now();
        let created = now - ChronoDuration::days(30);
        assert_eq!(expired_age_seconds(created, 0, now), None);
    }

    #[test]
    fn test_expired_age_seconds_clock_skew() {
        let now = chrono::Utc::now();
        let created = now + ChronoDuration::seconds(5);
        assert_eq!(expired_age_seconds(created, 60, now), None);
    }

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
            tool_visibility: AgUiToolVisibility::Generic,
            generic_tool_text: "Working...".to_string(),
            reasoning_summary_visible: false,
            active_tool_activity_count: 0,
            public_tool_activity_started: false,
            public_tool_activity_opened_thinking: false,
            finished: false,
        }
    }

    fn test_app() -> App {
        use everruns_provider::typed_id::{AppId, HarnessId, PrincipalId};

        let now = chrono::Utc::now();
        App {
            public_id: AppId::from_seed(1),
            internal_id: Uuid::nil(),
            org_id: 1,
            name: "Test App".to_string(),
            description: None,
            harness_id: HarnessId::from_seed(2),
            agent_id: None,
            agent_version_policy: everruns_platform::AgentVersionPolicy::Default,
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(3),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            channels: vec![],
            status: AppStatus::Published,
            published_at: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        }
    }

    #[test]
    fn ag_ui_message_metadata_includes_system_app_id() {
        let app = test_app();
        let metadata = ag_ui_message_metadata(&app, "thread-1".to_string(), "run-1".to_string());

        assert_eq!(
            metadata.get("_app_id"),
            Some(&Value::String(app.public_id.to_string()))
        );
        assert_eq!(
            metadata.get("ag_ui_thread_id"),
            Some(&Value::String("thread-1".to_string()))
        );
    }

    #[test]
    fn ag_ui_image_metadata_scopes_upload_to_app() {
        let app = test_app();
        let metadata = ag_ui_image_metadata(&app);
        let app_public_id = app.public_id.to_string();

        assert!(is_ag_ui_app_image(&metadata, &app_public_id));
        assert!(!is_ag_ui_app_image(
            &serde_json::json!({"source": "ag_ui"}),
            &app_public_id
        ));
    }

    #[test]
    fn forwarded_ag_ui_image_ids_parse_camel_case_ids() {
        let image_id = ImageId::from_seed(42);
        let props = serde_json::json!({ "imageIds": [image_id.to_string()] });

        assert_eq!(forwarded_ag_ui_image_ids(&props).unwrap(), vec![image_id]);
    }

    #[test]
    fn forwarded_ag_ui_image_ids_reject_bad_shape() {
        assert!(forwarded_ag_ui_image_ids(&serde_json::json!({ "imageIds": "img_bad" })).is_err());
        assert!(forwarded_ag_ui_image_ids(&serde_json::json!({ "imageIds": [42] })).is_err());
    }

    #[tokio::test]
    async fn test_translate_output_completion_to_ag_ui_run_finished() {
        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let output_message_id = MessageId::new();
        let event = Event::new(
            SessionId::from_uuid(state.session_id),
            EventContext::turn(turn_id, input_message_id),
            OutputMessageCompletedData::new(
                Message::assistant("Hello from AG-UI").with_id(output_message_id),
            ),
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
        match &state.queue[0] {
            AgUiEvent::TextMessageStart(event) => assert_eq!(
                event.message_id,
                AgUiMessageId::from(output_message_id.uuid())
            ),
            _ => panic!("expected text start event"),
        }
        assert!(state.finished);
    }

    #[tokio::test]
    async fn test_translate_streaming_delta_does_not_duplicate_final_content() {
        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let context = EventContext::turn(turn_id, input_message_id);
        let session_id = SessionId::from_uuid(state.session_id);
        let streamed_message_id = MessageId::new();

        let delta_event = Event::new(
            session_id,
            context.clone(),
            OutputMessageDeltaData {
                turn_id,
                message_id: streamed_message_id,
                delta: "Hello".to_string(),
                accumulated: "Hello".to_string(),
                phase: None,
            },
        );
        translate_event(&mut state, &delta_event);

        let completed_event = Event::new(
            session_id,
            context,
            OutputMessageCompletedData::new(
                Message::assistant("Hello from AG-UI").with_id(streamed_message_id),
            ),
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
    async fn test_commentary_delta_tool_completion_does_not_hide_final_answer() {
        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let context = EventContext::turn(turn_id, input_message_id);
        let session_id = SessionId::from_uuid(state.session_id);
        let commentary_message_id = MessageId::new();
        let final_message_id = MessageId::new();

        let delta_event = Event::new(
            session_id,
            context.clone(),
            OutputMessageDeltaData {
                turn_id,
                message_id: commentary_message_id,
                delta: "Let me check that.".to_string(),
                accumulated: "Let me check that.".to_string(),
                phase: None,
            },
        );
        translate_event(&mut state, &delta_event);

        let commentary_completed = Event::new(
            session_id,
            context.clone(),
            OutputMessageCompletedData::new(
                Message::assistant_with_tools(
                    "",
                    vec![ToolCall {
                        id: "call_lookup".to_string(),
                        name: "lookup".to_string(),
                        arguments: serde_json::json!({"query": "base harness"}),
                    }],
                )
                .with_id(commentary_message_id)
                .with_phase(ExecutionPhase::Commentary),
            ),
        );
        translate_event(&mut state, &commentary_completed);

        let event_types: Vec<AgUiEventType> =
            state.queue.iter().map(AgUiEvent::event_type).collect();
        assert_eq!(
            event_types,
            vec![
                AgUiEventType::TextMessageStart,
                AgUiEventType::TextMessageContent,
                AgUiEventType::TextMessageEnd,
            ]
        );
        assert!(!state.finished);

        let final_completed = Event::new(
            session_id,
            context,
            OutputMessageCompletedData::new(
                Message::assistant("The Base harness is the default execution wrapper.")
                    .with_id(final_message_id)
                    .with_phase(ExecutionPhase::FinalAnswer),
            ),
        );
        translate_event(&mut state, &final_completed);

        let event_types: Vec<AgUiEventType> =
            state.queue.iter().map(AgUiEvent::event_type).collect();
        assert_eq!(
            event_types,
            vec![
                AgUiEventType::TextMessageStart,
                AgUiEventType::TextMessageContent,
                AgUiEventType::TextMessageEnd,
                AgUiEventType::TextMessageStart,
                AgUiEventType::TextMessageContent,
                AgUiEventType::TextMessageEnd,
                AgUiEventType::RunFinished,
            ]
        );
        let content_events: Vec<&AgUiTextMessageContentEvent> = state
            .queue
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::TextMessageContent(event) => Some(event),
                _ => None,
            })
            .collect();
        assert_eq!(content_events.len(), 2);
        assert_eq!(content_events[0].delta, "Let me check that.");
        assert_eq!(
            content_events[1].delta,
            "The Base harness is the default execution wrapper."
        );
        assert!(state.finished);

        // EVE-773: AG-UI projects the canonical streamed message ids, preserving
        // the commentary/final boundary without exposing the turn uuid.
        let start_ids: Vec<&AgUiMessageId> = state
            .queue
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::TextMessageStart(event) => Some(&event.message_id),
                _ => None,
            })
            .collect();
        assert_eq!(start_ids.len(), 2);
        assert_ne!(
            start_ids[0], start_ids[1],
            "commentary and final answer must have distinct messageIds"
        );
        assert_eq!(
            *start_ids[0],
            AgUiMessageId::from(commentary_message_id.uuid())
        );
        assert_eq!(*start_ids[1], AgUiMessageId::from(final_message_id.uuid()));
        let turn_message_id = AgUiMessageId::from(turn_id.uuid());
        assert_ne!(*start_ids[0], turn_message_id);
        assert_ne!(*start_ids[1], turn_message_id);
    }

    #[tokio::test]
    async fn test_reason_item_summary_hidden_by_default() {
        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let event = Event::new(
            SessionId::from_uuid(state.session_id),
            EventContext::turn(turn_id, input_message_id),
            ReasonItemData {
                turn_id,
                provider: "openai".to_string(),
                model: Some("gpt-5.5".to_string()),
                item_id: "rs_private".to_string(),
                encrypted_content: Some("OPAQUE-DO-NOT-LEAK".to_string()),
                summary: vec!["Looked up private CRM details.".to_string()],
                token_count: Some(42),
            },
        );

        translate_event(&mut state, &event);

        assert!(state.queue.is_empty());
        assert!(!state.finished);
    }

    // EVE-775: a provider `reason.item` summary is a reasoning artifact and must
    // project onto the AG-UI reasoning channel (THINKING_*), never the
    // assistant-text channel, and never leak opaque reasoning content.
    #[tokio::test]
    async fn test_reason_item_summary_projects_to_reasoning_channel_when_enabled() {
        let mut state = test_stream_state().await;
        state.reasoning_summary_visible = true;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let event = Event::new(
            SessionId::from_uuid(state.session_id),
            EventContext::turn(turn_id, input_message_id),
            ReasonItemData {
                turn_id,
                provider: "openai".to_string(),
                model: Some("gpt-5.5".to_string()),
                item_id: "rs_1".to_string(),
                encrypted_content: Some("OPAQUE-DO-NOT-LEAK".to_string()),
                summary: vec!["Considered the file layout.".to_string()],
                token_count: Some(42),
            },
        );

        translate_event(&mut state, &event);

        let event_types: Vec<AgUiEventType> =
            state.queue.iter().map(AgUiEvent::event_type).collect();
        assert_eq!(
            event_types,
            vec![
                AgUiEventType::ThinkingStart,
                AgUiEventType::ThinkingTextMessageStart,
                AgUiEventType::ThinkingTextMessageContent,
                AgUiEventType::ThinkingTextMessageEnd,
                AgUiEventType::ThinkingEnd,
            ]
        );
        // The reasoning summary must never appear on the assistant-text channel.
        assert!(!state.queue.iter().any(|event| matches!(
            event,
            AgUiEvent::TextMessageStart(_)
                | AgUiEvent::TextMessageContent(_)
                | AgUiEvent::TextMessageEnd(_)
        )));
        match &state.queue[2] {
            AgUiEvent::ThinkingTextMessageContent(event) => {
                assert_eq!(event.delta, "Considered the file layout.");
            }
            other => panic!("expected thinking content, got {other:?}"),
        }
        // Hidden/opaque reasoning content is never exposed anywhere on the wire.
        let serialized: String = state
            .queue
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect();
        assert!(
            !serialized.contains("OPAQUE-DO-NOT-LEAK"),
            "opaque reasoning must never reach the AG-UI wire: {serialized}"
        );
        // A reasoning summary does not end the run — the final answer still follows.
        assert!(!state.finished);
    }

    #[tokio::test]
    async fn test_reason_item_empty_summary_emits_nothing() {
        let mut state = test_stream_state().await;
        state.reasoning_summary_visible = true;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let event = Event::new(
            SessionId::from_uuid(state.session_id),
            EventContext::turn(turn_id, input_message_id),
            ReasonItemData {
                turn_id,
                provider: "openai".to_string(),
                model: None,
                item_id: "rs_empty".to_string(),
                encrypted_content: Some("OPAQUE".to_string()),
                summary: vec![String::new(), "   ".to_string()],
                token_count: None,
            },
        );
        translate_event(&mut state, &event);
        assert!(state.queue.is_empty());
    }

    // EVE-768 AC#7: reasoning summary vs commentary channel separation. Commentary
    // is deliberate user-facing assistant text; the provider reasoning summary is a
    // reasoning artifact. They must land on different channels and never mix.
    #[tokio::test]
    async fn test_reason_summary_and_commentary_use_separate_channels() {
        let mut state = test_stream_state().await;
        state.reasoning_summary_visible = true;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let context = EventContext::turn(turn_id, input_message_id);
        let session_id = SessionId::from_uuid(state.session_id);

        let commentary_delta = Event::new(
            session_id,
            context.clone(),
            OutputMessageDeltaData {
                turn_id,
                message_id: MessageId::new(),
                delta: "Let me look into that.".to_string(),
                accumulated: "Let me look into that.".to_string(),
                phase: None,
            },
        );
        translate_event(&mut state, &commentary_delta);

        let reason_item = Event::new(
            session_id,
            context,
            ReasonItemData {
                turn_id,
                provider: "openai".to_string(),
                model: None,
                item_id: "rs_sep".to_string(),
                encrypted_content: None,
                summary: vec!["Summarized the plan.".to_string()],
                token_count: None,
            },
        );
        translate_event(&mut state, &reason_item);

        // Assistant-text channel carries only the commentary.
        let text_deltas: Vec<&str> = state
            .queue
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::TextMessageContent(event) => Some(event.delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text_deltas, vec!["Let me look into that."]);

        // Reasoning channel carries only the summary.
        let thinking_deltas: Vec<&str> = state
            .queue
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::ThinkingTextMessageContent(event) => Some(event.delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(thinking_deltas, vec!["Summarized the plan."]);
    }

    #[tokio::test]
    async fn test_reason_item_summary_appends_to_active_thinking_block() {
        let mut state = test_stream_state().await;
        state.reasoning_summary_visible = true;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let context = EventContext::turn(turn_id, input_message_id);
        let session_id = SessionId::from_uuid(state.session_id);

        let thinking_started = Event::new(
            session_id,
            context.clone(),
            ReasonThinkingStartedData {
                turn_id,
                model: None,
            },
        );
        translate_event(&mut state, &thinking_started);
        let thinking_delta = Event::new(
            session_id,
            context.clone(),
            ReasonThinkingDeltaData {
                turn_id,
                delta: "Reasoning...".to_string(),
                accumulated: "Reasoning...".to_string(),
            },
        );
        translate_event(&mut state, &thinking_delta);

        let reason_item = Event::new(
            session_id,
            context,
            ReasonItemData {
                turn_id,
                provider: "openai".to_string(),
                model: None,
                item_id: "rs_app".to_string(),
                encrypted_content: None,
                summary: vec!["Summary tail.".to_string()],
                token_count: None,
            },
        );
        translate_event(&mut state, &reason_item);

        let event_types: Vec<AgUiEventType> =
            state.queue.iter().map(AgUiEvent::event_type).collect();
        // The summary appends into the open reasoning block — no duplicate start.
        assert_eq!(
            event_types
                .iter()
                .filter(|event_type| **event_type == AgUiEventType::ThinkingStart)
                .count(),
            1
        );
        assert!(
            !state
                .queue
                .iter()
                .any(|event| matches!(event, AgUiEvent::TextMessageContent(_)))
        );
        match state.queue.back().unwrap() {
            AgUiEvent::ThinkingTextMessageContent(event) => {
                assert_eq!(event.delta, "\nSummary tail.");
            }
            other => panic!("expected appended thinking content, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_tool_call_completion_does_not_finish_before_final_answer() {
        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let context = EventContext::turn(turn_id, input_message_id);
        let session_id = SessionId::from_uuid(state.session_id);

        let commentary_completed = Event::new(
            session_id,
            context.clone(),
            OutputMessageCompletedData::new(
                Message::assistant_with_tools(
                    "",
                    vec![ToolCall {
                        id: "call_list_skills".to_string(),
                        name: "list_skills".to_string(),
                        arguments: serde_json::json!({"registry": "internal"}),
                    }],
                )
                .with_phase(ExecutionPhase::Commentary),
            ),
        );
        translate_event(&mut state, &commentary_completed);
        assert!(state.queue.is_empty());
        assert!(!state.finished);

        let tool_started = Event::new(
            session_id,
            context.clone(),
            ToolStartedData {
                tool_call: ToolCall {
                    id: "call_list_skills".to_string(),
                    name: "list_skills".to_string(),
                    arguments: serde_json::json!({"registry": "internal"}),
                },
                tool_call_fingerprint: None,
                display_name: None,
                narration: Some("Listing internal skills".to_string()),
            },
        );
        translate_event(&mut state, &tool_started);

        let tool_completed = Event::new(
            session_id,
            context.clone(),
            ToolCompletedData {
                tool_call_id: "call_list_skills".to_string(),
                tool_name: "list_skills".to_string(),
                tool_call_fingerprint: None,
                tool_result_fingerprint: None,
                display_name: None,
                success: true,
                status: "success".to_string(),
                result: Some(vec![ContentPart::text("skill_123")]),
                error: None,
                duration_ms: Some(10),
                capability_id: None,
                capability_name: None,
                narration: None,
            },
        );
        translate_event(&mut state, &tool_completed);
        assert!(!state.finished);

        let final_completed = Event::new(
            session_id,
            context,
            OutputMessageCompletedData::new(
                Message::assistant("The Base harness is the default execution wrapper.")
                    .with_phase(ExecutionPhase::FinalAnswer),
            ),
        );
        translate_event(&mut state, &final_completed);

        let event_types: Vec<AgUiEventType> =
            state.queue.iter().map(AgUiEvent::event_type).collect();
        assert_eq!(
            event_types,
            vec![
                AgUiEventType::ThinkingStart,
                AgUiEventType::ThinkingTextMessageStart,
                AgUiEventType::ThinkingTextMessageContent,
                AgUiEventType::ThinkingTextMessageEnd,
                AgUiEventType::ThinkingEnd,
                AgUiEventType::TextMessageStart,
                AgUiEventType::TextMessageContent,
                AgUiEventType::TextMessageEnd,
                AgUiEventType::RunFinished,
            ]
        );
        match &state.queue[2] {
            AgUiEvent::ThinkingTextMessageContent(event) => {
                assert_eq!(event.delta, "Working...");
                assert!(!event.delta.contains("list_skills"));
                assert!(!event.delta.contains("internal"));
                assert!(!event.delta.contains("skill_123"));
            }
            _ => panic!("expected generic thinking content"),
        }
        match &state.queue[6] {
            AgUiEvent::TextMessageContent(event) => {
                assert_eq!(
                    event.delta,
                    "The Base harness is the default execution wrapper."
                );
            }
            _ => panic!("expected final answer text content"),
        }
        assert!(state.finished);
    }

    #[tokio::test]
    async fn test_generic_tool_activity_hides_public_tool_details() {
        let mut state = test_stream_state().await;
        state.generic_tool_text = "Checking now".to_string();
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
                tool_call_fingerprint: None,
                display_name: None,
                narration: Some("Searching for hello".to_string()),
            },
        );
        translate_event(&mut state, &tool_started);

        let tool_completed = Event::new(
            session_id,
            context.clone(),
            ToolCompletedData {
                tool_call_id: "call_1".to_string(),
                tool_name: "web_search".to_string(),
                tool_call_fingerprint: None,
                tool_result_fingerprint: None,
                display_name: None,
                success: true,
                status: "success".to_string(),
                result: Some(vec![ContentPart::text("result")]),
                error: None,
                duration_ms: Some(10),
                capability_id: None,
                capability_name: None,
                narration: None,
            },
        );
        translate_event(&mut state, &tool_completed);

        let event_types: Vec<AgUiEventType> =
            state.queue.iter().map(AgUiEvent::event_type).collect();
        assert_eq!(
            event_types,
            vec![
                AgUiEventType::ThinkingStart,
                AgUiEventType::ThinkingTextMessageStart,
                AgUiEventType::ThinkingTextMessageContent,
                AgUiEventType::ThinkingTextMessageEnd,
                AgUiEventType::ThinkingEnd,
            ]
        );
        match &state.queue[2] {
            AgUiEvent::ThinkingTextMessageContent(event) => {
                assert_eq!(event.delta, "Checking now");
                assert!(!event.delta.contains("web_search"));
                assert!(!event.delta.contains("hello"));
                assert!(!event.delta.contains("result"));
            }
            _ => panic!("expected generic thinking content"),
        }

        let output_completed = Event::new(
            session_id,
            context,
            OutputMessageCompletedData::new(Message::assistant("Hello after tool")),
        );
        translate_event(&mut state, &output_completed);

        // The public messageId is message-scoped and must never be the raw turn uuid.
        let turn_message_id = AgUiMessageId::from(turn_id.uuid());
        match &state.queue[5] {
            AgUiEvent::TextMessageStart(event) => {
                assert_ne!(event.message_id, turn_message_id);
            }
            _ => panic!("expected text start event"),
        }
    }

    #[tokio::test]
    async fn test_tool_activity_reuses_active_reason_thinking_block() {
        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let context = EventContext::turn(turn_id, input_message_id);
        let session_id = SessionId::from_uuid(state.session_id);

        let thinking_started = Event::new(
            session_id,
            context.clone(),
            ReasonThinkingStartedData {
                turn_id,
                model: Some("test-model".to_string()),
            },
        );
        translate_event(&mut state, &thinking_started);

        let thinking_delta = Event::new(
            session_id,
            context.clone(),
            ReasonThinkingDeltaData {
                turn_id,
                delta: "Thinking".to_string(),
                accumulated: "Thinking".to_string(),
            },
        );
        translate_event(&mut state, &thinking_delta);

        let tool_started = Event::new(
            session_id,
            context.clone(),
            ToolStartedData {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "web_search".to_string(),
                    arguments: serde_json::json!({"q": "private query"}),
                },
                tool_call_fingerprint: None,
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
                tool_call_fingerprint: None,
                tool_result_fingerprint: None,
                display_name: None,
                success: true,
                status: "success".to_string(),
                result: Some(vec![ContentPart::text("private result")]),
                error: None,
                duration_ms: Some(10),
                capability_id: None,
                capability_name: None,
                narration: None,
            },
        );
        translate_event(&mut state, &tool_completed);

        let thinking_completed = Event::new(
            session_id,
            context,
            ReasonThinkingCompletedData {
                turn_id,
                thinking: "Thinking".to_string(),
            },
        );
        translate_event(&mut state, &thinking_completed);

        let event_types: Vec<AgUiEventType> =
            state.queue.iter().map(AgUiEvent::event_type).collect();
        assert_eq!(
            event_types,
            vec![
                AgUiEventType::ThinkingStart,
                AgUiEventType::ThinkingTextMessageStart,
                AgUiEventType::ThinkingTextMessageContent,
                AgUiEventType::ThinkingTextMessageContent,
                AgUiEventType::ThinkingTextMessageEnd,
                AgUiEventType::ThinkingEnd,
            ]
        );
        assert_eq!(
            event_types
                .iter()
                .filter(|event_type| **event_type == AgUiEventType::ThinkingStart)
                .count(),
            1
        );
        assert_eq!(
            event_types
                .iter()
                .filter(|event_type| **event_type == AgUiEventType::ThinkingEnd)
                .count(),
            1
        );
        match &state.queue[3] {
            AgUiEvent::ThinkingTextMessageContent(event) => {
                assert_eq!(event.delta, "\nWorking...");
                assert!(!event.delta.contains("web_search"));
                assert!(!event.delta.contains("private query"));
                assert!(!event.delta.contains("private result"));
            }
            _ => panic!("expected generic tool activity content"),
        }
    }

    #[tokio::test]
    async fn test_none_tool_visibility_hides_public_tool_activity() {
        let mut state = test_stream_state().await;
        state.tool_visibility = AgUiToolVisibility::None;
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
                tool_call_fingerprint: None,
                display_name: None,
                narration: Some("Searching for hello".to_string()),
            },
        );
        translate_event(&mut state, &tool_started);

        let tool_completed = Event::new(
            session_id,
            context,
            ToolCompletedData {
                tool_call_id: "call_1".to_string(),
                tool_name: "web_search".to_string(),
                tool_call_fingerprint: None,
                tool_result_fingerprint: None,
                display_name: None,
                success: true,
                status: "success".to_string(),
                result: Some(vec![ContentPart::text("result")]),
                error: None,
                duration_ms: Some(10),
                capability_id: None,
                capability_name: None,
                narration: None,
            },
        );
        translate_event(&mut state, &tool_completed);

        assert!(state.queue.is_empty());
    }

    #[tokio::test]
    async fn test_narrated_tool_visibility_falls_back_to_generic_text() {
        let mut state = test_stream_state().await;
        state.tool_visibility = AgUiToolVisibility::Narrated;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let context = EventContext::turn(turn_id, input_message_id);
        let session_id = SessionId::from_uuid(state.session_id);

        let tool_started = Event::new(
            session_id,
            context,
            ToolStartedData {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "web_search".to_string(),
                    arguments: serde_json::json!({"q": "private query"}),
                },
                tool_call_fingerprint: None,
                display_name: Some("Web Search".to_string()),
                narration: Some("Checking public sources".to_string()),
            },
        );
        translate_event(&mut state, &tool_started);

        match &state.queue[2] {
            AgUiEvent::ThinkingTextMessageContent(event) => {
                assert_eq!(event.delta, "Working...");
                assert!(!event.delta.contains("web_search"));
                assert!(!event.delta.contains("private query"));
            }
            _ => panic!("expected narrated thinking content"),
        }
    }

    #[tokio::test]
    async fn test_public_tool_text_falls_back_when_configured_value_is_empty() {
        // Existing channels may have stored an empty/whitespace generic_tool_text from
        // before validation tightened. The public stream must never emit an empty delta.
        for visibility in [AgUiToolVisibility::Generic, AgUiToolVisibility::Narrated] {
            for configured in ["", "   ", "\n\t"] {
                let mut state = test_stream_state().await;
                state.tool_visibility = visibility;
                state.generic_tool_text = configured.to_string();
                let turn_id = TurnId::new();
                let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
                let context = EventContext::turn(turn_id, input_message_id);
                let session_id = SessionId::from_uuid(state.session_id);

                let tool_started = Event::new(
                    session_id,
                    context,
                    ToolStartedData {
                        tool_call: ToolCall {
                            id: "call_1".to_string(),
                            name: "web_search".to_string(),
                            arguments: serde_json::json!({"q": "x"}),
                        },
                        tool_call_fingerprint: None,
                        display_name: None,
                        narration: Some("backend narration".to_string()),
                    },
                );
                translate_event(&mut state, &tool_started);

                match &state.queue[2] {
                    AgUiEvent::ThinkingTextMessageContent(event) => {
                        assert_eq!(
                            event.delta, "Working...",
                            "visibility={visibility:?} configured={configured:?}"
                        );
                    }
                    _ => panic!(
                        "expected non-empty thinking content for visibility {visibility:?} configured {configured:?}"
                    ),
                }
            }
        }
    }

    fn turn_failed_event(state: &AgUiStreamState, data: TurnFailedData) -> Event {
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        Event::new(
            SessionId::from_uuid(state.session_id),
            EventContext::turn(data.turn_id, input_message_id),
            data,
        )
    }

    fn assert_run_error(state: &AgUiStreamState, expected_message: &str, expected_code: &str) {
        assert_eq!(state.queue.len(), 1);
        match state.queue.front() {
            Some(AgUiEvent::RunError(event)) => {
                assert_eq!(event.message, expected_message);
                assert_eq!(event.code.as_deref(), Some(expected_code));
            }
            other => panic!("expected RunError, got {other:?}"),
        }
        assert!(state.finished);
    }

    #[tokio::test]
    async fn turn_failed_rate_limit_returns_generic_message() {
        let mut state = test_stream_state().await;
        let event = turn_failed_event(
            &state,
            TurnFailedData {
                turn_id: TurnId::new(),
                error: "OpenAI API error (429): {\"error\":{\"message\":\"Rate limit reached for gpt-4o in organization org-redacted on requests per min (RPM): Limit 500\",\"code\":\"rate_limit_exceeded\"}}".to_string(),
                error_code: Some(user_facing_error_codes::PROVIDER_RATE_LIMITED.to_string()),
                error_fields: None,
                error_disclosure: None,
            },
        );

        translate_event(&mut state, &event);

        assert_run_error(
            &state,
            "The service is busy right now. Please try again in a moment.",
            "rate_limited",
        );
    }

    #[tokio::test]
    async fn turn_failed_provider_unavailable_returns_generic_message() {
        let mut state = test_stream_state().await;
        let event = turn_failed_event(
            &state,
            TurnFailedData {
                turn_id: TurnId::new(),
                error: "OpenAI API error (503): server overloaded".to_string(),
                error_code: Some(user_facing_error_codes::PROVIDER_UNAVAILABLE.to_string()),
                error_fields: None,
                error_disclosure: None,
            },
        );

        translate_event(&mut state, &event);

        assert_run_error(
            &state,
            "The service is temporarily unavailable. Please try again shortly.",
            "service_unavailable",
        );
    }

    #[tokio::test]
    async fn turn_failed_zero_credits_does_not_leak_quota_details() {
        // OpenAI surfaces "no credits" as 429 + insufficient_quota; classifier
        // maps it to PROVIDER_MISCONFIGURED (operator must top up or rotate
        // the key). Either way, the public channel must not echo the raw
        // provider body.
        let mut state = test_stream_state().await;
        let raw_error = "OpenAI API error (429): {\"error\":{\"message\":\"You exceeded your current quota, please check your plan and billing details.\",\"type\":\"insufficient_quota\",\"code\":\"insufficient_quota\"}}";
        let event = turn_failed_event(
            &state,
            TurnFailedData {
                turn_id: TurnId::new(),
                error: raw_error.to_string(),
                error_code: Some(user_facing_error_codes::PROVIDER_MISCONFIGURED.to_string()),
                error_fields: None,
                error_disclosure: None,
            },
        );

        translate_event(&mut state, &event);

        match state.queue.front() {
            Some(AgUiEvent::RunError(event)) => {
                assert!(
                    !event.message.contains("OpenAI")
                        && !event.message.contains("quota")
                        && !event.message.contains("429"),
                    "public message must not leak provider details: {}",
                    event.message
                );
                assert!(!event.message.to_lowercase().contains("contact"));
                assert!(!event.message.to_lowercase().contains("admin"));
                assert!(!event.message.to_lowercase().contains("support"));
            }
            other => panic!("expected RunError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn turn_failed_misconfigured_maps_to_service_unavailable() {
        let mut state = test_stream_state().await;
        let event = turn_failed_event(
            &state,
            TurnFailedData {
                turn_id: TurnId::new(),
                error: "OpenAI API error (401): invalid api key".to_string(),
                error_code: Some(user_facing_error_codes::PROVIDER_MISCONFIGURED.to_string()),
                error_fields: None,
                error_disclosure: None,
            },
        );

        translate_event(&mut state, &event);

        assert_run_error(
            &state,
            "The service is temporarily unavailable. Please try again shortly.",
            "service_unavailable",
        );
    }

    #[tokio::test]
    async fn turn_failed_unknown_code_falls_back_to_internal_error() {
        let mut state = test_stream_state().await;
        let event = turn_failed_event(
            &state,
            TurnFailedData {
                turn_id: TurnId::new(),
                error: "stack trace: thread 'tokio-runtime-worker' panicked at 'oops'".to_string(),
                error_code: None,
                error_fields: None,
                error_disclosure: None,
            },
        );

        translate_event(&mut state, &event);

        assert_run_error(
            &state,
            "Something went wrong. Please try again.",
            "internal_error",
        );
    }

    // Note: the exhaustive "never leaks internal concepts" property is
    // verified in crates/server/src/api/public.rs::tests, since the
    // sanitization logic now lives there.

    #[tokio::test]
    async fn turn_cancelled_emits_run_finished_not_run_error() {
        // Cancellations are a deliberate terminal state. Public AG-UI clients
        // must see a clean RUN_FINISHED, not an internal_error.
        use everruns_core::TurnCancelledData;

        let mut state = test_stream_state().await;
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        let event = Event::new(
            SessionId::from_uuid(state.session_id),
            EventContext::turn(turn_id, input_message_id),
            TurnCancelledData {
                turn_id,
                reason: Some("client cancelled".to_string()),
                usage: None,
            },
        );

        translate_event(&mut state, &event);

        assert_eq!(state.queue.len(), 1);
        match state.queue.front() {
            Some(AgUiEvent::RunFinished(_)) => {}
            other => panic!("expected RunFinished for cancellation, got {other:?}"),
        }
        assert!(state.finished);
    }

    // Regression test for the stream_closed branch in run_agent. We exercise the
    // helper directly (the run_agent stream_closed path constructs the same
    // event) so the public RUN_ERROR contract for unexpected stream termination
    // stays pinned to PublicError::fallback() — internal_error / generic message.
    #[test]
    fn stream_closed_falls_back_to_internal_error() {
        let event = public_run_error_event(PublicError::fallback());
        match event {
            AgUiEvent::RunError(event) => {
                assert_eq!(event.code.as_deref(), Some("internal_error"));
                assert_eq!(event.message, "Something went wrong. Please try again.");
                let lower = event.message.to_lowercase();
                assert!(
                    !lower.contains("stream"),
                    "leaked stream detail: {}",
                    event.message
                );
                assert!(
                    !lower.contains("subscription"),
                    "leaked subscription detail: {}",
                    event.message
                );
            }
            other => panic!("expected RunError, got {other:?}"),
        }
    }
}
