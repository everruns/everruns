//! `POST /v1/channels/{channel_id}/slack/interactivity` — a click on an approval card.
//!
//! Mirrors the events endpoint exactly: same signing secret, same unscoped
//! endpoint lookup, same generic 404 for anything not published. It accepts
//! state-changing input from the internet, so it runs the same signature and
//! replay checks and gives away nothing about what does or does not exist.
//!
//! Slack posts interactivity as `application/x-www-form-urlencoded` with a
//! single `payload` field holding the JSON, which is why this cannot reuse the
//! events endpoint's JSON parse. The signature still covers the raw body, so
//! verification is unchanged and happens *before* the body is interpreted.
//!
//! What a click does is deliberately small: it posts a message into the session.
//! `soft_approval`'s pause is answered by the user's next message, so a click
//! that becomes that message resumes the turn through the path that already
//! exists, and [`crate::domains::audit_logs::approval_listener`] attributes it to the Slack
//! identity the API recorded — without approvals needing an identity path of
//! their own. See [`crate::channels::slack::approvals`].

use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use serde::Deserialize;

use super::{SlackState, SlackTarget, resolve_slack_channel, verify_slack_signature};
use crate::api::ErrorResponse;
use crate::api::channel_ingress::{IngressChannel, IngressContext};
use crate::channels::slack::approvals::{
    ApprovalBinding, ApprovalDecision, ApprovalPolicy, ApprovalPrompt, build_resolved_blocks,
};
use crate::middleware::RequestId;

type ApiError = (StatusCode, Json<ErrorResponse>);

/// Slack's interactivity envelope, narrowed to the parts a card click uses.
#[derive(Debug, Deserialize)]
struct InteractionPayload {
    #[serde(rename = "type")]
    interaction_type: String,
    #[serde(default)]
    user: Option<SlackUserRef>,
    #[serde(default)]
    actions: Vec<SlackAction>,
    #[serde(default)]
    channel: Option<SlackChannelRef>,
    #[serde(default)]
    message: Option<SlackMessageRef>,
    #[serde(default)]
    response_url: Option<String>,
    #[serde(default)]
    container: Option<SlackContainerRef>,
}

#[derive(Debug, Deserialize)]
struct SlackContainerRef {
    message_ts: String,
}

#[derive(Debug, Deserialize)]
struct SlackUserRef {
    id: String,
}

#[derive(Debug, Deserialize)]
struct SlackChannelRef {
    id: String,
}

#[derive(Debug, Deserialize)]
struct SlackMessageRef {
    ts: String,
    #[serde(default)]
    thread_ts: Option<String>,
}

impl InteractionPayload {
    fn approval_event(
        &self,
        clicker: &str,
        binding: &ApprovalBinding,
    ) -> Option<super::SlackEvent> {
        if !binding.private {
            return self.decision_event(clicker);
        }
        let (channel, thread) = binding.private_destination.as_ref()?;
        if channel.is_empty() || thread.is_empty() || self.channel.as_ref()?.id != *channel {
            return None;
        }
        let ts = self
            .message
            .as_ref()
            .map(|m| &m.ts)
            .or_else(|| self.container.as_ref().map(|c| &c.message_ts))?;
        if ts.is_empty() {
            return None;
        }
        // Reuse the standard envelope construction with the signed original
        // destination, never the ephemeral message's synthetic timestamp.
        let routed = InteractionPayload {
            interaction_type: self.interaction_type.clone(),
            user: None,
            actions: vec![],
            response_url: None,
            container: None,
            channel: Some(SlackChannelRef {
                id: channel.clone(),
            }),
            message: Some(SlackMessageRef {
                ts: ts.clone(),
                thread_ts: Some(thread.clone()),
            }),
        };
        routed.decision_event(clicker)
    }

    /// The verified click carries the card's conversation, including its root
    /// thread. Never use the session's last conversation for a resumed turn.
    fn decision_event(&self, clicker: &str) -> Option<super::SlackEvent> {
        let channel = self.channel.as_ref()?;
        let message = self.message.as_ref()?;
        if channel.id.is_empty() || message.ts.is_empty() {
            return None;
        }
        Some(super::SlackEvent {
            event_type: "message".into(),
            user: Some(clicker.into()),
            channel: Some(channel.id.clone()),
            ts: Some(message.ts.clone()),
            thread_ts: Some(
                message
                    .thread_ts
                    .clone()
                    .unwrap_or_else(|| message.ts.clone()),
            ),
            text: None,
            title: None,
            bot_id: None,
            subtype: None,
            channel_type: None,
            assistant_thread: None,
            files: vec![],
            attachments: vec![],
            reaction: None,
            item: None,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SlackAction {
    action_id: String,
    #[serde(default)]
    value: Option<String>,
}

pub(crate) async fn handle_slack_interactivity_channel(
    State(state): State<SlackState>,
    Path(channel_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    handle_slack_interactivity(
        state,
        SlackTarget::Channel(channel_id),
        req_id,
        headers,
        body,
    )
    .await
}

pub(crate) async fn handle_slack_interactivity_legacy(
    State(state): State<SlackState>,
    Path(app_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    handle_slack_interactivity(state, SlackTarget::LegacyApp(app_id), req_id, headers, body).await
}

/// Acknowledge without acting.
///
/// Slack retries anything that is not a prompt 200, and a retry cannot fix a
/// click we have decided not to honour. Every non-actionable outcome — an
/// interaction we do not handle, a stale card, a refusal — is a 200 whose
/// meaning is carried in the thread instead.
fn ack() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

async fn handle_slack_interactivity(
    state: SlackState,
    target: SlackTarget,
    req_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let request_id = req_id.map(|Extension(r)| r.0);
    let (app, slack_channel) = resolve_slack_channel(&state, target).await?;
    let app_id = app.public_id.to_string();

    let slack_config: SlackChannelConfig =
        serde_json::from_value(slack_channel.channel_config.clone()).map_err(|e| {
            tracing::error!(app_id = %app_id, error = %e, "Invalid Slack channel config");
            ErrorResponse::new("Invalid Slack channel configuration")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;

    // THREAT[TM-SLACK-001]: signature and replay window, before the body is
    // interpreted at all. Identical to the events endpoint.
    verify_slack_signature(&headers, &body, &slack_config.signing_secret).map_err(|e| {
        tracing::warn!(app_id = %app_id, error = %e, "Slack interactivity signature verification failed");
        ErrorResponse::new("Invalid signature").into_response(StatusCode::UNAUTHORIZED)
    })?;

    let Some(payload) = parse_interaction(&body) else {
        tracing::warn!(app_id = %app_id, "Failed to parse Slack interactivity payload");
        return Err(
            ErrorResponse::new("Invalid request body").into_response(StatusCode::BAD_REQUEST)
        );
    };

    if payload.interaction_type != "block_actions" {
        // Shortcuts, view submissions, and the rest are not ours. Acknowledged
        // so Slack stops, ignored because nothing here handles them.
        tracing::debug!(
            app_id = %app_id,
            interaction_type = %payload.interaction_type,
            "Ignoring Slack interaction this endpoint does not handle"
        );
        return Ok(ack());
    }

    handle_block_action(
        &state,
        &app,
        &slack_channel,
        &slack_config,
        payload,
        request_id,
    )
    .await
}

/// A button click delivered over Socket Mode: the same handling as the HTTP
/// route after its signature check, which the socket replaces (the
/// connection was opened with the channel's own app token).
pub(crate) async fn handle_socket_interaction(
    state: &SlackState,
    app: &IngressContext,
    slack_channel: &IngressChannel,
    slack_config: &SlackChannelConfig,
    payload: serde_json::Value,
) {
    let payload: InteractionPayload = match serde_json::from_value(payload) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(channel_id = %slack_channel.public_id, %error, "Failed to parse Slack Socket Mode interaction");
            return;
        }
    };
    if payload.interaction_type != "block_actions" {
        return;
    }
    if let Err(error) =
        handle_block_action(state, app, slack_channel, slack_config, payload, None).await
    {
        tracing::warn!(channel_id = %slack_channel.public_id, ?error, "Slack Socket Mode interaction failed");
    }
}

/// Slack posts `payload=<json>` as form-encoded, not as a JSON body.
fn parse_interaction(body: &[u8]) -> Option<InteractionPayload> {
    let form = std::str::from_utf8(body).ok()?;
    let raw = url::form_urlencoded::parse(form.as_bytes())
        .find(|(key, _)| key == "payload")
        .map(|(_, value)| value.into_owned())?;
    serde_json::from_str(&raw).ok()
}

async fn handle_block_action(
    state: &SlackState,
    app: &IngressContext,
    slack_channel: &IngressChannel,
    slack_config: &SlackChannelConfig,
    payload: InteractionPayload,
    request_id: Option<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let app_id = app.public_id.to_string();

    let Some(action) = payload.actions.first() else {
        return Ok(ack());
    };
    let Some(decision) = ApprovalDecision::from_action_id(&action.action_id) else {
        // A block action from some other card. Not ours to answer.
        return Ok(ack());
    };
    let Some(clicker) = payload.user.as_ref().map(|user| user.id.as_str()) else {
        // Slack always names the clicking user on a block action. Without one
        // there is no identity to authorize, so the click cannot be honoured.
        tracing::warn!(app_id = %app_id, "Slack block action carried no user");
        return Ok(ack());
    };
    let Some(binding) = action
        .value
        .as_deref()
        .and_then(|value| serde_json::from_str::<ApprovalBinding>(value).ok())
    else {
        tracing::warn!(app_id = %app_id, "Slack approval click carried no usable binding");
        return Ok(ack());
    };

    // THREAT[TM-TENANT-001]: the binding rode through Slack, so it names a
    // session this endpoint has not yet proven belongs to this endpoint's org.
    // Resolve it org-scoped before anything is written.
    let Ok(session_id) = binding
        .session_id
        .parse::<everruns_contracts::typed_id::SessionId>()
    else {
        tracing::warn!(app_id = %app_id, "Slack approval click named an unparseable session");
        return Ok(ack());
    };
    let session = match state.db.get_session(app.org_id, session_id).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            tracing::warn!(
                app_id = %app_id,
                %session_id,
                "Slack approval click named a session this endpoint cannot reach"
            );
            return Ok(ack());
        }
        Err(error) => {
            tracing::error!(app_id = %app_id, %error, "Failed to load session for Slack approval");
            return Err(ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR));
        }
    };

    // The session must belong to *this* endpoint, not merely to the same org:
    // one agent can carry two Slack endpoints, and a click on one must not
    // answer a pause raised through the other.
    if session.channel_id != Some(slack_channel.internal_id) {
        tracing::warn!(
            app_id = %app_id,
            %session_id,
            "Slack approval click named a session from another endpoint"
        );
        return Ok(ack());
    }

    let policy = ApprovalPolicy::default();
    if !policy.allows(&binding, clicker) {
        // Refusals are visible rather than silent: the person who clicked is
        // told why, in the thread, and the decision is not recorded.
        tracing::info!(
            app_id = %app_id,
            %session_id,
            clicker = %clicker,
            "Refused a Slack approval click from someone other than the requester"
        );
        respond_ephemeral(&payload, &policy.refusal(&binding)).await;
        return Ok(ack());
    }

    let Some(turn_id) = binding.turn_id.as_deref() else {
        tracing::warn!(app_id = %app_id, %session_id, "Slack approval click had no originating turn");
        return Ok(ack());
    };

    let Some(event) = payload.approval_event(clicker, &binding) else {
        tracing::warn!(app_id = %app_id, %session_id, "Slack approval click had no reply destination");
        return Ok(ack());
    };
    if slack_config
        .channel_id
        .as_deref()
        .is_some_and(|id| event.channel.as_deref() != Some(id))
    {
        return Ok(ack());
    }

    // THREAT[TM-SLACK-004]: claim the exact pending card before creating the
    // consent message. Storage serializes this with event creation, so retries,
    // concurrent clicks, prose replies, and superseding asks all fail closed.
    let claimed = state
        .db
        .claim_slack_approval_card(session.id, &binding.card_id, turn_id)
        .await
        .map_err(|error| {
            tracing::error!(app_id = %app_id, %session_id, %error, "Failed to claim Slack approval card");
            ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    if !claimed {
        tracing::info!(app_id = %app_id, %session_id, card_id = %binding.card_id, "Ignored stale or already answered Slack approval card");
        respond_ephemeral(&payload, "This approval request is no longer pending.").await;
        return Ok(ack());
    }

    let action_text = binding.action.clone();

    // Post the decision as the next user message. That is what `soft_approval`
    // is waiting for, so the turn resumes with no approval-specific plumbing.
    if let Err(error) = post_decision_message(
        state,
        app,
        slack_channel,
        &session,
        &event,
        clicker,
        slack_config,
        &decision.as_message(&action_text),
        request_id,
    )
    .await
    {
        tracing::error!(app_id = %app_id, %session_id, %error, "Failed to record Slack approval decision");
        return Err(ErrorResponse::new("Internal server error")
            .into_response(StatusCode::INTERNAL_SERVER_ERROR));
    }

    // Rewrite the card for clarity. Single-use enforcement is server-side, so
    // a failed update cannot make the already-consumed card actionable again.
    if binding.private {
        // Ephemeral cards cannot be updated with chat.update. Even if the
        // response URL expired, never copy their contents into the channel.
        respond_private_resolution(&payload, &decision.as_resolution(clicker)).await;
    } else if let (Some(channel), Some(message)) =
        (payload.channel.as_ref(), payload.message.as_ref())
    {
        let request = ApprovalPrompt {
            action: action_text.clone(),
            question: None,
            turn_id: None,
        };
        let blocks = build_resolved_blocks(&request, &decision.as_resolution(clicker));
        everruns_integrations::slack_channel::web_api::update_slack_message_blocks(
            &slack_config.bot_token,
            &channel.id,
            &message.ts,
            &decision.as_resolution(clicker),
            &blocks,
        )
        .await
        .unwrap_or_else(|error| {
            // The decision is already recorded and the card is consumed; a
            // card left showing its buttons is cosmetic.
            tracing::warn!(app_id = %app_id, %error, "Failed to resolve the Slack approval card");
        });
    }

    tracing::info!(
        app_id = %app_id,
        %session_id,
        clicker = %clicker,
        approved = decision == ApprovalDecision::Approved,
        "Recorded a Slack approval decision"
    );
    Ok(ack())
}

/// Tell the clicker something only they need to see.
///
/// Best effort: `response_url` is Slack's own ephemeral channel and needs no
/// token, but a refusal that fails to render is still a refusal — nothing was
/// recorded either way.
async fn respond_ephemeral(payload: &InteractionPayload, text: &str) {
    send_private_response(payload, text, false).await;
}

/// Retire a private card using Slack's short-lived callback. Its URL is never
/// persisted; durable single-use enforcement does not depend on this update.
async fn respond_private_resolution(payload: &InteractionPayload, text: &str) {
    send_private_response(payload, text, true).await;
}

async fn send_private_response(payload: &InteractionPayload, text: &str, replace: bool) {
    let Some(url) = payload.response_url.as_deref().and_then(slack_response_url) else {
        return;
    };
    let body = private_response_body(text, replace);
    // Do not follow redirects or log a callback URL (it is a bearer secret).
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return;
    };
    match client.post(url).json(&body).send().await {
        Ok(response) if response.status().is_success() => {}
        _ => tracing::warn!("Failed to post a private Slack approval response"),
    }
}

fn private_response_body(text: &str, replace: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "response_type": "ephemeral",
        "replace_original": replace,
        "text": text,
    });
    if replace {
        body["blocks"] = serde_json::json!([]);
    }
    body
}

fn slack_response_url(raw: &str) -> Option<url::Url> {
    let url = url::Url::parse(raw).ok()?;
    (url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some("hooks.slack.com" | "hooks.slack-gov.com")
        )
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.path().starts_with("/actions/"))
    .then_some(url)
}

/// Post a decision into the session as the clicking Slack user.
///
/// Carries the same `ExternalActor` an ordinary Slack message would, which is
/// the whole reason a click can be attributed: `ApprovalAuditListener` resolves
/// the approver from the `input.message` event the API writes, so an approval
/// clicked in Slack lands in the audit log under the person who clicked it,
/// with no approval-specific identity path. It goes through the channel host
/// like any message, so the clicker joins the session and the turn it resumes
/// is delivered back to the thread.
#[allow(clippy::too_many_arguments)]
async fn post_decision_message(
    state: &SlackState,
    app: &IngressContext,
    slack_channel: &IngressChannel,
    session: &crate::storage::SessionRow,
    event: &super::SlackEvent,
    clicker: &str,
    slack_config: &SlackChannelConfig,
    text: &str,
    request_id: Option<String>,
) -> anyhow::Result<()> {
    use crate::channels::host::ServerChannelScope;
    use crate::channels::slack::delivery::{SLACK_RECIPIENT_TEAM_ID, SLACK_RECIPIENT_USER_ID};
    use everruns_contracts::runtime::channel::DeliveryTarget;
    use everruns_contracts::runtime::message_retriever::InputMessage;

    let channels = state
        .channels
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Slack has no channel host"))?;
    let resolved = channels
        .resolver
        .channel_for(ServerChannelScope {
            request_id,
            ..ServerChannelScope::new(app.clone(), slack_channel.clone())
        })
        .ok_or_else(|| anyhow::anyhow!("Slack channel config did not deserialize"))?;

    let mut actor_metadata = std::collections::HashMap::new();
    if let Some(team_id) = slack_config.team_id.as_ref() {
        actor_metadata.insert("team_id".to_string(), team_id.clone());
    }
    let actor = everruns_core::ExternalActor {
        actor_id: clicker.to_string(),
        // The click carries an id, not a profile. Resolving a display name here
        // would cost a `users.info` round trip on the interactivity path, whose
        // budget is Slack's 3-second timeout; the id is enough to attribute.
        actor_name: None,
        source: "slack".to_string(),
        metadata: (!actor_metadata.is_empty()).then_some(actor_metadata),
    };
    let mut message = InputMessage::user(text);
    message.metadata = Some(super::slack_message_metadata(app, slack_channel, event));
    message.external_actor = Some(actor);

    let mut target = DeliveryTarget::new(
        event.channel.clone().unwrap_or_default(),
        event.thread_ts.clone().unwrap_or_default(),
    );
    target
        .extra
        .insert(SLACK_RECIPIENT_USER_ID.into(), clicker.to_string());
    if let Some(team) = &slack_config.team_id {
        target
            .extra
            .insert(SLACK_RECIPIENT_TEAM_ID.into(), team.clone());
    }
    channels
        .host
        .send(&resolved, &session.id.to_string(), message, target)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::slack::approvals::APPROVE_ACTION_ID;

    /// Slack posts interactivity as `payload=<url-encoded json>`, not as JSON.
    /// Getting this wrong makes every click a 400, so it is pinned against the
    /// wire shape rather than against our own serializer.
    #[tokio::test]
    async fn private_approval_checks_requester_endpoint_and_durable_single_use() {
        use crate::channels::slack::host_tests::{Slack, message, session_of, slack_config};
        use crate::domains::agent_channels::record::SessionBinding;
        use everruns_contracts::typed_id::SessionId;
        let mut config = slack_config(SessionBinding::Thread);
        config.private_approvals = true;
        let slack = Slack::new(config.clone()).await;
        let response = slack.send(message("C1", "1.0", None)).await.unwrap();
        let session_id: SessionId = session_of(&response).parse().unwrap();
        sqlx::query("INSERT INTO events (session_id, sequence, event_type, ts, context, data, metadata, tags) VALUES ($1, allocate_event_sequence($1), 'tool.completed', NOW(), $2, $3, NULL, '{}')")
            .bind(session_id.uuid())
            .bind(serde_json::json!({"turn_id": "private-turn"}))
            .bind(serde_json::json!({"tool_name": "request_approval"}))
            .execute(slack.db.database().pool()).await.unwrap();
        let mut binding = ApprovalBinding::for_new_card(
            session_id.uuid(),
            "U1",
            Some("private-turn".into()),
            "private action".into(),
        );
        binding.private = true;
        binding.private_destination = Some(("C1".into(), "1.0".into()));
        let mut body = click(APPROVE_ACTION_ID, "U2", "U1");
        body.as_object_mut().unwrap().remove("response_url");
        body.as_object_mut().unwrap().remove("message");
        body["container"] = serde_json::json!({"message_ts": "private-message"});
        body["actions"][0]["value"] = serde_json::json!(serde_json::to_string(&binding).unwrap());
        let handle =
            |body: serde_json::Value| serde_json::from_value::<InteractionPayload>(body).unwrap();
        // Unauthorized click must not consume the pending request.
        let _ = handle_block_action(
            &slack.state,
            &slack.context,
            &slack.channel,
            &config,
            handle(body.clone()),
            None,
        )
        .await
        .unwrap();
        let consumed = || {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events WHERE session_id = $1 AND event_type = 'slack.approval.consumed'").bind(session_id.uuid())
        };
        assert_eq!(
            consumed()
                .fetch_one(slack.db.database().pool())
                .await
                .unwrap(),
            0
        );
        // A different endpoint in the same organization cannot consume it.
        body["user"]["id"] = serde_json::json!("U1");
        let other = Slack::new(config.clone()).await;
        let _ = handle_block_action(
            &slack.state,
            &slack.context,
            &other.channel,
            &config,
            handle(body.clone()),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            consumed()
                .fetch_one(slack.db.database().pool())
                .await
                .unwrap(),
            0
        );
        // Changing delivery settings does not change the signed card's visibility.
        config.private_approvals = false;
        let _ = handle_block_action(
            &slack.state,
            &slack.context,
            &slack.channel,
            &config,
            handle(body.clone()),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            consumed()
                .fetch_one(slack.db.database().pool())
                .await
                .unwrap(),
            1
        );
        // The single-use decision is in storage, not in the ephemeral message.
        let _ = handle_block_action(
            &slack.state,
            &slack.context,
            &slack.channel,
            &config,
            handle(body),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            consumed()
                .fetch_one(slack.db.database().pool())
                .await
                .unwrap(),
            1
        );
    }

    #[test]
    fn ephemeral_click_resumes_original_thread_without_message_body() {
        let mut body = click(APPROVE_ACTION_ID, "U_R", "U_R");
        body.as_object_mut().unwrap().remove("message");
        body["container"] = serde_json::json!({"message_ts": "ephemeral-ts"});
        let mut binding: ApprovalBinding = serde_json::from_str(&binding_value("U_R")).unwrap();
        binding.private = true;
        binding.private_destination = Some(("C1".into(), "1.0".into()));
        let payload = parse_interaction(&form_body(&body)).unwrap();
        let event = payload.approval_event("U_R", &binding).unwrap();
        assert_eq!(event.thread_ts.as_deref(), Some("1.0"));
        assert_eq!(event.channel.as_deref(), Some("C1"));
        binding.private_destination = Some(("C_OTHER".into(), "1.0".into()));
        assert!(payload.approval_event("U_R", &binding).is_none());
        binding.private_destination = None;
        assert!(payload.approval_event("U_R", &binding).is_none());
    }

    #[test]
    fn private_resolution_keeps_visibility_and_removes_controls() {
        let body = private_response_body("Approved", true);
        assert_eq!(body["response_type"], "ephemeral");
        assert_eq!(body["replace_original"], true);
        assert_eq!(body["blocks"], serde_json::json!([]));
        assert_eq!(body["text"], "Approved");
    }

    #[test]
    fn private_callback_accepts_only_slack_action_urls() {
        assert!(slack_response_url("https://hooks.slack.com/actions/T1/secret").is_some());
        for url in [
            "http://hooks.slack.com/actions/secret",
            "https://hooks.slack.com.evil.test/actions/secret",
            "https://user@hooks.slack.com/actions/secret",
            "https://hooks.slack.com:444/actions/secret",
            "https://example.com/actions/secret",
        ] {
            assert!(slack_response_url(url).is_none());
        }
    }

    fn form_body(payload: &serde_json::Value) -> Vec<u8> {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("payload", &payload.to_string())
            .finish();
        encoded.into_bytes()
    }

    fn binding_value(requester: &str) -> String {
        serde_json::json!({
            "i": "0199-card-id",
            "s": "session_01a0277cffc07f42805458ce29db93a7",
            "u": requester,
            "t": "0199-turn-id",
            "a": "delete the staging database",
        })
        .to_string()
    }

    fn click(action_id: &str, clicker: &str, requester: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "block_actions",
            "user": { "id": clicker },
            "channel": { "id": "C1" },
            "message": { "ts": "1.2" },
            "response_url": "https://hooks.slack.example/actions/1",
            "actions": [{
                "action_id": action_id,
                "value": binding_value(requester),
            }]
        })
    }

    #[test]
    fn a_slack_form_body_parses() {
        let payload =
            parse_interaction(&form_body(&click(APPROVE_ACTION_ID, "U_R", "U_R"))).expect("parses");
        assert_eq!(payload.interaction_type, "block_actions");
        assert_eq!(payload.user.as_ref().map(|u| u.id.as_str()), Some("U_R"));
        assert_eq!(payload.channel.as_ref().map(|c| c.id.as_str()), Some("C1"));
        assert_eq!(payload.message.as_ref().map(|m| m.ts.as_str()), Some("1.2"));
        assert_eq!(payload.actions.len(), 1);
        assert_eq!(payload.actions[0].action_id, APPROVE_ACTION_ID);
    }

    #[test]
    fn approval_resume_keeps_the_signed_card_thread() {
        let mut body = click(APPROVE_ACTION_ID, "U_R", "U_R");
        body["message"]["thread_ts"] = serde_json::json!("1.0");
        let payload = parse_interaction(&form_body(&body)).unwrap();
        let event = payload.decision_event("U_R").unwrap();
        assert_eq!(event.channel.as_deref(), Some("C1"));
        assert_eq!(event.thread_ts.as_deref(), Some("1.0"));
        assert_eq!(event.user.as_deref(), Some("U_R"));
        assert_eq!(event.ts.as_deref(), Some("1.2"));
        body["message"].as_object_mut().unwrap().remove("thread_ts");
        let payload = parse_interaction(&form_body(&body)).unwrap();
        assert_eq!(
            payload.decision_event("U_R").unwrap().thread_ts.as_deref(),
            Some("1.2")
        );
        body.as_object_mut().unwrap().remove("channel");
        assert!(
            parse_interaction(&form_body(&body))
                .unwrap()
                .decision_event("U_R")
                .is_none()
        );
    }

    #[test]
    fn a_json_body_is_not_mistaken_for_a_form() {
        // Slack never posts interactivity as raw JSON. Accepting it would mean
        // the endpoint has a second, unsigned-in-practice shape.
        assert!(parse_interaction(br#"{"type":"block_actions"}"#).is_none());
    }

    #[test]
    fn a_body_without_a_payload_field_parses_to_nothing() {
        assert!(parse_interaction(b"not_payload=%7B%7D").is_none());
    }

    /// The binding survives the round trip through Slack's form encoding — the
    /// one place an escaping bug would silently unbind every card.
    #[test]
    fn the_binding_survives_form_encoding() {
        let payload =
            parse_interaction(&form_body(&click(APPROVE_ACTION_ID, "U_CLICKER", "U_REQ")))
                .expect("parses");
        let binding: ApprovalBinding =
            serde_json::from_str(payload.actions[0].value.as_deref().expect("value"))
                .expect("the value is still the binding");
        assert_eq!(binding.requester, "U_REQ");
        assert_eq!(binding.action, "delete the staging database");

        // And the authorization it exists for still refuses the bystander.
        assert!(!ApprovalPolicy::default().allows(&binding, "U_CLICKER"));
        assert!(ApprovalPolicy::default().allows(&binding, "U_REQ"));
    }

    #[test]
    fn an_interaction_we_do_not_handle_still_parses() {
        // Shortcuts and view submissions reach the same URL. They must parse
        // (so they can be acknowledged) and be recognisable as not ours.
        let payload = parse_interaction(&form_body(&serde_json::json!({
            "type": "view_submission",
            "user": { "id": "U_R" }
        })))
        .expect("parses");
        assert_eq!(payload.interaction_type, "view_submission");
        assert!(payload.actions.is_empty());
    }

    /// The signed-click path's gate: the same body with a wrong signature must
    /// not verify, and with the right one must.
    #[test]
    fn the_signature_covers_the_form_body() {
        use axum::http::{HeaderMap, HeaderValue};

        let secret = "test_signing_secret";
        let body = form_body(&click(APPROVE_ACTION_ID, "U_R", "U_R"));
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let signature = super::super::tests_support::make_signature(
            secret,
            &timestamp,
            &String::from_utf8_lossy(&body),
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Slack-Request-Timestamp",
            HeaderValue::from_str(&timestamp).unwrap(),
        );
        headers.insert(
            "X-Slack-Signature",
            HeaderValue::from_str(&signature).unwrap(),
        );
        assert!(verify_slack_signature(&headers, &body, secret).is_ok());

        // A body edited after signing — the shape a forged binding would take.
        let tampered = form_body(&click(APPROVE_ACTION_ID, "U_ATTACKER", "U_ATTACKER"));
        assert!(verify_slack_signature(&headers, &tampered, secret).is_err());
    }
}
