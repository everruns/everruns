//! `POST /v1/e/{channel_id}/slack/interactivity` — a click on an approval card.
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
//! exists, and [`crate::services::approval_audit`] attributes it to the Slack
//! identity the API recorded — without approvals needing an identity path of
//! their own. See [`crate::slack_approvals`].

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use everruns_platform::SlackChannelConfig;
use serde::Deserialize;

use super::{SlackState, SlackTarget, resolve_slack_channel, verify_slack_signature};
use crate::api::ErrorResponse;
use crate::api::app_ingress::{IngressContext, IngressEndpoint};
use crate::middleware::RequestId;
use crate::slack_approvals::{
    ApprovalBinding, ApprovalDecision, ApprovalPolicy, ApprovalRequest, build_resolved_blocks,
};

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
}

#[derive(Debug, Deserialize)]
struct SlackAction {
    action_id: String,
    #[serde(default)]
    value: Option<String>,
}

pub(crate) async fn handle_slack_interactivity_endpoint(
    State(state): State<SlackState>,
    Path(channel_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    handle_slack_interactivity(
        state,
        SlackTarget::Endpoint(channel_id),
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
    slack_channel: &IngressEndpoint,
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
        .parse::<everruns_provider::typed_id::SessionId>()
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
    if session.endpoint_id != Some(slack_channel.internal_id) {
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

    let action_text = binding.action.clone();

    // Post the decision as the next user message. That is what `soft_approval`
    // is waiting for, so the turn resumes with no approval-specific plumbing.
    if let Err(error) = post_decision_message(
        state,
        app,
        session.id,
        session.org_id,
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

    // Rewrite the card so the buttons are gone. A second click then has nothing
    // to hit, which is how double clicks and stale cards stop being a problem
    // rather than needing their own bookkeeping.
    if let (Some(channel), Some(message)) = (payload.channel.as_ref(), payload.message.as_ref()) {
        let request = ApprovalRequest {
            action: action_text.clone(),
            question: None,
        };
        let blocks = build_resolved_blocks(&request, &decision.as_resolution(clicker));
        crate::slack_delivery::update_slack_message_blocks(
            &slack_config.bot_token,
            &channel.id,
            &message.ts,
            &decision.as_resolution(clicker),
            &blocks,
        )
        .await
        .unwrap_or_else(|error| {
            // The decision is already recorded; a card left showing its buttons
            // is cosmetic, and a second click posts a duplicate answer at worst.
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
    let Some(url) = payload.response_url.as_deref() else {
        return;
    };
    let body = serde_json::json!({
        "response_type": "ephemeral",
        "replace_original": false,
        "text": text,
    });
    if let Err(error) = reqwest::Client::new().post(url).json(&body).send().await {
        tracing::warn!(%error, "Failed to post an ephemeral Slack refusal");
    }
}

/// Post a decision into the session as the clicking Slack user.
///
/// Carries the same `ExternalActor` an ordinary Slack message would, which is
/// the whole reason a click can be attributed: `ApprovalAuditListener` resolves
/// the approver from the `input.message` event the API writes, so an approval
/// clicked in Slack lands in the audit log under the person who clicked it,
/// with no approval-specific identity path.
#[allow(clippy::too_many_arguments)]
async fn post_decision_message(
    state: &SlackState,
    app: &IngressContext,
    session_id: everruns_provider::typed_id::SessionId,
    org_id: i64,
    clicker: &str,
    slack_config: &SlackChannelConfig,
    text: &str,
    request_id: Option<String>,
) -> anyhow::Result<()> {
    use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};

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
        metadata: if actor_metadata.is_empty() {
            None
        } else {
            Some(actor_metadata)
        },
    };

    let participant =
        super::ensure_slack_user_participant(state, org_id, session_id, &actor).await?;

    let mut event_metadata = crate::execution_metadata::app_message_metadata(
        app.public_id,
        app.owner_principal_id,
        app.agent_identity_id,
    );
    if let Some(map) = event_metadata.as_object_mut() {
        map.insert(
            "participant_id".to_string(),
            serde_json::Value::String(participant.id.to_string()),
        );
    }

    state
        .message_service
        .create(
            crate::domains::messages::CreateMessageContext {
                org_id,
                user_id: None,
                harness_id: app.harness_id.uuid(),
                agent_id: app.agent_id.map(|agent_id| agent_id.uuid()),
                session_id: session_id.uuid(),
                event_metadata: Some(event_metadata),
                request_id,
            },
            CreateMessageRequest {
                message: InputMessage {
                    role: MessageRole::User,
                    content: vec![InputContentPart::text(text.to_string())],
                },
                addressed_participant_id: None,
                controls: None,
                metadata: None,
                tags: None,
                external_actor: Some(actor),
            },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack_approvals::{APPROVE_ACTION_ID, DECLINE_ACTION_ID};

    /// Slack posts interactivity as `payload=<url-encoded json>`, not as JSON.
    /// Getting this wrong makes every click a 400, so it is pinned against the
    /// wire shape rather than against our own serializer.
    fn form_body(payload: &serde_json::Value) -> Vec<u8> {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("payload", &payload.to_string())
            .finish();
        encoded.into_bytes()
    }

    fn binding_value(requester: &str) -> String {
        serde_json::json!({
            "s": "session_01a0277cffc07f42805458ce29db93a7",
            "u": requester,
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
    fn a_json_body_is_not_mistaken_for_a_form() {
        // Slack never posts interactivity as raw JSON. Accepting it would mean
        // the endpoint has a second, unsigned-in-practice shape.
        assert!(parse_interaction(br#"{"type":"block_actions"}"#).is_none());
    }

    #[test]
    fn a_body_without_a_payload_field_parses_to_nothing() {
        assert!(parse_interaction(b"not_payload=%7B%7D").is_none());
    }

    #[test]
    fn a_decline_click_parses_as_a_decline() {
        let payload =
            parse_interaction(&form_body(&click(DECLINE_ACTION_ID, "U_R", "U_R"))).expect("parses");
        assert_eq!(
            ApprovalDecision::from_action_id(&payload.actions[0].action_id),
            Some(ApprovalDecision::Declined)
        );
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

    #[test]
    fn a_block_action_from_another_card_is_not_a_decision() {
        let payload =
            parse_interaction(&form_body(&click("some_other_card", "U_R", "U_R"))).expect("parses");
        assert_eq!(
            ApprovalDecision::from_action_id(&payload.actions[0].action_id),
            None
        );
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
