// Shared state of the Agent Execution API routes (`super::agent_api`), the
// turn-cancel helper every channel uses, and the answers for the retired
// `api_endpoint` routes.
//
// Decisions (see knowledge/integrations/agent-execution-api.md, phase 6):
// - `api_endpoint` channels are converted to `api` channels at startup
//   (`domains::agent_channels::api_endpoint_conversion`) and their handler is
//   gone. The agent base URL `/v1/channels/{channel_id}` is unchanged, so a
//   caller with an old `evr_app_` key reaches the Agent API; a request in the
//   old wire shape is told so there (`agent_api::retired_shape`).
// - The App-scoped aliases `/v1/apps/{app_id}/api/{channel_id}/…` answer 410
//   Gone with a pointer to the Agent API, without resolving anything, so they
//   reveal nothing about which channels exist.

use std::sync::Arc;

use axum::{
    Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use everruns_core::events::{TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED};

use super::agent_api;
use crate::api::channel_auth::ChannelAuthVerifier;
use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::api::common::ErrorResponse;
use crate::api::sse::SseConnectionTracker;
use crate::common_dto::AllowedAction;
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::live_updates::event_delivery::EventDelivery;
use crate::services::EventService;
use crate::storage::{EncryptionService, StorageBackend};

/// Where a caller of a retired `api_endpoint` route or wire shape is sent.
pub(crate) const AGENT_API_DOCS_URL: &str =
    "https://everruns.com/docs/features/channels/#agent-api";

#[derive(Clone)]
pub struct ChannelApiState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub rate_limiter: ChannelRateLimiter,
    pub auth_verifier: ChannelAuthVerifier,
    /// Live session streams on `api` channels.
    pub event_service: Arc<EventService>,
    pub sse_tracker: Arc<SseConnectionTracker>,
    /// Checks runtime tokens minted by `/runtime-auth` for `api` channels.
    pub runtime_auth: Option<crate::auth::AuthState>,
}

impl ChannelApiState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_core::host::TurnBackend>,
        event_delivery: EventDelivery,
        sse_tracker: Arc<SseConnectionTracker>,
        rate_limiter: ChannelRateLimiter,
    ) -> Self {
        let message_service = Arc::new(MessageService::new(db.clone(), runner, event_delivery));
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: Arc::new(message_service.event_service().clone()),
            message_service,
            db,
            encryption,
            rate_limiter,
            auth_verifier: ChannelAuthVerifier::new(),
            sse_tracker,
            runtime_auth: None,
        }
    }

    pub fn with_runtime_auth(mut self, auth: crate::auth::AuthState) -> Self {
        self.runtime_auth = Some(auth);
        self
    }
}

pub fn routes(state: ChannelApiState) -> Router {
    Router::new()
        .route(
            "/v1/apps/{app_id}/api/{channel_id}/sessions",
            post(retired_app_route),
        )
        .route(
            "/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}",
            get(retired_app_route),
        )
        .route(
            "/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/messages",
            post(retired_app_route),
        )
        .route(
            "/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/cancel",
            post(retired_app_route),
        )
        .merge(agent_api::routes(state))
}

/// The problem a caller of a retired `api_endpoint` route or shape gets.
pub(crate) fn retired_api_endpoint_problem(status: StatusCode, detail: &str) -> Response {
    ErrorResponse::new(detail)
        .with_code("api_endpoint_retired")
        .with_type(AGENT_API_DOCS_URL)
        .with_action(
            AllowedAction::new("docs")
                .with_href(AGENT_API_DOCS_URL)
                .with_hint("Call the agent through the Agent API at /v1/channels/{channel_id}"),
        )
        .into_response(status)
        .into_response()
}

/// `/v1/apps/{app_id}/api/{channel_id}/…`: the App-scoped `api_endpoint`
/// aliases, retired with the channel type.
async fn retired_app_route() -> Response {
    retired_api_endpoint_problem(
        StatusCode::GONE,
        "API endpoint routes under /v1/apps/{app_id}/api/ are retired. The channel is now an \
         Agent API channel: call /v1/channels/{channel_id} with the same key.",
    )
}

/// Cancel a session's in-flight turn and record it as cancelled.
///
/// Shared by every internal caller that cancels on a user's behalf — the Agent
/// API, the Slack stop button (EVE-976) and Poppy — so the terminal-state race
/// guard below cannot drift between them.
///
/// `reason` names the caller and reaches only the stored event, never the user;
/// the Slack notice EVE-966 posts is a fixed string.
pub(crate) async fn cancel_session_turn_for(
    db: &Arc<StorageBackend>,
    message_service: &Arc<MessageService>,
    session_id: everruns_contracts::typed_id::SessionId,
    reason: &str,
) -> anyhow::Result<()> {
    use everruns_contracts::typed_id::TurnId;
    use everruns_core::events::{EventRequest, TurnCancelledData};

    // A worker running the turn records the cancel itself; see
    // `crate::turns::ActorTurnBackend`.
    match message_service.runner().cancel(session_id).await {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(err) => {
            tracing::warn!(session_id = %session_id, error = %err, "{reason}: cancel failed");
        }
    }

    // Skip emission if the turn already reached a terminal state, so a real
    // completed/failed turn is not race-flipped to canceled.
    if last_turn_finished(db, session_id).await? {
        return Ok(());
    }

    // The open turn is the one cancelled; a session whose message had not
    // started a turn yet falls back to ids derived from the session.
    let context = crate::turns::open_turn_context(db, session_id).await?;
    let turn_id = context
        .turn_id
        .unwrap_or_else(|| TurnId::from_uuid(session_id.uuid()));
    let event_service = message_service.event_service();
    let cancelled_event = EventRequest::new(
        session_id,
        context.clone(),
        TurnCancelledData {
            turn_id,
            reason: Some(reason.to_string()),
            usage: None,
        },
    );
    if let Err(err) = event_service.emit(cancelled_event).await {
        tracing::warn!(session_id = %session_id, error = %err, "{reason}: emit turn.cancelled failed");
    }
    Ok(())
}

/// Whether the session's latest turn event is a terminal one. A session with
/// no turn events, or whose latest is `turn.started`, still has work to cancel.
async fn last_turn_finished(
    db: &Arc<StorageBackend>,
    session_id: everruns_contracts::typed_id::SessionId,
) -> anyhow::Result<bool> {
    use everruns_core::events::TURN_STARTED;
    let filter_types = [TURN_STARTED, TURN_COMPLETED, TURN_FAILED, TURN_CANCELLED]
        .map(str::to_string)
        .to_vec();
    let events = db
        .list_events(session_id, None, None, &filter_types, &[], None, Some(500))
        .await?;
    Ok(events.last().is_some_and(|event| {
        matches!(
            event.event_type.as_str(),
            TURN_COMPLETED | TURN_FAILED | TURN_CANCELLED
        )
    }))
}

pub(crate) fn command_error_response(
    err: crate::domains::common::CommandError,
) -> (StatusCode, Json<ErrorResponse>) {
    use crate::domains::common::CommandErrorKind;
    match err {
        crate::domains::common::CommandError {
            kind: CommandErrorKind::BadRequest(msg),
            ..
        } => ErrorResponse::new(msg).into_response(StatusCode::BAD_REQUEST),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::Forbidden(msg),
            ..
        } => ErrorResponse::new(msg).into_response(StatusCode::FORBIDDEN),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::NotFound(_),
            ..
        } => {
            ErrorResponse::new("Channel or session not found").into_response(StatusCode::NOT_FOUND)
        }
        crate::domains::common::CommandError {
            kind: CommandErrorKind::Conflict(msg),
            ..
        } => ErrorResponse::new(msg).into_response(StatusCode::CONFLICT),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::RateLimited(msg),
            ..
        } => ErrorResponse::new(msg).into_response(StatusCode::TOO_MANY_REQUESTS),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::Unprocessable(msg),
            ..
        } => ErrorResponse::new(msg).into_response(StatusCode::UNPROCESSABLE_ENTITY),
        error @ crate::domains::common::CommandError {
            kind: CommandErrorKind::Unavailable(_),
            ..
        } => error.into(),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::Internal(error),
            ..
        } => {
            tracing::error!(error = %error, "Failed to handle channel session request");
            ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retired_app_routes_answer_gone_with_a_pointer() {
        let response = retired_app_route().await;
        assert_eq!(response.status(), StatusCode::GONE);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(problem["code"], "api_endpoint_retired");
        assert_eq!(problem["type"], AGENT_API_DOCS_URL);
        assert_eq!(problem["allowed_actions"][0]["href"], AGENT_API_DOCS_URL);
    }
}
