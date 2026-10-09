// Voice calls: an agent's voice channel, or a session's chat microphone.
//
// Decisions (knowledge/framework/voice-agents.md):
// - Voice is a channel type. A call takes its settings (speech model, voice,
//   greeting, interruption policy, filler) from the agent's voice channel; a
//   session call without a channel uses the first voice channel of the
//   session's agent, then the defaults.
// - Delegated mode: browser audio goes straight to the speech provider over
//   WebRTC; the server only proxies the SDP offer and drives the call over the
//   provider's control connection with the shared core voice loop. Final
//   transcripts become ordinary user messages (start or steer a turn), and the
//   agent's streamed answer is spoken sentence by sentence from live events.
// - Client SDP, provider secrets and raw provider payloads are never persisted
//   or logged. Durable state is a leased resource with sanitized metadata plus
//   voice events.
// - Ending a call is best effort across instances: the instance running the
//   call stops it at once; elsewhere the call ends when the browser hangs up and
//   the provider closes the control connection.

use crate::api::common::{ErrorResponse, impl_auth_state};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::agent_channels::{GetAgentChannel, ListAgentChannels};
use crate::domains::common::{Command, Ctx};
use crate::domains::messages::MessageService;
use crate::domains::sessions::{CreateSession, SessionService};
use crate::event_delivery::EventDelivery;
use crate::kernel_imports::Caller;
use crate::records::{ChannelType, FeatureFlags, Session};
use crate::services::{EventService, ProviderResolverService};
use crate::storage::{DbLeasedResourceStore, DbSessionResourceRegistry, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use everruns_contracts::typed_id::SessionId;
use everruns_contracts::voice::VoiceChannelConfig;
use everruns_core::session_services::LeasedResourceStore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

mod authorization;
mod call;
mod lifecycle;

use authorization::authorize_session;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: EventService,
    pub event_delivery: EventDelivery,
    pub auth: AuthState,
    pub provider_resolver: Arc<ProviderResolverService>,
    pub leased_resource_store: Arc<dyn LeasedResourceStore>,
    pub feature_flags: FeatureFlags,
    pub runner: Arc<dyn everruns_core::host::TurnBackend>,
    pub fallback_default_harness_name: Option<String>,
    /// Decrypts stored channel configuration.
    pub encryption: Option<Arc<crate::storage::EncryptionService>>,
    /// Calls running on this instance, by voice connection id.
    calls: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

pub struct AppDependencies {
    pub runner: Arc<dyn everruns_core::host::TurnBackend>,
    pub message_service: Arc<MessageService>,
    pub provider_resolver: Arc<ProviderResolverService>,
    pub event_delivery: EventDelivery,
    pub encryption: Option<Arc<crate::storage::EncryptionService>>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        auth: AuthState,
        feature_flags: FeatureFlags,
        dependencies: AppDependencies,
        host_composition: &everruns_core::host::HostComposition,
        built_in_harnesses: &[crate::records::BuiltInHarnessDefinition],
    ) -> Self {
        let registry = Arc::new(DbSessionResourceRegistry::new(db.clone()));
        let leased_resource_store =
            Arc::new(DbLeasedResourceStore::new(db.clone()).with_registry(registry))
                as Arc<dyn LeasedResourceStore>;
        Self {
            session_service: Arc::new(SessionService::with_registry(
                db.clone(),
                (*host_composition.capability_registry()).clone(),
            )),
            message_service: dependencies.message_service,
            event_service: EventService::new(db.clone(), dependencies.event_delivery.clone()),
            event_delivery: dependencies.event_delivery,
            db,
            auth,
            provider_resolver: dependencies.provider_resolver,
            leased_resource_store,
            feature_flags,
            runner: dependencies.runner,
            fallback_default_harness_name: crate::records::harness_for_role(
                built_in_harnesses,
                crate::records::BuiltInHarnessRole::Default,
            )
            .map(|h| h.name.clone()),
            encryption: dependencies.encryption,
            calls: Arc::default(),
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            self.encryption.clone(),
            self.auth.permission_resolver.clone(),
        )
        .with_session_service(self.session_service.clone())
        .with_event_service(Arc::new(self.event_service.clone()))
        .with_runner(self.runner.clone())
        .with_message_service(self.message_service.clone())
        .with_fallback_harness_name(self.fallback_default_harness_name.clone())
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/voice/calls", post(create_call))
        .route(
            "/v1/sessions/{session_id}/voice/{voice_connection_id}/end",
            post(end_call),
        )
        .route(
            "/v1/agents/{agent_id}/channels/{channel_id}/voice/calls",
            post(create_channel_call),
        )
        .with_state(state)
}

/// Request body for a session voice call (the chat microphone).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct VoiceCallRequest {
    /// The browser's WebRTC SDP offer.
    pub sdp: String,
    /// Voice channel whose settings the call uses (`appchan_…`). Must belong
    /// to the session's agent. When omitted, the agent's first voice channel
    /// is used, or the defaults when it has none.
    #[serde(default)]
    #[schema(example = "appchan_01933b5a000070008000000000000001")]
    pub channel_id: Option<String>,
    /// Realtime provider binding: the public id of the provider connection
    /// that serves the call (`prov_…`). When omitted, the org's default (or
    /// single) realtime provider is used.
    #[serde(default)]
    #[schema(example = "prov_01h…")]
    pub provider_id: Option<String>,
}

/// Request body for a call to an agent's voice channel.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ChannelVoiceCallRequest {
    /// The browser's WebRTC SDP offer.
    pub sdp: String,
    /// Continue an existing session of the channel's agent (for example a text
    /// conversation). When omitted, the call starts a new session.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Realtime provider binding, as in `VoiceCallRequest`.
    #[serde(default)]
    pub provider_id: Option<String>,
}

/// Request body for ending a call.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct VoiceEndRequest {
    /// Free-text reason recorded with the session-ended event.
    #[serde(default)]
    #[schema(example = "User hung up after refund confirmed.")]
    pub reason: Option<String>,
}

/// A started voice call.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceCallResponse {
    /// Prefixed public identifier of the voice connection.
    pub voice_connection_id: String,
    /// Provider-side call identifier.
    pub provider_call_id: Option<String>,
    /// Realtime provider type serving the call (e.g. `openai`).
    pub provider: String,
    /// Speech model.
    pub model: String,
    /// Provider voice.
    pub voice: String,
    /// Voice channel whose settings the call uses, when there is one.
    pub channel_id: Option<String>,
    /// When the call's lease expires (RFC 3339).
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// SDP answer that completes the browser's WebRTC handshake.
    pub answer_sdp: String,
}

/// Response body for ending a call.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceEndResponse {
    /// Prefixed public identifier of the voice connection that was ended.
    pub voice_connection_id: String,
    /// Current lifecycle status.
    pub status: String,
}

/// A channel call: the session it talks to and the call itself.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceSessionResponse {
    /// The session the call is attached to.
    pub session: Session,
    /// The started call.
    pub voice: VoiceCallResponse,
}

#[utoipa::path(
    description = "Start a voice call on a session (the chat microphone). Uses the settings of a voice channel of the session's agent.",
    post,
    path = "/v1/sessions/{session_id}/voice/calls",
    request_body = VoiceCallRequest,
    responses((status = 200, description = "Realtime WebRTC call started", body = VoiceCallResponse)),
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
    let session = authorize_session(&state, &org, session_id).await?;
    let (channel_id, config) =
        session_channel_config(&state, &org, &session, req.channel_id.as_deref()).await?;
    call::start(
        &state,
        &org,
        session_id,
        call::CallSettings {
            config,
            channel_id,
            provider_id: req.provider_id,
            sdp: req.sdp,
        },
    )
    .await
    .map(Json)
}

#[utoipa::path(
    description = "Call an agent's voice channel. Starts a new session for the call, or continues an existing session of the agent.",
    post,
    path = "/v1/agents/{agent_id}/channels/{channel_id}/voice/calls",
    request_body = ChannelVoiceCallRequest,
    responses((status = 201, description = "Session and realtime WebRTC call started", body = VoiceSessionResponse)),
    tag = "voice"
)]
pub async fn create_channel_call(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id)): Path<(String, String)>,
    Json(req): Json<ChannelVoiceCallRequest>,
) -> Result<(StatusCode, Json<VoiceSessionResponse>), (StatusCode, Json<ErrorResponse>)> {
    ensure_voice_enabled(&org)?;
    let ctx = state.ctx(&org);
    let channel = GetAgentChannel {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
    }
    .run(&ctx)
    .await?;
    let config = channel.voice_config().ok_or_else(|| {
        ErrorResponse::new("Not a voice channel").into_response(StatusCode::BAD_REQUEST)
    })?;
    if channel.status == crate::records::ChannelStatus::Disabled {
        return Err(
            ErrorResponse::new("Voice channel is disabled").into_response(StatusCode::CONFLICT)
        );
    }
    let agent = crate::domains::agent_channels::commands::find_agent(&ctx, &agent_id).await?;
    let session = match req.session_id.as_deref() {
        Some(session_id) => {
            let session = authorize_session(&state, &org, parse_session_id(session_id)?).await?;
            if session.agent_id != Some(agent.id) {
                return Err(ErrorResponse::new("Session belongs to another agent")
                    .into_response(StatusCode::BAD_REQUEST));
            }
            session
        }
        None => create_voice_session(&state, &org, agent.id).await?,
    };
    let voice = call::start(
        &state,
        &org,
        session.id,
        call::CallSettings {
            config,
            channel_id: Some(channel.public_id.to_string()),
            provider_id: req.provider_id,
            sdp: req.sdp,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(VoiceSessionResponse { session, voice }),
    ))
}

#[utoipa::path(
    description = "End a voice call.",
    post,
    path = "/v1/sessions/{session_id}/voice/{voice_connection_id}/end",
    request_body = VoiceEndRequest,
    responses((status = 200, description = "Voice call ended", body = VoiceEndResponse)),
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
    call::end(&state, session_id, &voice_connection_id, req.reason).await?;
    Ok(Json(VoiceEndResponse {
        voice_connection_id,
        status: "ended".to_string(),
    }))
}

/// The voice settings for a session call: the named channel, else the first
/// voice channel of the session's agent, else the defaults.
async fn session_channel_config(
    state: &AppState,
    org: &ResolvedOrg,
    session: &Session,
    channel_id: Option<&str>,
) -> Result<(Option<String>, VoiceChannelConfig), (StatusCode, Json<ErrorResponse>)> {
    let Some(agent_id) = session.agent_id else {
        if channel_id.is_some() {
            return Err(
                ErrorResponse::new("Session has no agent with voice channels")
                    .into_response(StatusCode::BAD_REQUEST),
            );
        }
        return Ok((None, VoiceChannelConfig::default()));
    };
    let channels = ListAgentChannels {
        agent_id: agent_id.to_string(),
    }
    .run(&state.ctx(org))
    .await?;
    let channel = match channel_id {
        Some(id) => Some(
            channels
                .iter()
                .find(|c| c.public_id.to_string() == id)
                .filter(|c| c.channel_type == ChannelType::Voice)
                .ok_or_else(|| ErrorResponse::not_found("Voice channel"))?,
        ),
        None => channels
            .iter()
            .find(|c| c.channel_type == ChannelType::Voice && c.enabled),
    };
    Ok(match channel {
        Some(channel) => (
            Some(channel.public_id.to_string()),
            channel.voice_config().unwrap_or_default(),
        ),
        None => (None, VoiceChannelConfig::default()),
    })
}

async fn create_voice_session(
    state: &AppState,
    org: &ResolvedOrg,
    agent_id: everruns_contracts::typed_id::AgentId,
) -> Result<Session, (StatusCode, Json<ErrorResponse>)> {
    Ok(CreateSession(crate::api::sessions::CreateSessionRequest {
        playground_user_id: None,
        source: None,
        workspace_id: None,
        harness_id: None,
        harness_name: None,
        agent_id: Some(agent_id),
        agent_name: None,
        virtual_user_id: None,
        title: Some("Voice call".to_string()),
        goal: None,
        locale: None,
        tags: vec!["voice".to_string()],
        model_id: None,
        capabilities: Vec::new(),
        sandbox: None,
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
    .run(&state.ctx(org))
    .await?)
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

fn parse_session_id(session_id: &str) -> Result<SessionId, (StatusCode, Json<ErrorResponse>)> {
    session_id.parse::<SessionId>().map_err(|_| {
        ErrorResponse::new("Invalid session ID").into_response(StatusCode::BAD_REQUEST)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_request_accepts_channel_and_provider_binding() {
        let req: VoiceCallRequest = serde_json::from_value(json!({
            "sdp": "v=0",
            "channel_id": "appchan_1",
            "provider_id": "prov_realtime_1"
        }))
        .expect("valid request");
        assert_eq!(req.channel_id.as_deref(), Some("appchan_1"));
        assert_eq!(req.provider_id.as_deref(), Some("prov_realtime_1"));
        let bare: VoiceCallRequest =
            serde_json::from_value(json!({ "sdp": "v=0" })).expect("sdp only");
        assert_eq!(bare.channel_id, None);
    }

    #[test]
    fn microphone_policy_follows_the_flag() {
        assert_eq!(
            microphone_permissions_policy_directive(true),
            "microphone=(self)"
        );
        assert_eq!(
            microphone_permissions_policy_directive(false),
            "microphone=()"
        );
    }
}
