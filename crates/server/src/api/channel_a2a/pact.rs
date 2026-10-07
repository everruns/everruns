// PACT Identity profile (PACT 1.0, github.com/openpactprotocol/openpactprotocol)
// on an A2A endpoint: a personal agent talks to the endpoint's agent for one
// of its users, over A2A 1.0 HTTP+JSON at `/v1/a2a/{channel_id}`.
//
// Design Decisions:
// - Separate routes, not a mode of the channel's A2A URL. PACT changes what
//   the same operations mean (a reply is a Message, there are no tasks, the
//   version header is ignored), so one URL with two meanings would make every
//   handler branch. The channel's ordinary A2A URL keeps working unchanged.
// - Routing happens before authentication (§2.2): an unknown channel, a
//   channel without `pact`, or a path that is not a PACT operation is a plain
//   `404`/`405` even with a valid token. Every authentication failure is one
//   `401` with `WWW-Authenticate: Bearer realm="a2a"` and no body (§3.4).
// - One conversation per `contextId`. The `contextId` is the session id, and
//   the session carries the caller's tag (`pact_identity.rs`), so a context
//   continues only for the same channel and the same (personal agent, `sub`);
//   anything else is the same `INVALID_PARAMS` as an unknown context (§4.2).
// - Retries (§4.3): the caller's `messageId` is stored on the user message,
//   and the reply is read back from the session's events, so a repeated
//   `messageId` returns the stored reply without running the agent again and
//   needs no table of its own.
// See `knowledge/integrations/a2a-channel.md`.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Extension, Router,
    body::Bytes,
    extract::{ConnectInfo, OriginalUri, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{
    INPUT_MESSAGE, InputMessageData, OUTPUT_MESSAGE_COMPLETED, OutputMessageCompletedData,
    TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::http_json::{a2a_error, a2a_json};
use super::{AuthorizedA2a, ChannelA2aState, pact_identity, task_view};
use crate::api::channel_ingress;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::agent_channels::invocation::A2A_MESSAGE_ID_METADATA;
use crate::domains::agent_channels::{A2aInvocationRequest, invoke_channel_a2a_with_hook};
use crate::records::agent_channel::PactProfileConfig;
use crate::records::pact_delegation::PactDelegationConfig;

const BASE: &str = "/v1/a2a/{channel_id}";

const TASK_NOT_FOUND: i64 = -32001;
const PUSH_NOTIFICATION_NOT_SUPPORTED: i64 = -32003;
const UNSUPPORTED_OPERATION: i64 = -32004;
const CONTENT_TYPE_NOT_SUPPORTED: i64 = -32005;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL: i64 = -32603;

/// `pageSize` bounds and default for the (always empty) task list (§2.2).
const MAX_PAGE_SIZE: u64 = 100;
const DEFAULT_PAGE_SIZE: u64 = 50;

/// Event tail searched for a message and its reply. Only input, output and
/// turn-end events are read, so this spans dozens of turns.
const REPLY_EVENT_TAIL: i32 = 400;

pub(super) fn routes(router: Router<ChannelA2aState>) -> Router<ChannelA2aState> {
    router
        .route(
            &format!("{BASE}/.well-known/agent-card.json"),
            get(agent_card),
        )
        .route(&format!("{BASE}/message:send"), post(send_message))
        .route(&format!("{BASE}/message:stream"), post(unsupported))
        .route(&format!("{BASE}/tasks"), get(list_tasks))
        .route(
            &format!("{BASE}/tasks/{{task}}"),
            get(get_task).post(task_action),
        )
        .route(
            &format!("{BASE}/tasks/{{task}}/pushNotificationConfigs"),
            get(push_unsupported).post(push_unsupported),
        )
        .route(
            &format!("{BASE}/tasks/{{task}}/pushNotificationConfigs/{{config}}"),
            get(push_unsupported).delete(push_unsupported),
        )
        .route(&format!("{BASE}/extendedAgentCard"), get(unsupported))
}

pub(super) type Peer = Option<Extension<ConnectInfo<std::net::SocketAddr>>>;
type ReqId = Option<Extension<crate::middleware::RequestId>>;

/// A PACT request that passed routing and authentication.
struct Caller {
    auth: AuthorizedA2a,
    /// The id `invoke_channel_a2a_with_hook` resolves the channel's app by.
    legacy_app_id: String,
    user: pact_identity::PersonalAgentUser,
}

/// The live A2A channel at `channel_id` and its PACT profile, or `None` when
/// there is no such PACT endpoint. Every `None` is the same `404`.
pub(super) async fn pact_channel(
    state: &ChannelA2aState,
    channel_id: &str,
) -> Result<
    Option<(
        channel_ingress::IngressContext,
        channel_ingress::IngressChannel,
        crate::records::A2aChannelConfig,
        PactProfileConfig,
    )>,
    Response,
> {
    let Some((app, channel)) =
        channel_ingress::resolve_channel(&state.db, state.encryption.as_ref(), channel_id)
            .await
            .map_err(|err| super::internal_error(err).into_response())?
    else {
        return Ok(None);
    };
    if channel.channel_type != crate::records::ChannelType::A2a
        || channel_ingress::channel_liveness(&app, &channel).is_err()
    {
        return Ok(None);
    }
    let Some(config) = channel.a2a_config() else {
        return Ok(None);
    };
    let Some(pact) = config.pact.clone() else {
        return Ok(None);
    };
    Ok(Some((app, channel, config, pact)))
}

/// Resolve the endpoint, then authenticate the personal agent and apply the
/// channel's rate limit. `Err` is the finished response.
async fn admit(
    state: &ChannelA2aState,
    channel_id: &str,
    headers: &HeaderMap,
    peer: Peer,
) -> Result<Caller, Response> {
    let Some((app, channel, config, pact)) = pact_channel(state, channel_id).await? else {
        return Err(super::not_found().into_response());
    };
    let user = pact_identity::verify(&state.auth_verifier, &pact, headers)
        .await
        .map_err(|()| unauthorized())?;
    rate_limit(state, &app, &channel, &config, headers, peer).await?;
    Ok(Caller {
        legacy_app_id: app.legacy_app_id(),
        auth: AuthorizedA2a {
            org_id: app.org_id,
            app_public_id: app.public_id.to_string(),
            channel_public_id: channel.public_id,
            session_mode: config.session_mode,
        },
        user,
    })
}

/// THREAT[TM-A2A-013]: the channel's per-IP cap applies to PACT traffic
/// too, checked after authentication so an anonymous caller cannot grow the
/// limiter or probe the endpoint through it.
pub(super) async fn rate_limit(
    state: &ChannelA2aState,
    app: &channel_ingress::IngressContext,
    channel: &channel_ingress::IngressChannel,
    config: &crate::records::A2aChannelConfig,
    headers: &HeaderMap,
    peer: Peer,
) -> Result<(), Response> {
    let channel_scope = format!("{}:{}", app.public_id, channel.public_id);
    if let Some(limit) = config.rate_limit_per_minute
        && limit > 0
    {
        let client_ip = extract_client_ip_from_parts(peer.map(|Extension(ci)| ci.0), headers);
        if state
            .rate_limiter
            .check(&channel_scope, client_ip, limit)
            .await
            .is_err()
        {
            return Err(
                super::too_many_requests("A2A rate limit exceeded for this endpoint")
                    .into_response(),
            );
        }
    }
    Ok(())
}

/// §3.4: one `401` for every authentication failure, with no A2A body.
pub(super) fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"a2a\""),
        )],
    )
        .into_response()
}

async fn agent_card(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let (app, config, pact) = match pact_channel(&state, &channel_id).await {
        Ok(Some((app, _, config, pact))) => (app, config, pact),
        Ok(None) => return super::not_found().into_response(),
        Err(response) => return response,
    };
    let path = uri.path();
    let interface_url = super::agent_card::absolute_url(
        &headers,
        path.strip_suffix("/.well-known/agent-card.json")
            .unwrap_or(path),
    );
    let name = config
        .agent_card_name
        .clone()
        .unwrap_or_else(|| app.name.clone());
    let description = config
        .agent_card_description
        .clone()
        .or_else(|| app.description.clone())
        .unwrap_or_default();
    (
        [(header::CONTENT_TYPE, "application/json")],
        pact_card(
            &name,
            &description,
            &app.name,
            &interface_url,
            pact.delegation.as_ref(),
        )
        .to_string(),
    )
        .into_response()
}

/// The PACT Agent Card (§2.1).
///
/// Design Decision: the personal-agent JWT scheme is declared twice, as
/// `paJwt` (the spec's name) and `platformJwt` (the name PACT's conformance
/// suite and reference clients look up), each alone in its own requirement.
/// Both describe the same token, so either lookup finds it.
///
/// With delegation (§5.1) the card adds a `userDelegation` OAuth 2.0
/// device-code scheme and a requirement pairing it with `paJwt`; the
/// JWT-only requirements stay, so a personal agent may always talk with §3
/// alone.
fn pact_card(
    name: &str,
    description: &str,
    skill_name: &str,
    interface_url: &str,
    delegation: Option<&PactDelegationConfig>,
) -> Value {
    let jwt = json!({
        "httpAuthSecurityScheme": { "scheme": "Bearer", "bearerFormat": "JWT" }
    });
    let mut schemes = json!({ "paJwt": jwt, "platformJwt": jwt });
    let mut requirements = vec![
        json!({ "schemes": { "paJwt": { "list": [] } } }),
        json!({ "schemes": { "platformJwt": { "list": [] } } }),
    ];
    if let Some(delegation) = delegation {
        schemes["userDelegation"] = super::pact_oauth::security_scheme(interface_url, delegation);
        requirements.push(json!({
            "schemes": { "paJwt": { "list": [] }, "userDelegation": { "list": [] } }
        }));
    }
    json!({
        "name": name,
        "description": description,
        "version": super::A2A_AGENT_VERSION,
        "supportedInterfaces": [{
            "url": interface_url,
            "protocolBinding": super::A2A_PROTOCOL_BINDING_HTTP_JSON,
            "protocolVersion": "1.0",
        }],
        "capabilities": {
            "streaming": false,
            "pushNotifications": false,
            "extendedAgentCard": false,
        },
        "securitySchemes": schemes,
        "securityRequirements": requirements,
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [{
            "id": "default",
            "name": skill_name,
            "description": description,
            "tags": ["everruns", "a2a"],
        }],
    })
}

/// A `message:send` request that passed §4.1.
#[derive(Debug, PartialEq)]
struct UserMessage {
    message_id: String,
    context_id: Option<String>,
    text: String,
}

/// Validate a `message:send` body (§4.1). `Err` is the A2A error code and
/// message.
fn parse_message(body: &[u8]) -> Result<UserMessage, (i64, &'static str)> {
    const INVALID: (i64, &str) = (INVALID_PARAMS, "Invalid SendMessageRequest");
    let request: Value = serde_json::from_slice(body).map_err(|_| INVALID)?;
    let message = request
        .get("message")
        .filter(|m| m.is_object())
        .ok_or(INVALID)?;
    let field = |name: &str| message.get(name).and_then(Value::as_str);
    if message.get("taskId").is_some_and(|task| !task.is_null()) {
        return Err((TASK_NOT_FOUND, "Task not found"));
    }
    let message_id = field("messageId")
        .filter(|id| !id.is_empty())
        .ok_or((INVALID_PARAMS, "message.messageId is required"))?;
    if field("role") != Some("ROLE_USER") {
        return Err((INVALID_PARAMS, "message.role must be ROLE_USER"));
    }
    let parts = message
        .get("parts")
        .and_then(Value::as_array)
        .filter(|parts| !parts.is_empty())
        .ok_or((INVALID_PARAMS, "message.parts must not be empty"))?;
    let mut texts = Vec::with_capacity(parts.len());
    for part in parts {
        match part.get("text") {
            Some(Value::String(text)) => texts.push(text.as_str()),
            Some(_) => return Err(INVALID),
            None => {
                return Err((CONTENT_TYPE_NOT_SUPPORTED, "Only text parts are supported"));
            }
        }
    }
    if texts.iter().all(|text| text.trim().is_empty()) {
        return Err((INVALID_PARAMS, "message.parts has no non-blank text"));
    }
    Ok(UserMessage {
        message_id: message_id.to_string(),
        context_id: field("contextId").map(str::to_owned),
        text: texts.join("\n"),
    })
}

async fn send_message(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    req_id: ReqId,
    peer: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let caller = match admit(&state, &channel_id, &headers, peer).await {
        Ok(caller) => caller,
        Err(response) => return response,
    };
    let message = match parse_message(&body) {
        Ok(message) => message,
        Err((code, text)) => return a2a_error(code, text),
    };
    let session = match message.context_id.as_deref() {
        None => None,
        Some(context_id) => match owned_context(&state, &caller, context_id).await {
            Some(session) => Some(session),
            None => return a2a_error(INVALID_PARAMS, "Unknown contextId"),
        },
    };
    if let Some(session_id) = session {
        match stored_reply(&state, session_id, &message.message_id).await {
            Ok(None) => {}
            Ok(Some(Reply::Ready { message_id, text })) => {
                return reply(session_id, &message_id, &text);
            }
            Ok(Some(Reply::Pending | Reply::Failed)) => {
                return a2a_error(INVALID_PARAMS, "No stored reply for this messageId");
            }
            Err(err) => return internal(err),
        }
    }

    let subscription_slot = Arc::new(tokio::sync::Mutex::new(None));
    let hook_slot = subscription_slot.clone();
    let event_delivery = state.event_delivery.clone();
    let result = invoke_channel_a2a_with_hook(
        &state.db,
        state.encryption.as_ref(),
        &state.session_service,
        &state.message_service,
        A2aInvocationRequest {
            legacy_app_id: caller.legacy_app_id,
            channel_id: caller.auth.channel_public_id.to_string(),
            params: serde_json::from_slice(&body).unwrap_or(Value::Null),
            text: message.text,
            message_id: Some(message.message_id.clone()),
            task_id: Uuid::now_v7().to_string(),
            context_id: message.context_id,
            role: Some("ROLE_USER".to_string()),
            continue_session: session,
            caller_tag: Some(caller.user.session_tag()),
        },
        req_id.map(|Extension(id)| id.0),
        move |session_id| async move {
            // Subscribed before dispatch, so the turn cannot settle unseen.
            let subscription = event_delivery
                .subscribe(session_id.uuid())
                .await
                .map_err(crate::domains::common::CommandError::internal)?;
            *hook_slot.lock().await = Some(subscription);
            Ok(())
        },
    )
    .await;
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            tracing::warn!(error = %err, "PACT message:send failed to start a turn");
            return a2a_error(INTERNAL, "The agent could not take the message");
        }
    };
    if let Some(mut subscription) = subscription_slot.lock().await.take() {
        task_view::wait_until_settled(
            &mut subscription,
            result.session_id,
            task_view::BLOCKING_SEND_TIMEOUT,
        )
        .await;
    }
    match stored_reply(&state, result.session_id, &message.message_id).await {
        Ok(Some(Reply::Ready { message_id, text })) => reply(result.session_id, &message_id, &text),
        Ok(_) => a2a_error(INTERNAL, "The agent did not reply"),
        Err(err) => internal(err),
    }
}

/// The session a `contextId` names, when it belongs to this channel and this
/// caller (§4.2). THREAT[TM-A2A-012, TM-A2A-016]: a context of another channel
/// or another user collapses to "unknown".
async fn owned_context(
    state: &ChannelA2aState,
    caller: &Caller,
    context_id: &str,
) -> Option<SessionId> {
    let session_id = context_id.parse::<SessionId>().ok()?;
    let session = state
        .db
        .get_session(caller.auth.org_id, session_id)
        .await
        .ok()??;
    let caller_tag = caller.user.session_tag();
    (super::session_belongs_to_a2a_channel(&session, &caller.auth)
        && session.tags.iter().any(|tag| tag == &caller_tag))
    .then_some(session.id)
}

/// The agent's answer to one user message.
#[derive(Debug, PartialEq)]
enum Reply {
    /// The turn finished with final text. `message_id` is the last output's.
    Ready { message_id: String, text: String },
    /// The turn has not finished.
    Pending,
    /// The turn ended without text: failed, canceled, or tools only.
    Failed,
}

async fn stored_reply(
    state: &ChannelA2aState,
    session_id: SessionId,
    message_id: &str,
) -> anyhow::Result<Option<Reply>> {
    let filter_types = [
        INPUT_MESSAGE,
        OUTPUT_MESSAGE_COMPLETED,
        TURN_COMPLETED,
        TURN_FAILED,
        TURN_CANCELLED,
    ]
    .map(str::to_string);
    let events = state
        .db
        .list_events(
            session_id,
            None,
            None,
            &filter_types,
            &[],
            None,
            Some(REPLY_EVENT_TAIL),
        )
        .await?;
    Ok(reply_to(&events, message_id))
}

/// Pure: the reply to the user message carrying `message_id`, from a
/// session's filtered event tail (ascending). `None` when no stored message
/// carries it.
fn reply_to(events: &[crate::storage::EventRow], message_id: &str) -> Option<Reply> {
    let start = events.iter().rposition(|event| {
        event.event_type == INPUT_MESSAGE
            && serde_json::from_value::<InputMessageData>(event.data.clone())
                .ok()
                .and_then(|data| data.message.metadata)
                .and_then(|metadata| metadata.get(A2A_MESSAGE_ID_METADATA).cloned())
                .is_some_and(|id| id.as_str() == Some(message_id))
    })?;
    let mut outputs: Vec<(String, String)> = Vec::new();
    for event in &events[start + 1..] {
        match event.event_type.as_str() {
            INPUT_MESSAGE => break,
            OUTPUT_MESSAGE_COMPLETED => {
                if let Ok(data) =
                    serde_json::from_value::<OutputMessageCompletedData>(event.data.clone())
                    && let Some(output) = task_view::final_output(&data)
                {
                    outputs.push(output);
                }
            }
            TURN_COMPLETED => {
                let Some((message_id, _)) = outputs.last() else {
                    return Some(Reply::Failed);
                };
                let text = outputs
                    .iter()
                    .map(|(_, text)| text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                return Some(Reply::Ready {
                    message_id: message_id.clone(),
                    text,
                });
            }
            TURN_FAILED | TURN_CANCELLED => return Some(Reply::Failed),
            _ => {}
        }
    }
    Some(Reply::Pending)
}

/// §4.2: the synchronous reply, a Message (never a Task).
fn reply(session_id: SessionId, message_id: &str, text: &str) -> Response {
    a2a_json(
        StatusCode::OK,
        &json!({
            "message": {
                "messageId": message_id,
                "contextId": session_id.to_string(),
                "role": "ROLE_AGENT",
                "parts": [{ "text": text }],
            }
        }),
    )
}

fn internal(err: anyhow::Error) -> Response {
    tracing::error!(error = %err, "PACT request failed");
    a2a_error(INTERNAL, "Internal error")
}

/// §2.2: ordinary turns create no task, so the list is always empty.
async fn list_tasks(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    peer: Peer,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = admit(&state, &channel_id, &headers, peer).await {
        return response;
    }
    let page_size = match query.get("pageSize").map(|raw| raw.parse::<u64>()) {
        None => DEFAULT_PAGE_SIZE,
        Some(Ok(size)) if (1..=MAX_PAGE_SIZE).contains(&size) => size,
        Some(_) => return a2a_error(INVALID_PARAMS, "pageSize must be between 1 and 100"),
    };
    a2a_json(
        StatusCode::OK,
        &json!({ "tasks": [], "nextPageToken": "", "pageSize": page_size, "totalSize": 0 }),
    )
}

async fn get_task(
    State(state): State<ChannelA2aState>,
    Path((channel_id, task)): Path<(String, String)>,
    peer: Peer,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = admit(&state, &channel_id, &headers, peer).await {
        return response;
    }
    a2a_error(TASK_NOT_FOUND, &format!("Task not found: {task}"))
}

/// `POST tasks/{id}:cancel` and `POST tasks/{id}:subscribe`. Any other verb is
/// not a PACT route, so it is a plain `404` before authentication.
async fn task_action(
    State(state): State<ChannelA2aState>,
    Path((channel_id, task)): Path<(String, String)>,
    peer: Peer,
    headers: HeaderMap,
) -> Response {
    let (task_id, cancel) = match task.rsplit_once(':') {
        Some((id, "cancel")) => (id, true),
        Some((id, "subscribe")) => (id, false),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if let Err(response) = admit(&state, &channel_id, &headers, peer).await {
        return response;
    }
    if cancel {
        a2a_error(TASK_NOT_FOUND, &format!("Task not found: {task_id}"))
    } else {
        a2a_error(UNSUPPORTED_OPERATION, "Streaming is not supported")
    }
}

async fn unsupported(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    peer: Peer,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = admit(&state, &channel_id, &headers, peer).await {
        return response;
    }
    a2a_error(UNSUPPORTED_OPERATION, "Unsupported operation")
}

async fn push_unsupported(
    State(state): State<ChannelA2aState>,
    Path(params): Path<HashMap<String, String>>,
    peer: Peer,
    headers: HeaderMap,
) -> Response {
    let channel_id = params.get("channel_id").cloned().unwrap_or_default();
    if let Err(response) = admit(&state, &channel_id, &headers, peer).await {
        return response;
    }
    a2a_error(
        PUSH_NOTIFICATION_NOT_SUPPORTED,
        "Push notifications are not supported",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::EventRow;

    fn parse(body: Value) -> Result<UserMessage, i64> {
        parse_message(body.to_string().as_bytes()).map_err(|(code, _)| code)
    }

    fn message(fields: Value) -> Value {
        let mut message = json!({
            "messageId": "m-1",
            "role": "ROLE_USER",
            "parts": [{ "text": "hi", "mediaType": "text/plain" }],
        });
        for (key, value) in fields.as_object().unwrap() {
            message[key] = value.clone();
        }
        json!({ "message": message })
    }

    #[test]
    fn parses_a_text_message() {
        assert_eq!(
            parse(message(json!({ "contextId": "c-1" }))).unwrap(),
            UserMessage {
                message_id: "m-1".into(),
                context_id: Some("c-1".into()),
                text: "hi".into(),
            }
        );
    }

    #[test]
    fn rejects_what_section_4_1_rules_out() {
        assert_eq!(parse_message(b"{").unwrap_err().0, INVALID_PARAMS);
        assert_eq!(parse(json!({})).unwrap_err(), INVALID_PARAMS);
        assert_eq!(
            parse(message(json!({ "taskId": "task-1" }))).unwrap_err(),
            TASK_NOT_FOUND
        );
        assert_eq!(
            parse(message(json!({ "role": "ROLE_AGENT" }))).unwrap_err(),
            INVALID_PARAMS
        );
        assert_eq!(
            parse(message(json!({ "messageId": "" }))).unwrap_err(),
            INVALID_PARAMS
        );
        assert_eq!(
            parse(message(json!({ "parts": [{ "text": "  " }] }))).unwrap_err(),
            INVALID_PARAMS
        );
        assert_eq!(
            parse(message(json!({ "parts": [{ "raw": "aGVsbG8=" }] }))).unwrap_err(),
            CONTENT_TYPE_NOT_SUPPORTED
        );
        assert_eq!(
            parse(message(
                json!({ "parts": [{ "text": "a" }, { "data": {} }] })
            ))
            .unwrap_err(),
            CONTENT_TYPE_NOT_SUPPORTED
        );
    }

    fn event(event_type: &str, data: Value) -> EventRow {
        EventRow {
            id: everruns_contracts::typed_id::EventId::from_uuid(Uuid::now_v7()),
            session_id: SessionId::from_uuid(Uuid::now_v7()),
            sequence: 0,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data,
            metadata: None,
            tags: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn input(message_id: &str) -> EventRow {
        let mut message = everruns_core::message::RuntimeMessage::user("hi");
        message.metadata = Some(HashMap::from([(
            A2A_MESSAGE_ID_METADATA.to_string(),
            json!(message_id),
        )]));
        event(
            INPUT_MESSAGE,
            serde_json::to_value(InputMessageData::new(message)).unwrap(),
        )
    }

    /// An assistant output; returns the row and its message id.
    fn output(text: &str) -> (EventRow, String) {
        let message = everruns_core::message::RuntimeMessage::assistant(text);
        let id = message.id.to_string();
        let data = OutputMessageCompletedData::new(message);
        (
            event(
                OUTPUT_MESSAGE_COMPLETED,
                serde_json::to_value(data).unwrap(),
            ),
            id,
        )
    }

    #[test]
    fn reply_is_the_turn_after_the_matching_message() {
        let (first_output, first) = output("first answer");
        let (second_output, second) = output("second answer");
        let events = [
            input("m-1"),
            first_output,
            event(TURN_COMPLETED, json!({})),
            input("m-2"),
            second_output,
            event(TURN_COMPLETED, json!({})),
        ];
        assert_eq!(
            reply_to(&events, "m-1"),
            Some(Reply::Ready {
                message_id: first,
                text: "first answer".into(),
            })
        );
        assert_eq!(
            reply_to(&events, "m-2"),
            Some(Reply::Ready {
                message_id: second,
                text: "second answer".into(),
            })
        );
        assert_eq!(reply_to(&events, "m-3"), None);
    }

    #[test]
    fn unfinished_and_failed_turns_have_no_reply() {
        assert_eq!(
            reply_to(&[input("m-1"), output("partial").0], "m-1"),
            Some(Reply::Pending)
        );
        assert_eq!(
            reply_to(&[input("m-1"), event(TURN_FAILED, json!({}))], "m-1"),
            Some(Reply::Failed)
        );
    }

    #[test]
    fn card_declares_http_json_and_the_personal_agent_jwt() {
        let card = pact_card(
            "Shop",
            "Orders",
            "Shop",
            "https://x.example/v1/a2a/ch",
            None,
        );
        assert_eq!(
            card["supportedInterfaces"],
            json!([{
                "url": "https://x.example/v1/a2a/ch",
                "protocolBinding": "HTTP+JSON",
                "protocolVersion": "1.0",
            }])
        );
        for scheme in ["paJwt", "platformJwt"] {
            assert_eq!(
                card["securitySchemes"][scheme]["httpAuthSecurityScheme"]["scheme"],
                "Bearer"
            );
        }
        assert_eq!(card["capabilities"]["streaming"], false);
    }
}
