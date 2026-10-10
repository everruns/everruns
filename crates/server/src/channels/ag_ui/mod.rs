// AG-UI app channel — public streaming endpoint
//
// Design Decision: AG-UI ingress is keyed by channel ID at
// `POST /v1/channels/{channel_id}/ag-ui`. The app-scoped route remains a permanent
// alias when the App has exactly one enabled AG-UI channel.
//
// Design Decision: The endpoint is public. Requests are accepted
// without user API auth when the app is published and an enabled AG-UI channel
// is present, but a channel may require its own shared bearer token.
//
// Design Decision: The AG-UI stream is translated from Everruns session events
// instead of bypassing the durable runtime. This keeps app-channel behavior
// aligned with normal sessions and preserves streaming parity.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use crate::domains::agent_channels::record::exposure::public_tool_activity_text;
use crate::domains::agent_channels::record::{AgUiChannelConfig, ChannelType};
use crate::kernel_imports::{Caller, ContentPart, ExternalActor, RuntimeMessageRole};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, FromRequest, Path, Request, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use axum_extra::extract::Multipart;
use everruns_contracts::typed_id::ImageId;
#[cfg(test)]
use everruns_contracts::user_facing_error::codes as user_facing_error_codes;
use everruns_core::ag_ui::projection::{
    ProjectionPolicy, Projector, TurnFailure, latest_todos, public_text,
};
use everruns_core::ag_ui::{
    AssistantMessage as AgUiAssistantMessage, Event as AgUiEvent, Message as AgUiMessage,
    MessagesSnapshotEvent as AgUiMessagesSnapshotEvent, PROTOCOL_VERSION,
    RunAgentInput as AgUiRunAgentInput, RunErrorEvent as AgUiRunErrorEvent,
    RunStartedEvent as AgUiRunStartedEvent, ToolCall as AgUiToolCall,
};
use everruns_core::message_retriever::InputMessage as StoredInputMessage;
use futures::{
    StreamExt,
    stream::{self, Stream},
};
use serde_json::Value;
use uuid::Uuid;

use crate::api::channel_auth::{ChannelAuthError, ChannelAuthVerifier, LegacyChannelAuth};
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
use crate::channels::ag_ui::interrupts::{ResumeError, ResumeOutcome};
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::execution_metadata;
use crate::middleware::RequestId;
use crate::security::constant_time_eq;
use crate::services::EventService;
use crate::storage::{CreateImageRow, DbMessageRetriever, EncryptionService, StorageBackend};

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
    pub auth_verifier: ChannelAuthVerifier,
    pub public_chat_enabled: bool,
    pub runtime_auth: Option<crate::auth::AuthState>,
}

impl AgUiState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_core::host::TurnBackend>,
        notifications_enabled: bool,
        event_delivery: crate::live_updates::event_delivery::EventDelivery,
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
            auth_verifier: ChannelAuthVerifier::new(),
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
        .route("/v1/channels/{channel_id}/ag-ui", post(run_agent_channel))
        .route("/v1/e/{channel_id}/ag-ui", post(run_agent_channel))
        .route(
            "/v1/channels/{channel_id}/ag-ui/capabilities",
            get(capabilities_channel),
        )
        .route(
            "/v1/e/{channel_id}/ag-ui/capabilities",
            get(capabilities_channel),
        )
        .route(
            "/v1/channels/{channel_id}/ag-ui/images",
            post(upload_image_channel).layer(DefaultBodyLimit::max(
                MAX_PUBLIC_AG_UI_IMAGE_SIZE + 1024 * 1024,
            )),
        )
        .route(
            "/v1/e/{channel_id}/ag-ui/images",
            post(upload_image_channel).layer(DefaultBodyLimit::max(
                MAX_PUBLIC_AG_UI_IMAGE_SIZE + 1024 * 1024,
            )),
        )
        .with_state(state)
}
enum AgUiTarget {
    LegacyApp(String),
    Channel(String),
}

struct AuthorizedAgUiRequest {
    context: crate::api::channel_ingress::IngressContext,
    channel_id: String,
    /// Internal id of the endpoint this request arrived through, recorded on
    /// any session it creates (EVE-1004).
    channel_internal_id: uuid::Uuid,
    channel_config: AgUiChannelConfig,
    runtime_user: Option<everruns_contracts::typed_id::VirtualUserId>,
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

async fn upload_image_channel(
    State(state): State<AgUiState>,
    Path(channel_id): Path<String>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<(StatusCode, Json<ImageUploadResponse>), Response> {
    upload_image(
        state,
        AgUiTarget::Channel(channel_id),
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

async fn run_agent_channel(
    State(state): State<AgUiState>,
    Path(channel_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    request: Request,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, Response> {
    run_agent(
        state,
        AgUiTarget::Channel(channel_id),
        req_id,
        connect_info,
        headers,
        request,
    )
    .await
}

/// The endpoint's AG-UI 1.0 `AgentCapabilities`, behind the same auth, gates
/// and rate limit as a run.
async fn capabilities_channel(
    State(state): State<AgUiState>,
    Path(channel_id): Path<String>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
) -> Result<Json<everruns_core::ag_ui::AgentCapabilities>, Response> {
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let AuthorizedAgUiRequest {
        context,
        channel_config,
        ..
    } = authorize_ag_ui_request(&state, AgUiTarget::Channel(channel_id), &headers, peer_addr)
        .await?;
    Ok(Json(crate::channels::ag_ui::capabilities::capabilities(
        &context.name,
        context.description.as_deref(),
        &channel_config,
    )))
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
        channel_internal_id,
        channel_config,
        runtime_user,
    } = authorize_ag_ui_request(&state, target, &headers, peer_addr).await?;

    run_app_agent_stream(
        state,
        app,
        channel_internal_id,
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
    app: crate::api::channel_ingress::IngressContext,
    channel_internal_id: uuid::Uuid,
    channel_config: AgUiChannelConfig,
    tag_prefix: &str,
    extra_routing_tags: Vec<String>,
    runtime_user: Option<everruns_contracts::typed_id::VirtualUserId>,
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
    // Mitigation: Reject system/developer/activity/reasoning roles at the
    // runtime trust boundary, use a tool message only as the result of a
    // parked frontend tool call, and reject duplicate message IDs. The error is a generic
    // `invalid_request` so we don't echo the offending role back.
    validate_input_messages(&req.messages).map_err(|err| *err)?;
    // THREAT[TM-TENANT-009]: 1.0 ids are free-form strings, and the thread id
    // becomes part of the session routing tags. Bound them before use.
    if !is_valid_ag_ui_id(&req.thread_id) || !is_valid_ag_ui_id(&req.run_id) {
        return Err(bad_request("invalid_request"));
    }

    let frontend_tools = crate::channels::ag_ui::frontend_tools::definitions(&req.tools)
        .map_err(|message| bad_request(&message))?;
    let frontend_names: std::collections::HashSet<String> =
        req.tools.iter().map(|tool| tool.name.clone()).collect();
    let tool_results = crate::channels::ag_ui::frontend_tools::trailing_results(&req.messages);

    // A resuming run (AG-UI 1.0) answers the interrupts or frontend tool calls
    // that ended the last one and starts no new turn, so it needs no trailing
    // user message.
    let resuming = !req.resume.is_empty() || !tool_results.is_empty();
    let trigger = if resuming {
        None
    } else {
        let trigger_message = req
            .messages
            .last()
            .ok_or_else(|| bad_request("messages must contain at least one user message"))?;
        match trigger_message {
            AgUiMessage::User(message) => Some((message.content.to_text(), message.name.clone())),
            _ => return Err(bad_request("the final AG-UI message must have role=user")),
        }
    };

    let thread_id = req.thread_id.clone();
    let run_id = req.run_id.clone();
    let thread_tag = thread_id.clone();
    let run_tag = run_id.clone();

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
        let principals = crate::domains::users::PrincipalService::new(state.db.clone());
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
        channel_internal_id,
        &channel_config,
        &routing_tags,
        &req,
    )
    .await
    .map_err(SessionError::into_response)?;
    // A consumer sends its frontend tools on every run; the turn this run
    // starts or resumes sees the current set.
    let tools_value =
        serde_json::to_value(&frontend_tools).map_err(|e| internal_error(e.into()))?;
    if serde_json::to_value(&session.session.tools).ok() != Some(tools_value.clone()) {
        state
            .db
            .update_session(
                app.org_id,
                session.session.id,
                crate::storage::UpdateSession {
                    tools: Some(tools_value),
                    ..Default::default()
                },
            )
            .await
            .map_err(internal_error)?;
    }
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
    if session.is_new && !resuming {
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

    let (input_message_id, resume_outcome) = if let Some((trigger_content, trigger_name)) = trigger
    {
        let trigger_image_parts =
            ag_ui_image_content_parts(&state, &app, req.forwarded_props.as_ref())
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
                    event_metadata: Some(execution_metadata::channel_message_metadata(
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
                    client_message_id: None,
                },
            )
            .await
            .map_err(internal_error)?;
        (Some(message.id.to_string()), None)
    } else {
        // Subscribed above, so the resumed turn's events cannot be missed.
        let services = crate::channels::ag_ui::interrupts::ResumeServices {
            db: &state.db,
            session_service: state.session_service.as_ref(),
            event_service: state.event_service.as_ref(),
            runner: state.message_service.runner().clone(),
        };
        let outcome = if req.resume.is_empty() {
            crate::channels::ag_ui::frontend_tools::submit_results(
                &services,
                app.org_id,
                &session.session,
                &frontend_names,
                tool_results,
            )
            .await
        } else {
            crate::channels::ag_ui::interrupts::resume(
                &services,
                app.org_id,
                &session.session,
                &channel_config,
                &req.resume,
            )
            .await
        }
        .map_err(resume_error_response)?;
        let input_message_id = match &outcome {
            ResumeOutcome::Resumed { input_message_id } => input_message_id.clone(),
            ResumeOutcome::NothingParked | ResumeOutcome::StillOpen { .. } => None,
        };
        (input_message_id, Some(outcome))
    };

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
        message_id = input_message_id.as_deref().unwrap_or_default(),
        resuming,
        is_new_session = session.is_new,
        session_resolved_ms = session_resolved_ms as u64,
        ag_ui_handler_ms = ag_ui_handler_ms as u64,
        "AG-UI ingress complete; durable turn workflow start scheduled"
    );

    let mut policy = public_projection_policy(&channel_config);
    // THREAT[TM-API-026]: the session id is an internal handle; only an
    // identified caller on the AG-UI channel itself is told it, never an
    // anonymous or Public Chat visitor.
    if runtime_user.is_some() && tag_prefix == "ag_ui" {
        policy.session_id = Some(session.session.id.to_string());
    }
    let mut projector = Projector::new(thread_id.clone(), run_id.clone(), policy);
    // Shared state starts from the session's todo list, so a client that
    // reconnects holds what one that followed along does. Queued now, it
    // streams right after the initial events and before the turn's.
    if let Some(todos) = latest_todos(
        snapshot_messages
            .iter()
            .map(|message| message.content.as_slice()),
    ) {
        projector.restore_todos(todos);
    }
    // Version negotiation: a 1.0 consumer declares `protocolVersion`; answer
    // with ours. A pre-versioning consumer gets no field it might reject.
    let mut run_started = AgUiRunStartedEvent::new(thread_id, run_id);
    if req.protocol_version.is_some() {
        run_started.protocol_version = Some(PROTOCOL_VERSION.to_string());
    }
    run_started.base.metadata = projector.run_metadata();
    let initial_events = vec![
        AgUiEvent::RunStarted(run_started),
        AgUiEvent::MessagesSnapshot(AgUiMessagesSnapshotEvent {
            messages: snapshot_messages
                .iter()
                .filter_map(to_ag_ui_message)
                .collect::<Vec<_>>(),
            ..Default::default()
        }),
    ];

    match resume_outcome {
        // The entries answered nothing and no turn runs: an empty run.
        Some(ResumeOutcome::NothingParked) => {
            projector.project("turn.completed", &Value::Null);
        }
        // An interrupt without an entry is not abandoned (AG-UI 1.0): it stays
        // open and the run ends asking again, as does an unanswered frontend
        // tool call.
        Some(ResumeOutcome::StillOpen {
            tool_calls,
            interrupts,
        }) => projector.park(tool_calls, interrupts),
        Some(ResumeOutcome::Resumed { .. }) | None => {}
    }
    let stream_state = AgUiStreamState {
        subscription: Box::new(subscription),
        session_id: session.session.id.uuid(),
        input_message_id,
        projector,
        config: channel_config,
        frontend_tools: frontend_names,
    };

    let initial_stream = stream::iter(initial_events.into_iter().map(|event| Ok(agui_sse(&event))));
    let translated_stream = stream::unfold(stream_state, |mut state| async move {
        loop {
            if let Some(event) = state.projector.pop() {
                return Some((Ok(agui_sse(&event)), state));
            }

            if state.projector.is_finished() {
                return None;
            }

            let Some(event) = state.subscription.recv().await else {
                state
                    .projector
                    .fail(public_run_error_event(PublicError::fallback()));
                continue;
            };

            if event.session_id.uuid() != state.session_id {
                continue;
            }

            if let Some(wanted) = &state.input_message_id
                && event
                    .context
                    .input_message_id
                    .as_ref()
                    .map(|id| id.to_string())
                    .as_deref()
                    != Some(wanted.as_str())
            {
                continue;
            }

            translate_event(
                &mut state.projector,
                &state.config,
                &state.frontend_tools,
                &event,
            );
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
    app: &crate::api::channel_ingress::IngressContext,
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

fn ag_ui_image_metadata(app: &crate::api::channel_ingress::IngressContext) -> Value {
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
    app: &crate::api::channel_ingress::IngressContext,
    forwarded_props: Option<&Value>,
) -> Result<Vec<InputContentPart>, Box<Response>> {
    let Some(forwarded_props) = forwarded_props else {
        return Ok(vec![]);
    };
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
    session: crate::domains::sessions::record::Session,
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
    app: &crate::api::channel_ingress::IngressContext,
    runtime_app: Option<&crate::api::channel_ingress::IngressContext>,
    channel_internal_id: uuid::Uuid,
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
        .find_session_by_tags(&state.db, channel_internal_id, routing_tags)
        .await?;

    if existing.is_none()
        && let Some(runtime) = runtime_app
    {
        existing = runtime
            .find_session_by_tags(&state.db, channel_internal_id, routing_tags)
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
                Some(crate::setup::org_init::base_harness_id(&state.db, app.org_id).await?)
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
                    Some(channel_internal_id),
                    None, // channel ingress, not a trigger
                    app.owner_principal_id,
                    app.resolved_owner_user_id,
                    crate::domains::sessions::record::SessionSource::AgUi,
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
                .or_else(|| message.tool_calls.as_deref().map(format_agui_tool_calls))
                .unwrap_or_default(),
            message.name.clone(),
        ),
        AgUiMessage::System(message) | AgUiMessage::Developer(message) => (
            RuntimeMessageRole::System,
            message.content.clone(),
            message.name.clone(),
        ),
        // Activity and reasoning messages are UI state, not conversation. A
        // tool message is only ever a frontend tool result for a parked call
        // (TM-LLM-020), never history.
        AgUiMessage::Tool(_) | AgUiMessage::Activity(_) | AgUiMessage::Reasoning(_) => {
            return None;
        }
    };

    let mut stored = StoredInputMessage {
        external_actor: None,
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
    let content = public_text(&message.content);

    Some(match message.role {
        ApiMessageRole::User => AgUiMessage::user(id, content),
        ApiMessageRole::Agent => {
            // Replay must agree with the live stream. Live translation drops
            // commentary from the assistant-text channel; emitting it here
            // meant watching a session and reloading it produced different
            // transcripts, with no signal that anything differed.
            if everruns_core::conversation::is_commentary(message.phase) {
                return None;
            }
            AgUiMessage::Assistant(AgUiAssistantMessage {
                id,
                content: (!content.is_empty()).then_some(content),
                ..Default::default()
            })
        }
    })
}

struct AgUiStreamState {
    subscription: Box<crate::live_updates::event_delivery::EventSubscription>,
    session_id: Uuid,
    /// Events of other turns are skipped. `None` follows the whole session,
    /// for a resumed turn whose parked event recorded no input message.
    input_message_id: Option<String>,
    projector: Projector,
    config: AgUiChannelConfig,
    /// This run's frontend tool names: parked calls to them stream to the
    /// consumer instead of waiting on someone else.
    frontend_tools: std::collections::HashSet<String>,
}

/// The public channel's projection: reasoning only when the channel opts in,
/// tool activity as the channel's fixed text (never tool names or arguments),
/// and failures through the shared `PublicError` sanitizer so internal codes,
/// provider strings, model ids and quota state never reach the wire.
fn public_projection_policy(config: &AgUiChannelConfig) -> ProjectionPolicy {
    ProjectionPolicy {
        reasoning_visible: config.reasoning_summary_visible,
        tool_activity_text: public_tool_activity_text(
            config.tool_visibility,
            &config.generic_tool_text,
        )
        .map(str::to_string),
        usage_visible: config.usage_visible,
        subagents_visible: config.subagents_visible,
        // The model rides with usage, which already names it.
        model_visible: config.usage_visible,
        state_visible: config.state_visible,
        session_id: None,
        error: std::sync::Arc::new(|failure: &TurnFailure| {
            public_run_error(PublicError::from_internal_code(failure.code.as_deref()))
        }),
    }
}

fn translate_event(
    projector: &mut Projector,
    config: &AgUiChannelConfig,
    frontend_tools: &std::collections::HashSet<String>,
    event: &everruns_core::Event,
) {
    if let Some(turn_id) = event.context.turn_id {
        projector.observe_turn(turn_id.to_string());
    }
    // A turn that parks on a question or an approval ends the run with the
    // interrupt outcome (AG-UI 1.0); one that parks on frontend tool calls
    // streams them and ends in success. The next run answers either.
    if let everruns_core::events::EventData::ToolCallRequested(requested) = &event.data {
        let parked = crate::channels::ag_ui::interrupts::ParkedCalls::from_request(requested);
        projector.park(
            crate::channels::ag_ui::frontend_tools::pending_calls(
                requested,
                &parked,
                frontend_tools,
            ),
            parked.interrupts(config),
        );
        return;
    }
    match serde_json::to_value(&event.data) {
        Ok(data) => projector.project(&event.event_type, &data),
        Err(err) => tracing::warn!(error = %err, "AG-UI: unserializable event data"),
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
/// error emission on this endpoint must go through here so sanitization is
/// enforced in one place.
fn public_run_error_event(error: PublicError) -> AgUiRunErrorEvent {
    public_run_error(error)
}

fn public_run_error(error: PublicError) -> AgUiRunErrorEvent {
    AgUiRunErrorEvent::new(error.message).with_code(error.code.as_str())
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

fn resume_error_response(error: ResumeError) -> Response {
    match error {
        ResumeError::Invalid(detail) => bad_request(&detail),
        ResumeError::Conflict(detail) => conflict(&detail),
        ResumeError::Internal(err) => internal_error(err),
    }
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
    let mut seen_ids: HashSet<&str> = HashSet::with_capacity(messages.len());
    for message in messages {
        match message {
            // Tool messages are frontend tool results; only trailing ones that
            // answer a parked call are used (see `frontend_tools`).
            AgUiMessage::User(_) | AgUiMessage::Assistant(_) | AgUiMessage::Tool(_) => {}
            AgUiMessage::System(_)
            | AgUiMessage::Developer(_)
            | AgUiMessage::Activity(_)
            | AgUiMessage::Reasoning(_) => {
                tracing::warn!("AG-UI request rejected: disallowed message role");
                return Err(Box::new(bad_request("invalid_request")));
            }
        }
        if !is_valid_ag_ui_id(message.id()) || !seen_ids.insert(message.id()) {
            tracing::warn!("AG-UI request rejected: duplicate message id");
            return Err(Box::new(bad_request("invalid_request")));
        }
    }
    Ok(())
}

/// AG-UI 1.0 ids are free-form strings. Accept the shapes clients generate
/// (UUIDs, nanoids, prefixed ids) and nothing that could smuggle separators
/// or control characters into routing tags or logs.
fn is_valid_ag_ui_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
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

fn ag_ui_auth_error_response(error: ChannelAuthError) -> Response {
    match error {
        ChannelAuthError::Unauthorized => unauthorized(),
        ChannelAuthError::Misconfigured => forbidden("AG-UI auth is misconfigured"),
        ChannelAuthError::ProviderUnavailable => {
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
mod tests;

mod runtime_identity;

pub(crate) mod capabilities;
pub(crate) mod frontend_tools;
pub(crate) mod interrupts;
use runtime_identity::authorize_ag_ui_request;
pub(crate) use runtime_identity::{resolve_ingress_identity, runtime_channel_account};
