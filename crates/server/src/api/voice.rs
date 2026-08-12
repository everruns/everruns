// Realtime voice session endpoints.
//
// Security boundary: client SDP, ephemeral provider secrets, and raw sideband
// payloads are never persisted or logged. Durable state stores only sanitized
// lifecycle metadata plus text transcripts marked with metadata.source=voice.

use crate::api::common::{ErrorResponse, impl_auth_state};
use crate::api::messages::{InputMessage, MessageRole};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::messages::{CreateMessage, MessageService};
use crate::domains::sessions::{CreateSession, GetOrCreateChatSession, SessionService};
use crate::event_delivery::EventDelivery;
use crate::services::{
    EventService, ProviderResolverService,
    provider_resolver::{ResolvedProviderCredentials, ResolvedServiceProvider},
};
use crate::storage::{DbLeasedResourceStore, DbSessionResourceRegistry, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    routing::post,
};
use everruns_core::events::{
    EventContext, EventData, EventRequest, VOICE_INPUT_TRANSCRIPT_COMPLETED,
    VOICE_INPUT_TRANSCRIPT_DELTA, VOICE_OUTPUT_TRANSCRIPT_COMPLETED, VOICE_OUTPUT_TRANSCRIPT_DELTA,
    VoiceSessionEndedData, VoiceSessionFailedData, VoiceSessionStartedData, VoiceTranscriptData,
};
use everruns_core::message::ExecutionPhase;
use everruns_core::traits::LeasedResourceStore;
use everruns_core::typed_id::{AgentId, MessageId, SessionId};
use everruns_core::{
    Caller, ContentPart, Event, InputContentPart, LeasedResource, ServiceKind, ToolCall,
    UpsertLeasedResource,
};
use everruns_platform::FeatureFlags;
use futures_util::{SinkExt, StreamExt};
use reqwest::multipart;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message as WsMessage, client::IntoClientRequest, http::HeaderValue},
};
use utoipa::ToSchema;
use uuid::Uuid;

const VOICE_RESOURCE_TYPE: &str = "voice_connection";
/// Provider label reported in voice responses. Realtime is OpenAI-only in OSS;
/// the provider connection itself is selected by service-kind resolution.
const OPENAI_PROVIDER: &str = "openai";
const DEFAULT_MODEL: &str = "gpt-realtime-2";
const DEFAULT_VOICE: &str = "marin";
const DEFAULT_REASONING_EFFORT: &str = "low";
const LEASE_SECONDS: u32 = 15 * 60;
const PROVIDER_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

static VOICE_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: EventService,
    pub auth: AuthState,
    pub provider_resolver: Arc<ProviderResolverService>,
    pub leased_resource_store: Arc<dyn LeasedResourceStore>,
    pub feature_flags: FeatureFlags,
    pub runner: Arc<dyn everruns_worker::AgentRunner>,
    pub fallback_default_harness_name: Option<String>,
    pub chat_harness_name: Option<String>,
    pub chat_session_title: Option<String>,
}

pub struct AppDependencies {
    pub runner: Arc<dyn everruns_worker::AgentRunner>,
    pub message_service: Arc<MessageService>,
    pub provider_resolver: Arc<ProviderResolverService>,
    pub event_delivery: EventDelivery,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        auth: AuthState,
        feature_flags: FeatureFlags,
        dependencies: AppDependencies,
        host_composition: &everruns_host::HostComposition,
        built_in_harnesses: &[everruns_platform::BuiltInHarnessDefinition],
    ) -> Self {
        let registry = Arc::new(DbSessionResourceRegistry::new(db.clone()));
        let leased_resource_store =
            Arc::new(DbLeasedResourceStore::new(db.clone()).with_registry(registry))
                as Arc<dyn LeasedResourceStore>;
        Self {
            session_service: Arc::new(SessionService::with_registry(
                db.clone(),
                host_composition.capability_registry().clone(),
            )),
            message_service: dependencies.message_service,
            event_service: EventService::new(db.clone(), dependencies.event_delivery),
            db,
            auth,
            provider_resolver: dependencies.provider_resolver,
            leased_resource_store,
            feature_flags,
            runner: dependencies.runner,
            fallback_default_harness_name: everruns_platform::harness_for_role(
                built_in_harnesses,
                everruns_platform::BuiltInHarnessRole::Default,
            )
            .map(|h| h.name.clone()),
            chat_harness_name: everruns_platform::harness_for_role(
                built_in_harnesses,
                everruns_platform::BuiltInHarnessRole::Chat,
            )
            .map(|h| h.name.clone()),
            chat_session_title: everruns_platform::harness_for_role(
                built_in_harnesses,
                everruns_platform::BuiltInHarnessRole::Chat,
            )
            .map(|h| h.display_name.clone()),
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_session_service(self.session_service.clone())
        .with_event_service(Arc::new(self.event_service.clone()))
        .with_runner(self.runner.clone())
        .with_message_service(self.message_service.clone())
        .with_fallback_harness_name(self.fallback_default_harness_name.clone())
        .with_chat_harness_name(self.chat_harness_name.clone())
        .with_chat_session_title(self.chat_session_title.clone())
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/sessions/{session_id}/voice/client-secret",
            post(create_client_secret),
        )
        .route("/v1/sessions/{session_id}/voice/calls", post(create_call))
        .route(
            "/v1/sessions/{session_id}/voice/{voice_connection_id}/attach",
            post(attach_call),
        )
        .route(
            "/v1/sessions/{session_id}/voice/{voice_connection_id}/end",
            post(end_call),
        )
        .route(
            "/v1/agents/{agent_id}/voice/sessions",
            post(create_agent_voice_session),
        )
        .route("/v1/sessions/chat/voice", post(create_chat_voice_session))
        .with_state(state)
}

/// Realtime-session knobs flattened into the voice request bodies that
/// create or attach a realtime connection — `VoiceClientSecretRequest`,
/// `VoiceCallRequest`, and `VoiceAttachRequest`. The `/voice/.../end`
/// endpoint takes `VoiceEndRequest` and does not accept these options.
/// All fields are optional; omitted ones fall back to the agent's or
/// provider's default.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct VoiceSessionOptions {
    /// Provider-side realtime model identifier. When omitted the server picks the agent's configured default.
    #[serde(default)]
    #[schema(example = "gpt-realtime")]
    pub model: Option<String>,
    /// Realtime voice preset (provider-specific). When omitted the server picks the agent's configured default.
    #[serde(default)]
    #[schema(example = "alloy")]
    pub voice: Option<String>,
    /// Reasoning effort hint passed through to the realtime model. One of `low`, `medium`, `high`.
    /// When omitted the server picks the provider's default.
    #[serde(default)]
    #[schema(example = "medium")]
    pub reasoning_effort: Option<String>,
    /// Extra system instructions appended to the realtime session prompt.
    #[serde(default)]
    #[schema(example = "Always confirm before placing an order.")]
    pub instructions: Option<String>,
    /// Realtime provider binding: the prefixed public id of the provider
    /// connection to route this voice connection through (e.g. `prov_…`). Lets
    /// an org with more than one realtime-capable provider pick which one serves
    /// the connection. When omitted, the server resolves the org's default (or
    /// single) realtime provider. The bound provider's driver MUST declare the
    /// realtime service, otherwise the request is rejected with 400.
    #[serde(default)]
    #[schema(example = "prov_01h…")]
    pub provider_id: Option<String>,
}

/// Request body for voice client secret.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct VoiceClientSecretRequest {
    #[serde(flatten)]
    pub options: VoiceSessionOptions,
}

/// Request body for voice call.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct VoiceCallRequest {
    pub sdp: String,
    #[serde(flatten)]
    pub options: VoiceSessionOptions,
}

/// Request body for voice attach.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct VoiceAttachRequest {
    pub provider_call_id: String,
    #[serde(flatten)]
    pub options: VoiceSessionOptions,
}

/// Request body for voice end.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct VoiceEndRequest {
    /// Free-text reason recorded with the session-ended event. Useful for operator forensics.
    #[serde(default)]
    #[schema(example = "User hung up after refund confirmed.")]
    pub reason: Option<String>,
}

/// Response body for voice client secret.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceClientSecretResponse {
    /// Prefixed public identifier of the voice connection. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub voice_connection_id: String,
    /// Realtime provider routing this connection (e.g. `openai`).
    pub provider: String,
    /// Provider-side model identifier used for the realtime session.
    pub model: String,
    /// Realtime voice preset selected for the connection (provider-specific).
    pub voice: String,
    /// Reasoning effort tier for thinking-capable models (`none`, `minimal`, `low`, `medium`, `high`).
    pub reasoning_effort: String,
    /// Timestamp when the client secret expires (RFC 3339). The client must establish the realtime connection before this.
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Provider-specific ephemeral credential payload the client uses to authenticate the realtime connection.
    pub client_secret: Value,
}

/// Response body for voice call.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceCallResponse {
    /// Prefixed public identifier of the voice connection. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub voice_connection_id: String,
    /// Provider-side call identifier once issued. `None` until the realtime call is established.
    pub provider_call_id: Option<String>,
    /// Realtime provider routing this connection.
    pub provider: String,
    /// Provider-side model identifier used for the realtime session.
    pub model: String,
    /// Realtime voice preset selected for the connection.
    pub voice: String,
    /// Reasoning effort tier for thinking-capable models.
    pub reasoning_effort: String,
    /// Timestamp when the call's lease expires (RFC 3339).
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Server-generated SDP answer to send back to the client to complete the WebRTC handshake.
    pub answer_sdp: String,
}

/// Response body for voice attach.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceAttachResponse {
    /// Prefixed public identifier of the voice connection. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub voice_connection_id: String,
    /// Provider-side call identifier of the connected realtime call.
    pub provider_call_id: String,
    /// Realtime provider routing this connection.
    pub provider: String,
    /// Provider-side model identifier used for the realtime session.
    pub model: String,
    /// Realtime voice preset selected for the connection.
    pub voice: String,
    /// Reasoning effort tier for thinking-capable models.
    pub reasoning_effort: String,
    /// Timestamp when the connection's lease expires (RFC 3339).
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Response body for voice end.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceEndResponse {
    /// Prefixed public identifier of the voice connection that was ended.
    pub voice_connection_id: String,
    /// Current lifecycle status.
    pub status: String,
}

/// Generic envelope returned by the agent/chat voice-session endpoints that
/// create-or-attach a session and a voice connection in one round trip.
/// `T` is the per-endpoint voice payload (`VoiceCallResponse`,
/// `VoiceAttachResponse`).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceSessionResponse<T> {
    /// The session this voice connection is attached to. Returned alongside
    /// the voice payload so a caller has a single round-trip view of both.
    pub session: everruns_platform::Session,
    /// Voice connection details — concrete shape depends on the endpoint
    /// (e.g. `VoiceCallResponse` for `/voice/call`, `VoiceAttachResponse`
    /// for `/voice/attach`).
    pub voice: T,
}

#[utoipa::path(
    description = "Create an ephemeral client secret for the voice channel.",
    post,
    path = "/v1/sessions/{session_id}/voice/client-secret",
    request_body = VoiceClientSecretRequest,
    responses((status = 200, description = "Realtime client secret created", body = VoiceClientSecretResponse)),
    tag = "voice"
)]
pub async fn create_client_secret(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<VoiceClientSecretRequest>,
) -> Result<Json<VoiceClientSecretResponse>, (StatusCode, Json<ErrorResponse>)> {
    ensure_voice_enabled(&org)?;
    let session_id = parse_session_id(&session_id)?;
    authorize_session(&state, &org, session_id).await?;
    let binding = req.options.provider_id.clone();
    let normalized = normalize_options(req.options)?;
    let resolved = resolve_realtime_credentials(&state, org.org_id, binding.as_deref()).await?;
    let credentials = resolved.credentials;
    let voice_connection_id = new_voice_connection_id();
    let lease = upsert_voice_resource(VoiceResourceUpsert {
        state: &state,
        session_id,
        org: &org,
        voice_connection_id: &voice_connection_id,
        provider_call_id: None,
        provider_id: &resolved.provider_id,
        options: &normalized,
        transport: "client_secret",
        status: "pending_client",
    })
    .await?;

    let session = realtime_session_payload(&normalized);
    let response = state
        .http_client()
        .post(format!(
            "{}/realtime/client_secrets",
            openai_base_url(&credentials.base_url)
        ))
        .bearer_auth(&credentials.api_key)
        .header(
            "OpenAI-Safety-Identifier",
            safety_identifier(org.org_id, org.user_id, session_id),
        )
        .json(&json!({ "session": session }))
        .send()
        .await
        .map_err(provider_error)?;
    if !response.status().is_success() {
        emit_voice_failed(&state, session_id, &voice_connection_id, "provider_error").await;
        return Err(ErrorResponse::bad_gateway());
    }
    let client_secret = response.json::<Value>().await.map_err(provider_error)?;

    Ok(Json(VoiceClientSecretResponse {
        voice_connection_id,
        provider: OPENAI_PROVIDER.to_string(),
        model: normalized.model,
        voice: normalized.voice,
        reasoning_effort: normalized.reasoning_effort,
        expires_at: lease.lease_expires_at,
        client_secret,
    }))
}

#[utoipa::path(
    description = "Create a voice call attached to the session.",
    post,
    path = "/v1/sessions/{session_id}/voice/calls",
    request_body = VoiceCallRequest,
    responses((status = 200, description = "Realtime WebRTC call created", body = VoiceCallResponse)),
    tag = "voice"
)]
pub async fn create_call(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<VoiceCallRequest>,
) -> Result<Json<VoiceCallResponse>, (StatusCode, Json<ErrorResponse>)> {
    ensure_voice_enabled(&org)?;
    let session_id = parse_session_id(&session_id)?;
    authorize_session(&state, &org, session_id).await?;
    bootstrap_call(&state, &org, session_id, req)
        .await
        .map(Json)
}

#[utoipa::path(
    description = "Attach an external voice call to the session.",
    post,
    path = "/v1/sessions/{session_id}/voice/{voice_connection_id}/attach",
    request_body = VoiceAttachRequest,
    responses((status = 200, description = "Realtime sideband attached", body = VoiceAttachResponse)),
    tag = "voice"
)]
pub async fn attach_call(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, voice_connection_id)): Path<(String, String)>,
    Json(req): Json<VoiceAttachRequest>,
) -> Result<Json<VoiceAttachResponse>, (StatusCode, Json<ErrorResponse>)> {
    ensure_voice_enabled(&org)?;
    let session_id = parse_session_id(&session_id)?;
    authorize_session(&state, &org, session_id).await?;
    if voice_connection_id.trim().is_empty() || req.provider_call_id.trim().is_empty() {
        return Err(
            ErrorResponse::new("Invalid voice connection").into_response(StatusCode::BAD_REQUEST)
        );
    }
    let binding = req.options.provider_id.clone();
    let normalized = normalize_options(req.options)?;
    let resolved = resolve_realtime_credentials(&state, org.org_id, binding.as_deref()).await?;
    let credentials = resolved.credentials;
    let lease = upsert_voice_resource(VoiceResourceUpsert {
        state: &state,
        session_id,
        org: &org,
        voice_connection_id: &voice_connection_id,
        provider_call_id: Some(&req.provider_call_id),
        provider_id: &resolved.provider_id,
        options: &normalized,
        transport: "webrtc",
        status: "active",
    })
    .await?;
    emit_voice_started(
        &state,
        session_id,
        &voice_connection_id,
        &normalized,
        "webrtc",
    )
    .await;
    spawn_sideband(
        state.clone(),
        org.clone(),
        session_id,
        voice_connection_id.clone(),
        req.provider_call_id.clone(),
        credentials,
    );
    Ok(Json(VoiceAttachResponse {
        voice_connection_id,
        provider_call_id: req.provider_call_id,
        provider: OPENAI_PROVIDER.to_string(),
        model: normalized.model,
        voice: normalized.voice,
        reasoning_effort: normalized.reasoning_effort,
        expires_at: lease.lease_expires_at,
    }))
}

#[utoipa::path(
    description = "End the in-flight voice call.",
    post,
    path = "/v1/sessions/{session_id}/voice/{voice_connection_id}/end",
    request_body = VoiceEndRequest,
    responses((status = 200, description = "Realtime voice connection ended", body = VoiceEndResponse)),
    tag = "voice"
)]
pub async fn end_call(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, voice_connection_id)): Path<(String, String)>,
    Json(req): Json<VoiceEndRequest>,
) -> Result<Json<VoiceEndResponse>, (StatusCode, Json<ErrorResponse>)> {
    ensure_voice_enabled(&org)?;
    let session_id = parse_session_id(&session_id)?;
    authorize_session(&state, &org, session_id).await?;
    release_voice_resource(&state, session_id, &voice_connection_id).await?;
    emit_voice_ended(&state, session_id, &voice_connection_id, req.reason, None).await;
    Ok(Json(VoiceEndResponse {
        voice_connection_id,
        status: "ended".to_string(),
    }))
}

#[utoipa::path(
    description = "Create a voice session for a specific agent. Returns connection details for the realtime audio channel.",
    post,
    path = "/v1/agents/{agent_id}/voice/sessions",
    request_body = VoiceCallRequest,
    responses((status = 201, description = "Agent session and realtime call created", body = VoiceSessionResponse<VoiceCallResponse>)),
    tag = "voice"
)]
pub async fn create_agent_voice_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<AgentId>,
    Json(req): Json<VoiceCallRequest>,
) -> Result<
    (StatusCode, Json<VoiceSessionResponse<VoiceCallResponse>>),
    (StatusCode, Json<ErrorResponse>),
> {
    ensure_voice_enabled(&org)?;
    let session = CreateSession(crate::api::sessions::CreateSessionRequest {
        source: None,
        workspace_id: None,
        harness_id: None,
        harness_name: None,
        agent_id: Some(agent_id),
        agent_name: None,
        agent_identity_id: None,
        title: Some("Voice session".to_string()),
        goal: None,
        locale: None,
        tags: vec!["voice".to_string()],
        model_id: None,
        capabilities: Vec::new(),
        tools: Vec::new(),
        mcp_servers: Default::default(),
        system_prompt: None,
        initial_files: Vec::new(),
        hints: Some(HashMap::from([("voice".to_string(), json!(true))])),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        parent_session_id: None,
        forked_from_session_id: None,
        budget_root_session_id: None,
        seed: everruns_core::SessionSeedMode::Fresh,
    })
    .run(&state.ctx(&org))
    .await?;
    let voice = bootstrap_call(&state, &org, session.id, req).await?;
    Ok((
        StatusCode::CREATED,
        Json(VoiceSessionResponse { session, voice }),
    ))
}

#[utoipa::path(
    description = "Create a voice session for the user's Platform Chat. Returns connection details for the realtime audio channel. Requires the `voice` feature flag; returns 404 when disabled.",
    post,
    path = "/v1/sessions/chat/voice",
    request_body = VoiceCallRequest,
    responses(
        (status = 200, description = "Platform chat session and realtime call created", body = VoiceSessionResponse<VoiceCallResponse>),
        (status = 404, description = "Voice is disabled for the org"),
    ),
    tag = "voice"
)]
pub async fn create_chat_voice_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<VoiceCallRequest>,
) -> Result<Json<VoiceSessionResponse<VoiceCallResponse>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_voice_enabled(&org)?;
    let session = GetOrCreateChatSession { locale: None }
        .run(&state.ctx(&org))
        .await?;
    let voice = bootstrap_call(&state, &org, session.id, req).await?;
    Ok(Json(VoiceSessionResponse { session, voice }))
}

async fn bootstrap_call(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: SessionId,
    req: VoiceCallRequest,
) -> Result<VoiceCallResponse, (StatusCode, Json<ErrorResponse>)> {
    if req.sdp.trim().is_empty() {
        return Err(ErrorResponse::new("Missing SDP").into_response(StatusCode::BAD_REQUEST));
    }
    let binding = req.options.provider_id.clone();
    let normalized = normalize_options(req.options)?;
    let resolved = resolve_realtime_credentials(state, org.org_id, binding.as_deref()).await?;
    let credentials = resolved.credentials;
    let voice_connection_id = new_voice_connection_id();
    let form = multipart::Form::new()
        .part(
            "sdp",
            multipart::Part::text(req.sdp)
                .mime_str("application/sdp")
                .map_err(provider_error)?,
        )
        .text("session", realtime_session_payload(&normalized).to_string());
    let response = state
        .http_client()
        .post(format!(
            "{}/realtime/calls",
            openai_base_url(&credentials.base_url)
        ))
        .bearer_auth(&credentials.api_key)
        .header(
            "OpenAI-Safety-Identifier",
            safety_identifier(org.org_id, org.user_id, session_id),
        )
        .multipart(form)
        .send()
        .await
        .map_err(provider_error)?;
    if !response.status().is_success() {
        let status = response.status();
        let content_length = response.content_length();
        tracing::error!(
            provider_status = %status,
            provider_body = %redacted_provider_error_body(content_length),
            "Realtime voice provider returned an error"
        );
        emit_voice_failed(state, session_id, &voice_connection_id, "provider_error").await;
        return Err(ErrorResponse::bad_gateway());
    }
    let provider_call_id = parse_call_id_from_headers(response.headers());
    let answer_sdp = response.text().await.map_err(provider_error)?;
    let lease = upsert_voice_resource(VoiceResourceUpsert {
        state,
        session_id,
        org,
        voice_connection_id: &voice_connection_id,
        provider_call_id: provider_call_id.as_deref(),
        provider_id: &resolved.provider_id,
        options: &normalized,
        transport: "webrtc",
        status: "active",
    })
    .await?;
    emit_voice_started(
        state,
        session_id,
        &voice_connection_id,
        &normalized,
        "webrtc",
    )
    .await;
    if let Some(call_id) = provider_call_id.clone() {
        spawn_sideband(
            state.clone(),
            org.clone(),
            session_id,
            voice_connection_id.clone(),
            call_id,
            credentials,
        );
    }
    Ok(VoiceCallResponse {
        voice_connection_id,
        provider_call_id,
        provider: OPENAI_PROVIDER.to_string(),
        model: normalized.model,
        voice: normalized.voice,
        reasoning_effort: normalized.reasoning_effort,
        expires_at: lease.lease_expires_at,
        answer_sdp,
    })
}

impl AppState {
    fn http_client(&self) -> reqwest::Client {
        VOICE_HTTP_CLIENT
            .get_or_init(|| {
                reqwest::Client::builder()
                    .timeout(PROVIDER_REQUEST_TIMEOUT)
                    .build()
                    .expect("voice provider HTTP client configuration is valid")
            })
            .clone()
    }
}

#[derive(Debug, Clone)]
struct NormalizedVoiceOptions {
    model: String,
    voice: String,
    reasoning_effort: String,
    instructions: Option<String>,
}

fn ensure_voice_enabled(org: &ResolvedOrg) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if org.feature_flags.voice {
        Ok(())
    } else {
        Err(ErrorResponse::feature_not_enabled("voice"))
    }
}

pub(crate) fn microphone_permissions_policy_directive(voice_enabled: bool) -> &'static str {
    if voice_enabled {
        "microphone=(self)"
    } else {
        "microphone=()"
    }
}

fn normalize_options(
    options: VoiceSessionOptions,
) -> Result<NormalizedVoiceOptions, (StatusCode, Json<ErrorResponse>)> {
    let model = options.model.unwrap_or_else(|| DEFAULT_MODEL.to_string());
    if model != DEFAULT_MODEL {
        return Err(
            ErrorResponse::new("Unsupported voice model").into_response(StatusCode::BAD_REQUEST)
        );
    }
    let reasoning_effort = options
        .reasoning_effort
        .unwrap_or_else(|| DEFAULT_REASONING_EFFORT.to_string());
    if !matches!(
        reasoning_effort.as_str(),
        "minimal" | "low" | "medium" | "high" | "xhigh"
    ) {
        return Err(ErrorResponse::new("Unsupported reasoning effort")
            .into_response(StatusCode::BAD_REQUEST));
    }
    let voice = options
        .voice
        .or_else(|| std::env::var("OPENAI_REALTIME_VOICE").ok())
        .unwrap_or_else(|| DEFAULT_VOICE.to_string());
    Ok(NormalizedVoiceOptions {
        model,
        voice,
        reasoning_effort,
        instructions: options.instructions,
    })
}

fn realtime_session_payload(options: &NormalizedVoiceOptions) -> Value {
    let mut payload = json!({
        "type": "realtime",
        "model": options.model,
        "audio": {
            "input": {
                "transcription": {
                    "model": "gpt-4o-transcribe"
                },
                "turn_detection": {
                    "type": "server_vad",
                    "interrupt_response": false,
                    "create_response": false
                }
            },
            "output": {
                "voice": options.voice
            }
        },
        "reasoning": {
            "effort": options.reasoning_effort
        },
        "tools": []
    });
    if let Some(instructions) = options
        .instructions
        .as_ref()
        .filter(|instructions| !instructions.trim().is_empty())
    {
        payload["instructions"] = json!(instructions);
    }
    payload
}

async fn authorize_session(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: SessionId,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    state
        .session_service
        .get(&Caller::from(org), session_id.uuid(), None)
        .await
        .map(|_| ())
        .map_err(|err| {
            tracing::debug!(error = %err, "voice session authorization failed");
            ErrorResponse::not_found("Session")
        })
}

/// Resolve the realtime-voice provider connection for an org via service-bound
/// resolution (knowledge/foundations/providers.md): the provider whose driver declares
/// `ServiceKind::Realtime`, fail-closed. Replaces the previous "first active
/// provider matching the `openai` type string" behavior — only realtime-capable
/// drivers are eligible now.
///
/// `binding` is an optional client-supplied realtime provider public id. When
/// present it pins resolution to that provider (resolver tier 1); a binding that
/// is unknown or whose driver does not declare the realtime service is a request
/// error surfaced as 400, not an upstream `502`.
async fn resolve_realtime_credentials(
    state: &AppState,
    org_id: i64,
    binding: Option<&str>,
) -> Result<ResolvedServiceProvider, (StatusCode, Json<ErrorResponse>)> {
    state
        .provider_resolver
        .resolve_service(org_id, ServiceKind::Realtime, binding)
        .await
        .map_err(|error| map_realtime_resolution_error(binding, error))
}

/// Map a realtime provider resolution failure onto an HTTP error.
///
/// When the caller pinned a `binding`, the failure is about their request
/// (unknown provider id, driver that does not declare the realtime service, or
/// missing credentials), so return a `400` with the resolver's reason — a
/// multi-provider caller needs to know which selection to fix. With no binding
/// the failure is server-side configuration (no realtime provider available),
/// surfaced as the generic `502` like any other provider problem.
fn map_realtime_resolution_error(
    binding: Option<&str>,
    error: anyhow::Error,
) -> (StatusCode, Json<ErrorResponse>) {
    match binding {
        Some(provider_id) => {
            tracing::debug!(%provider_id, error = %error, "voice realtime provider binding rejected");
            ErrorResponse::new(format!(
                "Realtime provider '{provider_id}' is not available for voice: {error}"
            ))
            .into_response(StatusCode::BAD_REQUEST)
        }
        None => provider_error(error),
    }
}

fn parse_session_id(session_id: &str) -> Result<SessionId, (StatusCode, Json<ErrorResponse>)> {
    session_id.parse::<SessionId>().map_err(|_| {
        ErrorResponse::new("Invalid session ID").into_response(StatusCode::BAD_REQUEST)
    })
}

fn provider_error(error: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!("Realtime voice provider request failed: {error}");
    ErrorResponse::bad_gateway()
}

fn redacted_provider_error_body(content_length: Option<u64>) -> String {
    match content_length {
        Some(bytes) => format!("<redacted; content_length={bytes} bytes>"),
        None => "<redacted; content_length=unknown>".to_string(),
    }
}

fn new_voice_connection_id() -> String {
    format!("voice_conn_{}", Uuid::now_v7().simple())
}

fn safety_identifier(org_id: i64, user_id: Option<Uuid>, session_id: SessionId) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"everruns:voice:v1:");
    hasher.update(org_id.to_be_bytes());
    hasher.update(b":");
    if let Some(user_id) = user_id {
        hasher.update(user_id.as_bytes());
    }
    hasher.update(b":");
    hasher.update(session_id.uuid().as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("evr_{}", &digest[..60])
}

fn openai_base_url(base_url: &Option<String>) -> String {
    base_url
        .as_deref()
        .unwrap_or("https://api.openai.com/v1")
        .trim_end_matches('/')
        .to_string()
}

fn sideband_url(base_url: &Option<String>, call_id: &str) -> String {
    openai_base_url(base_url)
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
        + &format!("/realtime?call_id={call_id}")
}

fn parse_call_id_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_call_id_from_location)
}

fn parse_call_id_from_location(location: &str) -> Option<String> {
    location
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|segment| segment.starts_with("rtc_"))
        .map(ToOwned::to_owned)
}

struct VoiceResourceUpsert<'a> {
    state: &'a AppState,
    session_id: SessionId,
    org: &'a ResolvedOrg,
    voice_connection_id: &'a str,
    provider_call_id: Option<&'a str>,
    /// Public id of the realtime provider connection resolved for this voice
    /// connection (the binding when one was supplied, otherwise the
    /// auto-selected provider). Persisted for audit / multi-provider visibility.
    provider_id: &'a str,
    options: &'a NormalizedVoiceOptions,
    transport: &'a str,
    status: &'a str,
}

async fn upsert_voice_resource(
    req: VoiceResourceUpsert<'_>,
) -> Result<LeasedResource, (StatusCode, Json<ErrorResponse>)> {
    let mut metadata = json!({
        "status": req.status,
        "model": req.options.model,
        "voice": req.options.voice,
        "reasoning_effort": req.options.reasoning_effort,
        "transport": req.transport,
        "provider_id": req.provider_id,
    });
    if let Some(provider_call_id) = req.provider_call_id {
        metadata["provider_call_id"] = json!(provider_call_id);
    }
    req.state
        .leased_resource_store
        .upsert_resource(UpsertLeasedResource {
            session_id: req.session_id,
            provider: OPENAI_PROVIDER.to_string(),
            resource_type: VOICE_RESOURCE_TYPE.to_string(),
            external_id: req.voice_connection_id.to_string(),
            display_name: Some("Voice Connection".to_string()),
            owner_user_id: req.org.user_id,
            lease_duration_seconds: LEASE_SECONDS,
            metadata,
        })
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "failed to upsert voice leased resource");
            ErrorResponse::internal_error()
        })
}

async fn release_voice_resource(
    state: &AppState,
    session_id: SessionId,
    voice_connection_id: &str,
) -> Result<Option<LeasedResource>, (StatusCode, Json<ErrorResponse>)> {
    state
        .leased_resource_store
        .release_resource(
            session_id,
            OPENAI_PROVIDER,
            VOICE_RESOURCE_TYPE,
            voice_connection_id,
        )
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "failed to release voice leased resource");
            ErrorResponse::internal_error()
        })
}

async fn emit_voice_started(
    state: &AppState,
    session_id: SessionId,
    voice_connection_id: &str,
    options: &NormalizedVoiceOptions,
    transport: &str,
) {
    let _ = state
        .event_service
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            VoiceSessionStartedData {
                voice_connection_id: voice_connection_id.to_string(),
                model: options.model.clone(),
                voice: options.voice.clone(),
                reasoning_effort: options.reasoning_effort.clone(),
                transport: transport.to_string(),
            },
        ))
        .await;
}

async fn emit_voice_ended(
    state: &AppState,
    session_id: SessionId,
    voice_connection_id: &str,
    reason: Option<String>,
    duration_ms: Option<u64>,
) {
    let _ = state
        .event_service
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            VoiceSessionEndedData {
                voice_connection_id: voice_connection_id.to_string(),
                reason,
                duration_ms,
            },
        ))
        .await;
}

async fn emit_voice_failed(
    state: &AppState,
    session_id: SessionId,
    voice_connection_id: &str,
    error: &str,
) {
    let _ = state
        .event_service
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            VoiceSessionFailedData {
                voice_connection_id: voice_connection_id.to_string(),
                error: error.to_string(),
            },
        ))
        .await;
}

fn spawn_sideband(
    state: AppState,
    org: ResolvedOrg,
    session_id: SessionId,
    voice_connection_id: String,
    provider_call_id: String,
    credentials: ResolvedProviderCredentials,
) {
    tokio::spawn(async move {
        if let Err(error) = run_sideband(
            state.clone(),
            org,
            session_id,
            voice_connection_id.clone(),
            provider_call_id,
            credentials,
        )
        .await
        {
            tracing::warn!(session_id = %session_id, voice_connection_id = %voice_connection_id, "Realtime voice sideband ended with sanitized error: {error}");
            emit_voice_failed(&state, session_id, &voice_connection_id, "sideband_failed").await;
        }
    });
}

async fn run_sideband(
    state: AppState,
    org: ResolvedOrg,
    session_id: SessionId,
    voice_connection_id: String,
    provider_call_id: String,
    credentials: ResolvedProviderCredentials,
) -> anyhow::Result<()> {
    let url = sideband_url(&credentials.base_url, &provider_call_id);
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", credentials.api_key))?,
    );
    let (mut ws, _) = connect_async(request).await?;
    let started_at = Instant::now();
    let mut input_accumulator = TranscriptAccumulator::default();
    let mut output_accumulator = TranscriptAccumulator::default();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Value>();
    loop {
        tokio::select! {
            message = ws.next() => {
                let Some(message) = message else {
                    break;
                };
                let message = message?;
                let WsMessage::Text(text) = message else {
                    continue;
                };
                let value: Value = match serde_json::from_str(&text) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if let Some(event) = map_sideband_event(
                    &voice_connection_id,
                    &value,
                    &mut input_accumulator,
                    &mut output_accumulator,
                ) {
                    emit_voice_transcript(&state, &org, session_id, event, &outbound_tx).await;
                }
                for call in extract_function_calls(&value) {
                    let output = execute_realtime_tool(session_id, call).await;
                    ws.send(WsMessage::Text(
                        json!({
                            "type": "conversation.item.create",
                            "item": {
                                "type": "function_call_output",
                                "call_id": output.call_id,
                                "output": output.output
                            }
                        })
                        .to_string()
                        .into(),
                    ))
                    .await?;
                    ws.send(WsMessage::Text(
                        json!({ "type": "response.create" }).to_string().into(),
                    ))
                    .await?;
                }
            }
            outbound = outbound_rx.recv() => {
                if let Some(outbound) = outbound {
                    ws.send(WsMessage::Text(outbound.to_string().into())).await?;
                }
            }
        }
    }
    release_voice_resource(&state, session_id, &voice_connection_id)
        .await
        .ok();
    emit_voice_ended(
        &state,
        session_id,
        &voice_connection_id,
        Some("sideband_closed".to_string()),
        Some(
            started_at
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        ),
    )
    .await;
    Ok(())
}

#[derive(Default)]
struct TranscriptAccumulator {
    by_key: HashMap<String, String>,
}

impl TranscriptAccumulator {
    fn add_delta(&mut self, key: &str, delta: &str) -> String {
        let entry = self.by_key.entry(key.to_string()).or_default();
        entry.push_str(delta);
        entry.clone()
    }
    fn complete(&mut self, key: &str, transcript: Option<&str>) -> String {
        if let Some(transcript) = transcript.filter(|s| !s.is_empty()) {
            self.by_key.insert(key.to_string(), transcript.to_string());
        }
        self.by_key.remove(key).unwrap_or_default()
    }
}

struct MappedTranscriptEvent {
    event_type: &'static str,
    data: VoiceTranscriptData,
}

fn map_sideband_event(
    voice_connection_id: &str,
    value: &Value,
    input: &mut TranscriptAccumulator,
    output: &mut TranscriptAccumulator,
) -> Option<MappedTranscriptEvent> {
    let event_type = value.get("type")?.as_str()?;
    match event_type {
        "conversation.item.input_audio_transcription.delta" => {
            let item_id = value_string(value, "item_id").unwrap_or_else(|| "input".to_string());
            let delta = value_string(value, "delta").unwrap_or_default();
            let accumulated = input.add_delta(&item_id, &delta);
            Some(MappedTranscriptEvent {
                event_type: VOICE_INPUT_TRANSCRIPT_DELTA,
                data: VoiceTranscriptData {
                    voice_connection_id: voice_connection_id.to_string(),
                    item_id: Some(item_id),
                    response_id: None,
                    phase: None,
                    delta,
                    accumulated,
                },
            })
        }
        "conversation.item.input_audio_transcription.completed" => {
            let item_id = value_string(value, "item_id").unwrap_or_else(|| "input".to_string());
            let transcript = value_string(value, "transcript");
            let accumulated = input.complete(&item_id, transcript.as_deref());
            Some(MappedTranscriptEvent {
                event_type: VOICE_INPUT_TRANSCRIPT_COMPLETED,
                data: VoiceTranscriptData {
                    voice_connection_id: voice_connection_id.to_string(),
                    item_id: Some(item_id),
                    response_id: None,
                    phase: None,
                    delta: String::new(),
                    accumulated,
                },
            })
        }
        "response.audio_transcript.delta"
        | "response.output_audio_transcript.delta"
        | "response.output_text.delta" => {
            let response_id =
                value_string(value, "response_id").unwrap_or_else(|| "response".to_string());
            let delta = value_string(value, "delta").unwrap_or_default();
            let accumulated = output.add_delta(&response_id, &delta);
            Some(MappedTranscriptEvent {
                event_type: VOICE_OUTPUT_TRANSCRIPT_DELTA,
                data: VoiceTranscriptData {
                    voice_connection_id: voice_connection_id.to_string(),
                    item_id: value_string(value, "item_id"),
                    response_id: Some(response_id),
                    phase: Some("commentary".to_string()),
                    delta,
                    accumulated,
                },
            })
        }
        "response.audio_transcript.done"
        | "response.output_audio_transcript.done"
        | "response.output_text.done" => {
            let response_id =
                value_string(value, "response_id").unwrap_or_else(|| "response".to_string());
            let transcript =
                value_string(value, "transcript").or_else(|| value_string(value, "text"));
            let accumulated = output.complete(&response_id, transcript.as_deref());
            Some(MappedTranscriptEvent {
                event_type: VOICE_OUTPUT_TRANSCRIPT_COMPLETED,
                data: VoiceTranscriptData {
                    voice_connection_id: voice_connection_id.to_string(),
                    item_id: value_string(value, "item_id"),
                    response_id: Some(response_id),
                    phase: Some("final_answer".to_string()),
                    delta: String::new(),
                    accumulated,
                },
            })
        }
        _ => None,
    }
}

async fn emit_voice_transcript(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: SessionId,
    event: MappedTranscriptEvent,
    outbound_tx: &mpsc::UnboundedSender<Value>,
) {
    let data = event.data.clone();
    let _ = state
        .event_service
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            EventData::voice_transcript_event(event.data, event.event_type),
        ))
        .await;
    if data.accumulated.trim().is_empty() {
        return;
    }
    match event.event_type {
        VOICE_INPUT_TRANSCRIPT_COMPLETED => {
            let command = build_voice_message_command(session_id, &data);
            match command.run(&state.ctx(org)).await {
                Ok(message) => {
                    let state = state.clone();
                    let outbound_tx = outbound_tx.clone();
                    tokio::spawn(async move {
                        wait_for_voice_answer(
                            state,
                            session_id,
                            message.id,
                            message.sequence,
                            outbound_tx,
                        )
                        .await;
                    });
                }
                Err(error) => {
                    tracing::error!(
                        session_id = %session_id,
                        voice_connection_id = %data.voice_connection_id,
                        error = %error,
                        "failed to create user message from voice transcript"
                    );
                }
            }
        }
        VOICE_OUTPUT_TRANSCRIPT_COMPLETED => {}
        _ => {}
    }
}

async fn wait_for_voice_answer(
    state: AppState,
    session_id: SessionId,
    input_message_id: MessageId,
    mut since_sequence: i32,
    outbound_tx: mpsc::UnboundedSender<Value>,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let filters = Vec::<String>::new();
    loop {
        if tokio::time::Instant::now() > deadline {
            tracing::warn!(
                session_id = %session_id,
                input_message_id = %input_message_id,
                "timed out waiting for voice answer"
            );
            return;
        }
        let events = match state
            .event_service
            .list(
                session_id.uuid(),
                Some(since_sequence),
                None,
                &filters,
                &filters,
                None,
                Some(100),
            )
            .await
        {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    input_message_id = %input_message_id,
                    error = %error,
                    "failed to list events while waiting for voice answer"
                );
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };
        for event in events {
            if let Some(sequence) = event.sequence {
                since_sequence = sequence;
            }
            if let Some(answer) = final_answer_for_voice_input(&event, input_message_id) {
                if outbound_tx
                    .send(realtime_spoken_answer_event(&answer))
                    .is_err()
                {
                    tracing::debug!(
                        session_id = %session_id,
                        input_message_id = %input_message_id,
                        "voice sideband closed before spoken answer could be sent"
                    );
                }
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn final_answer_for_voice_input(event: &Event, input_message_id: MessageId) -> Option<String> {
    if event.context.input_message_id != Some(input_message_id) {
        return None;
    }
    let EventData::OutputMessageCompleted(data) = &event.data else {
        return None;
    };
    if matches!(data.message.phase, Some(ExecutionPhase::Commentary)) {
        return None;
    }
    let text = content_parts_to_text(&data.message.content);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn content_parts_to_text(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn realtime_spoken_answer_event(answer: &str) -> Value {
    json!({
        "type": "response.create",
        "response": {
            "conversation": "none",
            "output_modalities": ["audio"],
            "input": [],
            "instructions": format!(
                "Speak this Everruns answer to the user exactly. Do not add new facts or tool results.\n\nAnswer:\n\n{}",
                answer.trim()
            )
        }
    })
}

fn build_voice_message_command(session_id: SessionId, data: &VoiceTranscriptData) -> CreateMessage {
    let mut metadata = HashMap::from([
        ("source".to_string(), json!("voice")),
        (
            "voice_connection_id".to_string(),
            json!(data.voice_connection_id.clone()),
        ),
    ]);
    if let Some(item_id) = &data.item_id {
        metadata.insert("voice_item_id".to_string(), json!(item_id));
    }
    CreateMessage {
        session_id: session_id.to_string(),
        message: InputMessage {
            role: MessageRole::User,
            content: vec![InputContentPart::text(data.accumulated.trim().to_string())],
        },
        addressed_participant_id: None,
        controls: None,
        metadata: Some(metadata),
        tags: Some(vec!["voice".to_string()]),
        external_actor: None,
        request_id: None,
    }
}

fn value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

struct RealtimeToolOutput {
    call_id: String,
    output: String,
}

fn extract_function_calls(value: &Value) -> Vec<ToolCall> {
    if value.get("type").and_then(Value::as_str) != Some("response.done") {
        return Vec::new();
    }
    value
        .pointer("/response/output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
        .filter_map(|item| {
            let name = value_string(item, "name")?;
            let id = value_string(item, "call_id").or_else(|| value_string(item, "id"))?;
            let arguments = value_string(item, "arguments")
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .unwrap_or_else(|| json!({}));
            Some(ToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
}

async fn execute_realtime_tool(_session_id: SessionId, call: ToolCall) -> RealtimeToolOutput {
    RealtimeToolOutput {
        call_id: call.id,
        output: json!({ "error": "tool_execution_disabled" }).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_session_options_accept_provider_binding() {
        // The realtime provider binding rides on the shared options struct that
        // is flattened into every create/attach voice request body.
        let opts: VoiceSessionOptions =
            serde_json::from_value(json!({ "provider_id": "prov_realtime_1" }))
                .expect("provider_id is an accepted option");
        assert_eq!(opts.provider_id.as_deref(), Some("prov_realtime_1"));

        // Omitting it is valid and resolves to the org default later.
        let empty: VoiceSessionOptions = serde_json::from_value(json!({})).expect("empty options");
        assert_eq!(empty.provider_id, None);
    }

    #[test]
    fn provider_binding_is_not_forwarded_to_the_realtime_payload() {
        // The binding is a routing decision, not a realtime-session knob, so it
        // must not leak into the provider session payload.
        let normalized = normalize_options(VoiceSessionOptions {
            model: None,
            voice: None,
            reasoning_effort: None,
            instructions: None,
            provider_id: Some("prov_realtime_1".to_string()),
        })
        .expect("default options normalize");
        let payload = realtime_session_payload(&normalized);
        assert_eq!(payload.get("provider_id"), None);
        assert_eq!(payload.get("provider"), None);
    }

    #[test]
    fn bad_provider_binding_maps_to_bad_request() {
        // A client-chosen provider that cannot serve realtime is the caller's
        // mistake — surface 400 with the resolver's reason, not an opaque 502.
        let (status, body) = map_realtime_resolution_error(
            Some("prov_realtime_1"),
            anyhow::anyhow!("provider prov_realtime_1 does not provide the realtime service"),
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let detail = body.0.detail.expect("detail set");
        assert!(detail.contains("prov_realtime_1"), "names the provider");
        assert!(
            detail.contains("does not provide the realtime service"),
            "surfaces the resolver reason: {detail}"
        );
    }

    #[test]
    fn missing_realtime_provider_without_binding_maps_to_bad_gateway() {
        // No binding => server-side configuration gap, reported like any other
        // provider failure (generic 502, no internal detail leaked).
        let (status, _body) = map_realtime_resolution_error(
            None,
            anyhow::anyhow!("no provider configured for the realtime service"),
        );
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn parses_call_id_from_location() {
        assert_eq!(
            parse_call_id_from_location("https://api.openai.com/v1/realtime/calls/rtc_abc123"),
            Some("rtc_abc123".to_string())
        );
        assert_eq!(
            parse_call_id_from_location("https://example.com/nope"),
            None
        );
    }

    #[test]
    fn safety_identifier_is_stable_and_privacy_preserving() {
        let session_id = SessionId::new();
        let user_id = Uuid::from_u128(42);
        let first = safety_identifier(7, Some(user_id), session_id);
        let second = safety_identifier(7, Some(user_id), session_id);
        assert_eq!(first, second);
        assert!(first.starts_with("evr_"));
        assert_eq!(first.len(), 64);
        assert!(!first.contains(&user_id.to_string()));
        assert!(!first.contains(&session_id.to_string()));
    }

    #[test]
    fn transcript_mapping_accumulates_and_completes() {
        let mut input = TranscriptAccumulator::default();
        let mut output = TranscriptAccumulator::default();
        let delta = map_sideband_event(
            "voice_conn_1",
            &json!({
                "type": "response.audio_transcript.delta",
                "response_id": "resp_1",
                "delta": "hel"
            }),
            &mut input,
            &mut output,
        )
        .expect("delta event");
        assert_eq!(delta.event_type, VOICE_OUTPUT_TRANSCRIPT_DELTA);
        assert_eq!(delta.data.accumulated, "hel");
        let done = map_sideband_event(
            "voice_conn_1",
            &json!({
                "type": "response.audio_transcript.done",
                "response_id": "resp_1",
                "transcript": "hello"
            }),
            &mut input,
            &mut output,
        )
        .expect("done event");
        assert_eq!(done.event_type, VOICE_OUTPUT_TRANSCRIPT_COMPLETED);
        assert_eq!(done.data.accumulated, "hello");
    }

    #[test]
    fn extracts_response_done_function_calls() {
        let calls = extract_function_calls(&json!({
            "type": "response.done",
            "response": {
                "output": [{
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "get_current_time",
                    "arguments": "{\"timezone\":\"UTC\"}"
                }]
            }
        }));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_current_time");
        assert_eq!(calls[0].arguments["timezone"], "UTC");
    }

    #[test]
    fn realtime_session_payload_does_not_advertise_tools() {
        let payload = realtime_session_payload(&NormalizedVoiceOptions {
            model: DEFAULT_MODEL.to_string(),
            voice: DEFAULT_VOICE.to_string(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.to_string(),
            instructions: None,
        });

        assert_eq!(payload["tools"], json!([]));
    }

    #[test]
    fn realtime_session_payload_uses_realtime_reasoning_model() {
        let payload = realtime_session_payload(&NormalizedVoiceOptions {
            model: DEFAULT_MODEL.to_string(),
            voice: DEFAULT_VOICE.to_string(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.to_string(),
            instructions: None,
        });

        assert_eq!(payload["model"], json!("gpt-realtime-2"));
        assert_eq!(
            payload["reasoning"]["effort"],
            json!(DEFAULT_REASONING_EFFORT)
        );
    }

    #[test]
    fn realtime_session_payload_requests_input_transcription() {
        let payload = realtime_session_payload(&NormalizedVoiceOptions {
            model: DEFAULT_MODEL.to_string(),
            voice: DEFAULT_VOICE.to_string(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.to_string(),
            instructions: None,
        });

        assert_eq!(
            payload["audio"]["input"]["transcription"]["model"],
            json!("gpt-4o-transcribe")
        );
    }

    #[test]
    fn realtime_session_payload_disables_automatic_voice_responses() {
        let payload = realtime_session_payload(&NormalizedVoiceOptions {
            model: DEFAULT_MODEL.to_string(),
            voice: DEFAULT_VOICE.to_string(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.to_string(),
            instructions: None,
        });

        assert_eq!(
            payload["audio"]["input"]["turn_detection"]["type"],
            json!("server_vad")
        );
        assert_eq!(
            payload["audio"]["input"]["turn_detection"]["interrupt_response"],
            json!(false)
        );
        assert_eq!(
            payload["audio"]["input"]["turn_detection"]["create_response"],
            json!(false)
        );
    }

    #[test]
    fn microphone_permissions_policy_follows_voice_flag() {
        assert_eq!(
            microphone_permissions_policy_directive(false),
            "microphone=()"
        );
        assert_eq!(
            microphone_permissions_policy_directive(true),
            "microphone=(self)"
        );
    }

    #[test]
    fn voice_input_transcript_builds_chat_message_command() {
        let session_id = SessionId::new();
        let command = build_voice_message_command(
            session_id,
            &VoiceTranscriptData {
                voice_connection_id: "voice_conn_1".to_string(),
                item_id: Some("item_1".to_string()),
                response_id: None,
                phase: None,
                delta: String::new(),
                accumulated: "  search the docs  ".to_string(),
            },
        );

        assert_eq!(command.session_id, session_id.to_string());
        assert_eq!(command.message.role, MessageRole::User);
        assert_eq!(
            command.message.content[0].as_text(),
            Some("search the docs")
        );
        assert_eq!(command.tags, Some(vec!["voice".to_string()]));
        let metadata = command.metadata.expect("voice metadata");
        assert_eq!(metadata["source"], json!("voice"));
        assert_eq!(metadata["voice_connection_id"], json!("voice_conn_1"));
        assert_eq!(metadata["voice_item_id"], json!("item_1"));
    }

    #[test]
    fn maps_ga_output_audio_transcript_events() {
        let mut input = TranscriptAccumulator::default();
        let mut output = TranscriptAccumulator::default();
        let delta = map_sideband_event(
            "voice_conn_1",
            &json!({
                "type": "response.output_audio_transcript.delta",
                "response_id": "resp_1",
                "delta": "hel"
            }),
            &mut input,
            &mut output,
        )
        .expect("delta event");
        assert_eq!(delta.event_type, VOICE_OUTPUT_TRANSCRIPT_DELTA);
        assert_eq!(delta.data.accumulated, "hel");

        let done = map_sideband_event(
            "voice_conn_1",
            &json!({
                "type": "response.output_audio_transcript.done",
                "response_id": "resp_1",
                "transcript": "hello"
            }),
            &mut input,
            &mut output,
        )
        .expect("done event");
        assert_eq!(done.event_type, VOICE_OUTPUT_TRANSCRIPT_COMPLETED);
        assert_eq!(done.data.accumulated, "hello");
    }

    #[test]
    fn final_answer_for_voice_input_returns_matching_final_text() {
        let input_message_id = MessageId::new();
        let event = output_completed_event(
            input_message_id,
            everruns_core::Message::assistant("The tool result is 42.")
                .with_phase(ExecutionPhase::FinalAnswer),
        );

        assert_eq!(
            final_answer_for_voice_input(&event, input_message_id),
            Some("The tool result is 42.".to_string())
        );
    }

    #[test]
    fn final_answer_for_voice_input_ignores_commentary_and_other_turns() {
        let input_message_id = MessageId::new();
        let commentary = output_completed_event(
            input_message_id,
            everruns_core::Message::assistant_with_tools(
                "Checking.",
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "read_harnesses".to_string(),
                    arguments: json!({}),
                }],
            )
            .with_phase(ExecutionPhase::Commentary),
        );
        let other_turn = output_completed_event(
            MessageId::new(),
            everruns_core::Message::assistant("Other answer.")
                .with_phase(ExecutionPhase::FinalAnswer),
        );

        assert_eq!(
            final_answer_for_voice_input(&commentary, input_message_id),
            None
        );
        assert_eq!(
            final_answer_for_voice_input(&other_turn, input_message_id),
            None
        );
    }

    #[test]
    fn realtime_spoken_answer_event_requests_audio_only() {
        let event = realtime_spoken_answer_event(" Done. ");

        assert_eq!(event["type"], json!("response.create"));
        assert_eq!(event["response"]["output_modalities"], json!(["audio"]));
        assert_eq!(event["response"]["conversation"], json!("none"));
        assert_eq!(event["response"]["input"], json!([]));
        assert!(
            event["response"]["instructions"]
                .as_str()
                .expect("instructions")
                .contains("Done.")
        );
    }

    #[test]
    fn redacts_provider_error_bodies_for_logs() {
        assert_eq!(
            redacted_provider_error_body(Some(3000)),
            "<redacted; content_length=3000 bytes>"
        );
        assert_eq!(
            redacted_provider_error_body(None),
            "<redacted; content_length=unknown>"
        );
    }

    #[tokio::test]
    async fn realtime_tool_execution_is_disabled() {
        let output = execute_realtime_tool(
            SessionId::new(),
            ToolCall {
                id: "call_passthrough".to_string(),
                name: "web_fetch".to_string(),
                arguments: json!({ "url": "https://example.com" }),
            },
        )
        .await;

        assert_eq!(output.call_id, "call_passthrough");
        assert_eq!(
            serde_json::from_str::<Value>(&output.output).expect("json output"),
            json!({ "error": "tool_execution_disabled" })
        );
    }

    fn output_completed_event(
        input_message_id: MessageId,
        message: everruns_core::Message,
    ) -> Event {
        Event {
            id: everruns_core::typed_id::EventId::new(),
            event_type: everruns_core::events::OUTPUT_MESSAGE_COMPLETED.to_string(),
            ts: chrono::Utc::now(),
            session_id: SessionId::new(),
            context: EventContext::turn(everruns_core::typed_id::TurnId::new(), input_message_id),
            data: EventData::OutputMessageCompleted(
                everruns_core::events::OutputMessageCompletedData::new(message),
            ),
            metadata: None,
            tags: None,
            sequence: Some(1),
        }
    }
}
