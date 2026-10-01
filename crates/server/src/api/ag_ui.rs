// AG-UI app channel — public streaming endpoint
//
// Design Decision: AG-UI ingress is keyed by channel ID at
// `POST /v1/e/{channel_id}/ag-ui`. The app-scoped route remains a permanent
// alias when the App has exactly one enabled AG-UI channel.
//
// Design Decision: The endpoint is public. Requests are accepted
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
    Caller, ContentPart, ExternalActor, RuntimeMessageRole, everruns_provider::typed_id::MessageId,
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
use everruns_platform::exposure::{PublicToolVisibility, public_tool_activity_text};
use everruns_platform::{AgUiChannelConfig, ChannelType};
use everruns_provider::execution_phase::ExecutionPhase;
use everruns_provider::typed_id::ImageId;
#[cfg(test)]
use everruns_provider::user_facing_error::codes as user_facing_error_codes;
use futures::{
    StreamExt,
    stream::{self, Stream},
};
// Design Decision (EVE-1135): the endpoint speaks only AG-UI 1.0 (the
// `everruns-ag-ui` crate) and announces it on `RUN_STARTED.protocolVersion`.
// There is no version negotiation: dropping `THINKING_*` is a breaking change
// for pre-1.0 stream consumers, while pre-1.0 request bodies still parse.
//
// Design Decision (EVE-1135): no `SUBAGENT_*` or `ACTIVITY_*` events yet.
// Delegated work is Session Tasks (`task.*`, which replaced `subagent.*` in
// EVE-585), whose names and descriptions are agent-authored, and this public
// channel never names tools or agents: tool work surfaces only as the
// channel-configured generic text on the reasoning channel. Projecting tasks
// onto `SUBAGENT_*` needs its own channel exposure policy first.
use everruns_ag_ui::{
    AssistantMessage as AgUiAssistantMessage, Content as AgUiMessageContent, Event as AgUiEvent,
    Message as AgUiMessage, MessagesSnapshotEvent as AgUiMessagesSnapshotEvent,
    RunAgentInput as AgUiRunAgentInput, RunErrorEvent as AgUiRunErrorEvent,
    RunFinishedEvent as AgUiRunFinishedEvent, RunStartedEvent as AgUiRunStartedEvent,
    TextMessageContentEvent as AgUiTextMessageContentEvent,
    TextMessageEndEvent as AgUiTextMessageEndEvent,
    TextMessageStartEvent as AgUiTextMessageStartEvent, ToolCall as AgUiToolCall,
    UserMessage as AgUiUserMessage,
};
use reasoning::ReasoningState;
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
    pub public_chat_enabled: bool,
    pub runtime_auth: Option<crate::auth::AuthState>,
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
            public_chat_enabled: false,
            runtime_auth: None,
            encryption,
            db,
        }
    }

    pub fn with_runtime_auth(mut self, auth: crate::auth::AuthState) -> Self {
        self.runtime_auth = Some(auth);
        self
    }

    pub fn with_public_chat_enabled(mut self, enabled: bool) -> Self {
        self.public_chat_enabled = enabled;
        self
    }
}

pub fn routes(state: AgUiState) -> Router {
    Router::new()
        .route("/v1/apps/{app_id}/ag-ui", post(run_agent_legacy))
        .route(
            "/v1/apps/{app_id}/ag-ui/images",
            post(upload_image_legacy).layer(DefaultBodyLimit::max(
                MAX_PUBLIC_AG_UI_IMAGE_SIZE + 1024 * 1024,
            )),
        )
        .route("/v1/e/{channel_id}/ag-ui", post(run_agent_endpoint))
        .route(
            "/v1/e/{channel_id}/ag-ui/images",
            post(upload_image_endpoint).layer(DefaultBodyLimit::max(
                MAX_PUBLIC_AG_UI_IMAGE_SIZE + 1024 * 1024,
            )),
        )
        .with_state(state)
}
enum AgUiTarget {
    LegacyApp(String),
    Endpoint(String),
}

struct AuthorizedAgUiRequest {
    context: crate::api::app_ingress::IngressContext,
    channel_id: String,
    /// Internal id of the endpoint this request arrived through, recorded on
    /// any session it creates (EVE-1004).
    endpoint_internal_id: uuid::Uuid,
    channel_config: AgUiChannelConfig,
    runtime_user: Option<everruns_provider::typed_id::VirtualUserId>,
}

async fn upload_image_legacy(
    State(state): State<AgUiState>,
    Path(app_id): Path<String>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<(StatusCode, Json<ImageUploadResponse>), Response> {
    upload_image(
        state,
        AgUiTarget::LegacyApp(app_id),
        connect_info,
        headers,
        multipart,
    )
    .await
}

async fn upload_image_endpoint(
    State(state): State<AgUiState>,
    Path(channel_id): Path<String>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<(StatusCode, Json<ImageUploadResponse>), Response> {
    upload_image(
        state,
        AgUiTarget::Endpoint(channel_id),
        connect_info,
        headers,
        multipart,
    )
    .await
}

async fn upload_image(
    state: AgUiState,
    target: AgUiTarget,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<ImageUploadResponse>), Response> {
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let AuthorizedAgUiRequest { context, .. } =
        authorize_ag_ui_request(&state, target, &headers, peer_addr).await?;

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
            context.org_id,
            CreateImageRow {
                org_id: context.org_id,
                filename,
                content_type,
                size_bytes,
                data,
                thumbnail_data,
                thumbnail_content_type,
                metadata: ag_ui_image_metadata(&context),
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

async fn run_agent_legacy(
    State(state): State<AgUiState>,
    Path(app_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    request: Request,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, Response> {
    run_agent(
        state,
        AgUiTarget::LegacyApp(app_id),
        req_id,
        connect_info,
        headers,
        request,
    )
    .await
}

async fn run_agent_endpoint(
    State(state): State<AgUiState>,
    Path(channel_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    request: Request,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, Response> {
    run_agent(
        state,
        AgUiTarget::Endpoint(channel_id),
        req_id,
        connect_info,
        headers,
        request,
    )
    .await
}

async fn run_agent(
    state: AgUiState,
    target: AgUiTarget,
    req_id: Option<Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    request: Request,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, Response> {
    let request_id = req_id.map(|Extension(r)| r.0);
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let AuthorizedAgUiRequest {
        context: app,
        channel_id,
        endpoint_internal_id,
        channel_config,
        runtime_user,
    } = authorize_ag_ui_request(&state, target, &headers, peer_addr).await?;

    run_app_agent_stream(
        state,
        app,
        endpoint_internal_id,
        channel_config,
        "ag_ui",
        {
            let mut tags = vec![format!("ag_ui:channel:{channel_id}")];
            if let Some(id) = runtime_user {
                tags.push(format!("ag_ui:virtual_user:{id}"));
            }
            tags
        },
        runtime_user,
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
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_app_agent_stream(
    state: AgUiState,
    app: crate::api::app_ingress::IngressContext,
    endpoint_internal_id: uuid::Uuid,
    channel_config: AgUiChannelConfig,
    tag_prefix: &str,
    extra_routing_tags: Vec<String>,
    runtime_user: Option<everruns_provider::typed_id::VirtualUserId>,
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

    // Everruns requires UUID thread and run ids (1.0 allows any string): they
    // key durable session routing tags, and a UUID bounds their length.
    let thread_id = parse_ag_ui_uuid(&req.thread_id, "threadId")?;
    let run_id = parse_ag_ui_uuid(&req.run_id, "runId")?;

    let trigger_message = req
        .messages
        .last()
        .ok_or_else(|| bad_request("messages must contain at least one user message"))?;
    let (trigger_content, trigger_name) = match trigger_message {
        AgUiMessage::User(message) => (user_text(&message.content)?, message.name.clone()),
        _ => return Err(bad_request("the final AG-UI message must have role=user")),
    };

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
    let runtime_principal = if let Some(id) = runtime_user {
        let principals = crate::services::PrincipalService::new(state.db.clone());
        let parent = principals
            .ensure_system_principal(app.org_id, "external-users")
            .await
            .map_err(internal_error)?;
        Some(
            principals
                .ensure_virtual_user_principal(app.org_id, id, parent.id)
                .await
                .map_err(internal_error)?
                .id,
        )
    } else {
        None
    };
    let mut runtime_app = app.clone();
    if let Some(principal) = runtime_principal {
        runtime_app.owner_principal_id = principal;
        runtime_app.resolved_owner_user_id = None;
    }
    let session = find_or_create_session(
        &state,
        &app,
        Some(&runtime_app),
        endpoint_internal_id,
        &channel_config,
        &routing_tags,
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
        .map_err(|r| too_many_requests(&r.report("ag_ui", app.org_id, &session.session.id)))?;

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

    let trigger_image_parts = ag_ui_image_content_parts(
        &state,
        &app,
        req.forwarded_props.as_ref().unwrap_or(&Value::Null),
    )
    .await
    .map_err(|err| *err)?;
    let mut trigger_parts = vec![InputContentPart::text(trigger_content)];
    trigger_parts.extend(trigger_image_parts);

    let message = state
        .message_service
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: runtime_principal,
                org_id: app.org_id,
                user_id: None,
                harness_id: app.harness_id.uuid(),
                agent_id: Some(app.agent_internal_id),
                session_id: session.session.id.uuid(),
                event_metadata: Some(execution_metadata::app_message_metadata(
                    app.public_id,
                    app.owner_principal_id,
                    app.virtual_user_id,
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
        AgUiEvent::RunStarted(
            AgUiRunStartedEvent::new(thread_id.to_string(), run_id.to_string())
                .with_protocol_version(),
        ),
        AgUiEvent::MessagesSnapshot(AgUiMessagesSnapshotEvent {
            messages: snapshot_messages
                .iter()
                .filter_map(to_ag_ui_message)
                .collect::<Vec<_>>(),
            ..AgUiMessagesSnapshotEvent::default()
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
        reasoning: ReasoningState::default(),
        tool_visibility: channel_config.tool_visibility,
        generic_tool_text: channel_config.generic_tool_text.clone(),
        reasoning_summary_visible: channel_config.reasoning_summary_visible,
        active_tool_activity_count: 0,
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
    app: &crate::api::app_ingress::IngressContext,
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

fn ag_ui_image_metadata(app: &crate::api::app_ingress::IngressContext) -> Value {
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
    app: &crate::api::app_ingress::IngressContext,
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
    app: &crate::api::app_ingress::IngressContext,
    runtime_app: Option<&crate::api::app_ingress::IngressContext>,
    endpoint_internal_id: uuid::Uuid,
    config: &AgUiChannelConfig,
    routing_tags: &[String],
    req: &AgUiRunAgentInput,
) -> Result<SessionResolution, SessionError> {
    let org_row = state
        .db
        .get_organization(app.org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Organization not found for app"))?;
    let org_public_id = org_row.public_id;

    let mut existing = app
        .find_session_by_tags(&state.db, endpoint_internal_id, routing_tags)
        .await?;

    if existing.is_none()
        && let Some(runtime) = runtime_app
    {
        existing = runtime
            .find_session_by_tags(&state.db, endpoint_internal_id, routing_tags)
            .await?;
    }
    let app = runtime_app.unwrap_or(app);
    match existing {
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
            let title = format!("AG-UI thread {}", req.thread_id);
            let session = state
                .session_service
                .create_from_app(
                    &Caller::internal(app.org_id),
                    app.harness_id.uuid(),
                    Some(app.agent_internal_id),
                    app.agent_id,
                    app.historical_app_id,
                    app.agent_version_policy.clone(),
                    app.agent_version_id,
                    Some(endpoint_internal_id),
                    None, // endpoint ingress, not a trigger
                    app.owner_principal_id,
                    app.resolved_owner_user_id,
                    everruns_platform::SessionSource::AgUi,
                    CreateSessionRequest {
                        harness_id: Some(app.harness_id),
                        agent_id: app.agent_id,
                        agent_name: None,
                        virtual_user_id: app.virtual_user_id,
                        title: Some(title),
                        tags: routing_tags.to_vec(),
                        ..Default::default()
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
        // `validate_input_messages` already rejected media parts.
        AgUiMessage::User(message) => (
            RuntimeMessageRole::User,
            message.content.to_text(),
            message.name.clone(),
        ),
        AgUiMessage::Assistant(message) => (
            RuntimeMessageRole::Agent,
            message
                .content
                .clone()
                .or_else(|| {
                    message
                        .tool_calls
                        .as_ref()
                        .map(|calls| format_agui_tool_calls(calls))
                })
                .unwrap_or_default(),
            message.name.clone(),
        ),
        AgUiMessage::Tool(message) => (
            RuntimeMessageRole::Agent,
            {
                let content = message.content.to_text();
                let tool_call_id = &message.tool_call_id;
                match &message.error {
                    Some(error) => format!("[Tool {tool_call_id} error: {error}]\n{content}"),
                    None => format!("[Tool {tool_call_id} result]\n{content}"),
                }
            },
            None,
        ),
        AgUiMessage::System(message) | AgUiMessage::Developer(message) => (
            RuntimeMessageRole::System,
            message.content.clone(),
            message.name.clone(),
        ),
        // Client-materialised UI state that 1.0 clients echo back; never
        // model input.
        AgUiMessage::Activity(_) | AgUiMessage::Reasoning(_) => return None,
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

fn to_ag_ui_message(message: &crate::api::messages::Message) -> Option<AgUiMessage> {
    let id = message.id.uuid().to_string();
    let content = public_content_parts_to_string(&message.content);

    Some(match message.role {
        ApiMessageRole::User => AgUiMessage::User(AgUiUserMessage {
            id,
            content: AgUiMessageContent::Text(content),
            name: None,
            encrypted_value: None,
            metadata: None,
            subagent_run_id: None,
        }),
        ApiMessageRole::Agent => {
            // Replay must agree with the live stream. Live translation drops
            // commentary from the assistant-text channel; emitting it here
            // meant watching a session and reloading it produced different
            // transcripts, with no signal that anything differed.
            if matches!(message.phase, Some(ExecutionPhase::Commentary)) {
                return None;
            }
            AgUiMessage::Assistant(AgUiAssistantMessage {
                id,
                content: (!content.is_empty()).then_some(content),
                ..AgUiAssistantMessage::default()
            })
        }
    })
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
        ContentPart::File(file) => Some(format!(
            "[File: {}]",
            file.filename.as_deref().unwrap_or("unnamed")
        )),
        ContentPart::ToolCall(_) | ContentPart::ToolResult(_) => None,
        // Reasoning belongs to the reasoning channel and is projected there by
        // the reason.* handlers. Rendering it as assistant text would move
        // content across channels, which the projection contract forbids.
        ContentPart::Reasoning(_) => None,
        // `ContentPart` is `#[non_exhaustive]`; this arm is required outside
        // `everruns-core` and is unreachable in-workspace, where every crate
        // shares one core version. A part this build cannot project is omitted
        // rather than rendered as an unknown marker.
        _ => None,
    }
}

fn is_terminal_public_output_message(
    message: &everruns_core::RuntimeMessage,
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
) -> String {
    let projected = streamed_message_id.uuid().to_string();
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
            .push_back(AgUiEvent::TextMessageEnd(AgUiTextMessageEndEvent::new(
                message_id,
            )));
    }
    state.assistant_message_id = None;
    state.assistant_content_started = false;
    state.assistant_emitted_delta = false;
}

struct AgUiStreamState {
    subscription: Box<crate::event_delivery::EventSubscription>,
    session_id: Uuid,
    input_message_id: String,
    thread_id: Uuid,
    run_id: Uuid,
    queue: VecDeque<AgUiEvent>,
    assistant_message_id: Option<String>,
    assistant_content_started: bool,
    assistant_emitted_delta: bool,
    reasoning: ReasoningState,
    tool_visibility: PublicToolVisibility,
    generic_tool_text: String,
    reasoning_summary_visible: bool,
    active_tool_activity_count: usize,
    finished: bool,
}

impl AgUiStreamState {
    /// The text this stream may show for tool activity, or `None` when the
    /// channel's visibility forbids exposing it.
    fn public_tool_activity_text(&self) -> Option<&str> {
        public_tool_activity_text(self.tool_visibility, &self.generic_tool_text)
    }
}

fn push_text_message_start(state: &mut AgUiStreamState, message_id: String) {
    state.queue.push_back(AgUiEvent::TextMessageStart(
        AgUiTextMessageStartEvent::assistant(message_id),
    ));
}

/// Emit `RUN_FINISHED` and end the stream. 1.0 clients reject `RUN_FINISHED`
/// while a text or reasoning message is still open, so close those first.
fn finish_run(state: &mut AgUiStreamState) {
    close_assistant_text_without_finishing(state);
    reasoning::close_open_reasoning(state);
    state
        .queue
        .push_back(AgUiEvent::RunFinished(AgUiRunFinishedEvent::new(
            state.thread_id.to_string(),
            state.run_id.to_string(),
        )));
    state.finished = true;
}

fn translate_event(state: &mut AgUiStreamState, event: &everruns_core::Event) {
    match event.event_type.as_str() {
        "output.message.delta" => {
            if let Ok(data) = parse_event_data::<OutputMessageDeltaData>(event) {
                let message_id = ensure_assistant_message_id(state, data.message_id);
                if !state.assistant_content_started {
                    push_text_message_start(state, message_id.clone());
                    state.assistant_content_started = true;
                }
                state.queue.push_back(AgUiEvent::TextMessageContent(
                    AgUiTextMessageContentEvent::new(message_id, data.delta),
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
                    push_text_message_start(state, message_id.clone());
                }
                if !state.assistant_emitted_delta && !public_text.is_empty() {
                    state.queue.push_back(AgUiEvent::TextMessageContent(
                        AgUiTextMessageContentEvent::new(message_id.clone(), public_text),
                    ));
                }
                state
                    .queue
                    .push_back(AgUiEvent::TextMessageEnd(AgUiTextMessageEndEvent::new(
                        message_id.clone(),
                    )));
                state.assistant_content_started = false;
                state.assistant_emitted_delta = false;
                finish_run(state);
                state.assistant_message_id = Some(message_id);
            }
        }
        "reason.thinking.started"
            if state.reasoning_summary_visible
                && parse_event_data::<ReasonThinkingStartedData>(event).is_ok() =>
        {
            reasoning::thinking_started(state);
        }
        "reason.thinking.delta" => {
            if state.reasoning_summary_visible
                && let Ok(data) = parse_event_data::<ReasonThinkingDeltaData>(event)
            {
                reasoning::thinking_delta(state, data.delta);
            }
        }
        "reason.thinking.completed"
            if state.reasoning_summary_visible
                && parse_event_data::<ReasonThinkingCompletedData>(event).is_ok() =>
        {
            reasoning::thinking_completed(state);
        }
        // EVE-775: a provider `reason.item` summary is a *reasoning artifact*
        // (EVE-768 design note), so when channel policy opts in it renders on
        // the AG-UI reasoning channel (`REASONING_*`) — never on the
        // assistant-text channel. Opaque `encrypted_content` is never emitted.
        "reason.item" => {
            if state.reasoning_summary_visible
                && let Ok(data) = parse_event_data::<ReasonItemData>(event)
            {
                reasoning::push_reasoning_summary(state, &data.summary);
            }
        }
        "tool.started" if parse_event_data::<ToolStartedData>(event).is_ok() => {
            state.active_tool_activity_count += 1;
            // `None` shows nothing; Generic and Narrated both emit the safe
            // channel-configured text, never backend/model-authored narration,
            // which may derive from raw tool-call arguments. See
            // `everruns_platform::exposure::public_tool_activity_text`.
            if let Some(text) = state.public_tool_activity_text() {
                let text = text.to_string();
                reasoning::push_tool_activity_start(state, text);
            }
        }
        "tool.completed" if parse_event_data::<ToolCompletedData>(event).is_ok() => {
            state.active_tool_activity_count = state.active_tool_activity_count.saturating_sub(1);
            reasoning::push_tool_activity_end(state);
        }
        // Cancellation (`turn.cancelled`) is a deliberate terminal state
        // (typically client-initiated), not a server fault. Emit RUN_FINISHED so
        // AG-UI clients see a clean end rather than a misleading internal_error.
        // The cancellation reason lives in the internal session events.
        "turn.completed" | "session.idled" | "turn.cancelled" if !state.finished => {
            finish_run(state);
        }
        "turn.failed" => {
            // AG-UI is a public channel — see knowledge/execution/public-endpoints.md.
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

/// Adapt a sanitized `PublicError` into an AG-UI `RunError` event. All public
/// error emission on this endpoint must go through here so the contract from
/// `knowledge/execution/public-endpoints.md` is enforced in one place.
fn public_run_error_event(error: PublicError) -> AgUiEvent {
    AgUiEvent::RunError(
        AgUiRunErrorEvent::new(error.message.to_string()).with_code(error.code.as_str()),
    )
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
/// roles, carry non-text user content parts, or reuse a message id.
///
/// `Message` deserialises every role the 1.0 protocol defines, so without this
/// gate a public AG-UI client could populate `system`, `developer`, or `tool`
/// messages and have them flow into the LLM context alongside the agent's real
/// system prompt. We refuse such requests with a generic 400 `invalid_request`
/// and never echo the offending role. `reasoning` and `activity` messages are
/// client-materialised UI state that 1.0 clients echo back; they are accepted
/// and dropped, never forwarded to the model.
fn validate_input_messages(messages: &[AgUiMessage]) -> Result<(), Box<Response>> {
    use std::collections::HashSet;
    let mut seen_ids: HashSet<&str> = HashSet::with_capacity(messages.len());
    for message in messages {
        match message {
            AgUiMessage::User(message) => {
                user_text(&message.content).map_err(Box::new)?;
            }
            AgUiMessage::Assistant(_) | AgUiMessage::Activity(_) | AgUiMessage::Reasoning(_) => {}
            AgUiMessage::System(_) | AgUiMessage::Developer(_) | AgUiMessage::Tool(_) => {
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

/// User message text. 1.0 allows media content parts; this endpoint takes
/// images through the image-upload route plus `forwardedProps.imageIds`.
/// Silently dropping media would change what the user asked, so it is refused.
fn user_text(content: &AgUiMessageContent) -> Result<String, Response> {
    if content.has_media() {
        return Err(bad_request(
            "unsupported content part; upload images and pass forwardedProps.imageIds",
        ));
    }
    Ok(content.to_text())
}

fn parse_ag_ui_uuid(value: &str, field: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(value).map_err(|_| bad_request(&format!("{field} must be a UUID")))
}

fn bad_request(message: &str) -> Response {
    ErrorResponse::new(message.to_string())
        .into_response(StatusCode::BAD_REQUEST)
        .into_response()
}
fn conflict(message: &str) -> Response {
    ErrorResponse::new(message.to_string())
        .into_response(StatusCode::CONFLICT)
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
#[path = "ag_ui_tests.rs"]
mod tests;

mod runtime_identity;
use runtime_identity::authorize_ag_ui_request;
pub(crate) use runtime_identity::{resolve_ingress_identity, runtime_endpoint_account};
mod reasoning;
#[cfg(test)]
mod wire_tests;
