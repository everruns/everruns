// Realtime voice session endpoints.
//
// Security boundary: client SDP, ephemeral provider secrets, and raw sideband
// payloads are never persisted or logged. Durable state stores only sanitized
// lifecycle metadata plus text transcripts marked with metadata.source=voice.

use crate::api::common::{ErrorResponse, impl_auth_state};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::sessions::{CreateSession, GetOrCreateChatSession, SessionService};
use crate::event_delivery::EventDelivery;
use crate::services::{
    EventService, LlmResolverService, llm_resolver::ResolvedProviderCredentials,
};
use crate::storage::{DbLeasedResourceStore, DbSessionResourceRegistry, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    routing::post,
};
use everruns_core::events::{
    EventContext, EventData, EventRequest, InputMessageData, OutputMessageCompletedData,
    VOICE_INPUT_TRANSCRIPT_COMPLETED, VOICE_INPUT_TRANSCRIPT_DELTA,
    VOICE_OUTPUT_TRANSCRIPT_COMPLETED, VOICE_OUTPUT_TRANSCRIPT_DELTA, VoiceSessionEndedData,
    VoiceSessionFailedData, VoiceSessionStartedData, VoiceTranscriptData,
};
use everruns_core::traits::LeasedResourceStore;
use everruns_core::typed_id::{AgentId, SessionId};
use everruns_core::{
    Caller, FeatureFlags, LeasedResource, Message, ToolCall, UpsertLeasedResource,
};
use futures_util::{SinkExt, StreamExt};
use reqwest::multipart;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message as WsMessage, client::IntoClientRequest, http::HeaderValue},
};
use utoipa::ToSchema;
use uuid::Uuid;

const OPENAI_PROVIDER: &str = "openai";
const VOICE_RESOURCE_TYPE: &str = "voice_connection";
const DEFAULT_MODEL: &str = "gpt-realtime-2";
const DEFAULT_VOICE: &str = "marin";
const DEFAULT_REASONING_EFFORT: &str = "low";
const LEASE_SECONDS: u32 = 15 * 60;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub event_service: EventService,
    pub auth: AuthState,
    pub llm_resolver: Arc<LlmResolverService>,
    pub leased_resource_store: Arc<dyn LeasedResourceStore>,
    pub feature_flags: FeatureFlags,
    pub runner: Arc<dyn everruns_worker::AgentRunner>,
    pub fallback_default_harness_name: Option<String>,
    pub chat_harness_name: Option<String>,
    pub chat_session_title: Option<String>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn everruns_worker::AgentRunner>,
        auth: AuthState,
        llm_resolver: Arc<LlmResolverService>,
        feature_flags: FeatureFlags,
        platform_definition: &everruns_core::PlatformDefinition,
        event_delivery: EventDelivery,
    ) -> Self {
        let registry = Arc::new(DbSessionResourceRegistry::new(db.clone()));
        let leased_resource_store =
            Arc::new(DbLeasedResourceStore::new(db.clone()).with_registry(registry))
                as Arc<dyn LeasedResourceStore>;
        Self {
            session_service: Arc::new(SessionService::with_registry(
                db.clone(),
                platform_definition.capability_registry().clone(),
            )),
            event_service: EventService::new(db.clone(), event_delivery),
            db,
            auth,
            llm_resolver,
            leased_resource_store,
            feature_flags,
            runner,
            fallback_default_harness_name: platform_definition
                .harness_for_role(everruns_core::BuiltInHarnessRole::Default)
                .map(|h| h.name.clone()),
            chat_harness_name: platform_definition
                .harness_for_role(everruns_core::BuiltInHarnessRole::Chat)
                .map(|h| h.name.clone()),
            chat_session_title: platform_definition
                .harness_for_role(everruns_core::BuiltInHarnessRole::Chat)
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

/// Realtime-session knobs shared by every voice endpoint
/// (`/voice/client-secret`, `/voice/calls`, `/voice/attach`,
/// `/voice/sessions`). All fields are optional; omitted ones fall back to
/// the agent's or provider's default.
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
    pub session: everruns_core::Session,
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
    ensure_voice_enabled(&state)?;
    let session_id = parse_session_id(&session_id)?;
    authorize_session(&state, &org, session_id).await?;
    let normalized = normalize_options(req.options)?;
    let credentials = resolve_openai_credentials(&state, org.org_id).await?;
    let voice_connection_id = new_voice_connection_id();
    let lease = upsert_voice_resource(VoiceResourceUpsert {
        state: &state,
        session_id,
        org: &org,
        voice_connection_id: &voice_connection_id,
        provider_call_id: None,
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
    ensure_voice_enabled(&state)?;
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
    ensure_voice_enabled(&state)?;
    let session_id = parse_session_id(&session_id)?;
    authorize_session(&state, &org, session_id).await?;
    if voice_connection_id.trim().is_empty() || req.provider_call_id.trim().is_empty() {
        return Err(
            ErrorResponse::new("Invalid voice connection").into_response(StatusCode::BAD_REQUEST)
        );
    }
    let normalized = normalize_options(req.options)?;
    let credentials = resolve_openai_credentials(&state, org.org_id).await?;
    let lease = upsert_voice_resource(VoiceResourceUpsert {
        state: &state,
        session_id,
        org: &org,
        voice_connection_id: &voice_connection_id,
        provider_call_id: Some(&req.provider_call_id),
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
    ensure_voice_enabled(&state)?;
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
    ensure_voice_enabled(&state)?;
    let session = CreateSession(crate::api::sessions::CreateSessionRequest {
        harness_id: None,
        harness_name: None,
        agent_id: Some(agent_id),
        agent_identity_id: None,
        title: Some("Voice session".to_string()),
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
    description = "Create a voice session for the user's global chat. Returns connection details for the realtime audio channel.",
    post,
    path = "/v1/sessions/chat/voice",
    request_body = VoiceCallRequest,
    responses((status = 200, description = "Platform chat session and realtime call created", body = VoiceSessionResponse<VoiceCallResponse>)),
    tag = "voice"
)]
pub async fn create_chat_voice_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<VoiceCallRequest>,
) -> Result<Json<VoiceSessionResponse<VoiceCallResponse>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_voice_enabled(&state)?;
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
    let normalized = normalize_options(req.options)?;
    let credentials = resolve_openai_credentials(state, org.org_id).await?;
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
        reqwest::Client::new()
    }
}

#[derive(Debug, Clone)]
struct NormalizedVoiceOptions {
    model: String,
    voice: String,
    reasoning_effort: String,
    instructions: Option<String>,
}

fn ensure_voice_enabled(state: &AppState) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if state.feature_flags.voice {
        Ok(())
    } else {
        Err(ErrorResponse::not_found("Voice"))
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

async fn resolve_openai_credentials(
    state: &AppState,
    org_id: i64,
) -> Result<ResolvedProviderCredentials, (StatusCode, Json<ErrorResponse>)> {
    state
        .llm_resolver
        .resolve_provider_credentials(org_id, OPENAI_PROVIDER)
        .await
        .map_err(provider_error)?
        .ok_or_else(ErrorResponse::bad_gateway)
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
    format!("evr_{}", hex::encode(hasher.finalize()))
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
    session_id: SessionId,
    voice_connection_id: String,
    provider_call_id: String,
    credentials: ResolvedProviderCredentials,
) {
    tokio::spawn(async move {
        if let Err(error) = run_sideband(
            state.clone(),
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
    while let Some(message) = ws.next().await {
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
            emit_voice_transcript(&state, session_id, event).await;
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
        "response.audio_transcript.delta" | "response.output_text.delta" => {
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
        "response.audio_transcript.done" | "response.output_text.done" => {
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
    session_id: SessionId,
    event: MappedTranscriptEvent,
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
            let mut message = Message::user(data.accumulated);
            message.metadata = Some(HashMap::from([("source".to_string(), json!("voice"))]));
            let _ = state
                .event_service
                .emit(EventRequest::new(
                    session_id,
                    EventContext::empty(),
                    InputMessageData::new(message),
                ))
                .await;
        }
        VOICE_OUTPUT_TRANSCRIPT_COMPLETED => {
            let mut message = Message::assistant(data.accumulated);
            message.metadata = Some(HashMap::from([("source".to_string(), json!("voice"))]));
            let _ = state
                .event_service
                .emit(EventRequest::new(
                    session_id,
                    EventContext::empty(),
                    OutputMessageCompletedData::new(message),
                ))
                .await;
        }
        _ => {}
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
        assert_eq!(first.len(), 68);
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
}
