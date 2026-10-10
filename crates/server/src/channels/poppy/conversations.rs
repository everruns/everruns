// Poppy conversations (spec 7): the personal agent talks to the channel's
// agent over a small REST API. One conversation is one Everruns session.
//
// Design Decisions:
// - A conversation belongs to the (`client_id`, User ID) that started it
//   (`poppy_conversations`); any of that pair's Sessions may continue it.
//   Anything else, including another channel's conversation id, is the same
//   `404 conversation_not_found` (spec 7.13). THREAT[TM-POPPY-004].
// - Message ids are unique per User, not per conversation (spec 7.3). The
//   claim on (`client_id`, User ID, id) is taken before the message is
//   delivered, with a hash of what was sent: a retry with the same content
//   gets the original answer and starts no second turn, the same id with
//   other content is `409 message_id_conflict`.
// - What the model reads is the text, then `data` as JSON, then the User's
//   context (spec 7.4) as it stands after this message. A message with only
//   `context` updates the stored context and starts no turn.
// - Handoff to a person (spec 7.9): no person can join yet, which the spec
//   allows. The channel tells the agent in a hidden note, and the agent says
//   so in its own words; the conversation stays with the agent.
// - Direct Conversations (spec 7.10) are not offered yet: a start with
//   `parent_conversation_id` is `400`.
// - Events are read from the session's own log (`events.rs`); a read with
//   `wait` long-polls the session's live event stream. It returns once the
//   company side has something to say (a message, the turn ending, closing),
//   not on the echo of the personal agent's own message, so `wait` on a
//   `POST` brings back the reply (spec 7.3).

use std::time::Duration;

use axum::{
    body::Bytes,
    extract::{OriginalUri, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use everruns_contracts::typed_id::SessionId;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::events::{self, Cursor};
use super::tokens::{Authenticated, authenticate};
use super::{Peer, PoppyState, error, internal, json_response};
use crate::domains::common::CommandError;
use crate::domains::messages::CreateMessageContext;
use crate::domains::messages::types::{CreateMessageRequest, InputMessage, MessageRole};
use crate::domains::sessions::record::SessionSource;
use crate::domains::sessions::types::CreateSessionRequest;
use crate::execution_metadata;
use crate::storage::poppy::{PoppyConversationRow, PoppyOwner};

/// Longest `wait` honoured, in seconds (spec 7.5 lets the Company cap it).
const MAX_WAIT_SECS: u64 = 30;
/// Stored events read per page.
const PAGE: i32 = 200;
/// Longest message `text` accepted, in characters.
const MAX_TEXT_CHARS: usize = 32_000;
/// Largest `data` or `context`, serialised.
const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_ID_LEN: usize = 256;

/// The hidden note a handoff request sends the agent.
const HANDOFF_NOTE: &str = "The personal agent asked for a person at the company to take over \
this conversation. No one is available to join it right now. Tell them so briefly, and keep \
helping with what you can.";

#[derive(Debug, Deserialize)]
pub(super) struct ReadQuery {
    cursor: Option<String>,
    wait: Option<u64>,
}

/// One message as the personal agent sent it (spec 7.4).
#[derive(Debug, Clone)]
struct IncomingMessage {
    id: String,
    sender: &'static str,
    text: Option<String>,
    data: Option<Value>,
    context: Option<Map<String, Value>>,
    /// The message as sent, for the retry hash.
    raw: Value,
}

/// Pure: a valid conversation, message or event id (spec 7.2).
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Pure: parse and check the body's `message`.
fn parse_message(body: &Value) -> Result<IncomingMessage, &'static str> {
    let raw = body.get("message").ok_or("message is required")?;
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or("message.id must be 1 to 256 URL-safe base64 characters")?;
    let sender = match raw.get("sender").and_then(Value::as_str) {
        Some("agent") => "agent",
        Some("human") => "human",
        _ => return Err("message.sender must be agent or human"),
    };
    let text = match raw.get("text") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.chars().count() <= MAX_TEXT_CHARS => {
            Some(text.clone()).filter(|text| !text.trim().is_empty())
        }
        Some(_) => return Err("message.text must be a string of at most 32,000 characters"),
    };
    let object = |name: &str| -> Result<Option<Map<String, Value>>, &'static str> {
        match raw.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Object(object))
                if Value::Object(object.clone()).to_string().len() <= MAX_JSON_BYTES =>
            {
                Ok(Some(object.clone()))
            }
            Some(_) => {
                Err("message.data and message.context must be JSON objects of at most 64 KiB")
            }
        }
    };
    let data = object("data")?.map(Value::Object);
    let context = object("context")?;
    if text.is_none() && data.is_none() && context.is_none() {
        return Err("A message needs text, data or context");
    }
    Ok(IncomingMessage {
        id: id.to_string(),
        sender,
        text,
        data,
        context,
        raw: raw.clone(),
    })
}

/// Pure: a hash of what was sent where, sorted so key order never matters.
fn content_hash(conversation: Option<Uuid>, message: &Value) -> Vec<u8> {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for key in keys {
                    out.insert(key.clone(), sorted(&map[key]));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    let target = conversation.map(|id| id.to_string()).unwrap_or_default();
    Sha256::new()
        .chain_update(target.as_bytes())
        .chain_update([0u8])
        .chain_update(sorted(message).to_string().as_bytes())
        .finalize()
        .to_vec()
}

/// Pure: the context after `update` (fields it leaves out keep their values).
fn merged_context(current: &Value, update: &Map<String, Value>) -> Value {
    let mut merged = current.as_object().cloned().unwrap_or_default();
    for (key, value) in update {
        merged.insert(key.clone(), value.clone());
    }
    Value::Object(merged)
}

/// Pure: what the model reads for one message.
fn render(message: &IncomingMessage, context: &Value) -> String {
    let mut parts = Vec::new();
    if message.sender == "human" {
        parts.push("The user wrote this message themselves.".to_string());
    }
    if let Some(text) = &message.text {
        parts.push(text.clone());
    }
    if let Some(data) = &message.data {
        parts.push(format!("Structured details (JSON): {data}"));
    }
    if message.context.is_some() {
        parts.push(format!("The user's situation (JSON): {context}"));
    }
    parts.join("\n\n")
}

pub(super) fn conversation_id(session_id: Uuid) -> String {
    format!("cnv_{}", session_id.simple())
}

fn parse_conversation_id(id: &str) -> Option<Uuid> {
    Uuid::try_parse(id.strip_prefix("cnv_")?).ok()
}

fn owner(auth: &Authenticated) -> PoppyOwner<'_> {
    PoppyOwner {
        channel_id: auth.channel.channel.internal_id,
        client_id: &auth.session.client_id,
        user_id: &auth.session.user_id,
    }
}

fn not_found() -> Response {
    error(
        StatusCode::NOT_FOUND,
        "conversation_not_found",
        "Unknown conversation",
    )
}

fn bad_request(description: &str) -> Response {
    error(StatusCode::BAD_REQUEST, "invalid_request", description)
}

fn parse_body(body: &[u8]) -> Result<Value, Response> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| bad_request("The body must be a JSON object"))
}

fn command_error(err: CommandError) -> Response {
    internal(anyhow::anyhow!("{err}"))
}

/// `POST {endpoint}`: start a conversation with its first message.
pub(super) async fn start(
    State(state): State<PoppyState>,
    Path(channel_id): Path<String>,
    Query(query): Query<ReadQuery>,
    OriginalUri(uri): OriginalUri,
    peer: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let auth = match authenticate(&state, &channel_id, "POST", uri.path(), &headers, peer).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let body = match parse_body(&body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    if body.get("parent_conversation_id").is_some() {
        return bad_request("Direct Conversations are not offered by this Company yet");
    }
    let message = match parse_message(&body) {
        Ok(message) => message,
        Err(description) => return bad_request(description),
    };
    let hash = content_hash(None, &message.raw);
    match retried(&state, &auth, &message, &hash).await {
        Ok(Some(session_id)) => {
            return answer(&state, &auth, session_id, StatusCode::CREATED, &query).await;
        }
        Ok(None) => {}
        Err(response) => return response,
    }
    let session_id = match create_session(&state, &auth).await {
        Ok(session_id) => session_id,
        Err(response) => return response,
    };
    let context = message
        .context
        .as_ref()
        .map(|update| merged_context(&json!({}), update))
        .unwrap_or_else(|| json!({}));
    if let Err(err) = state
        .db
        .create_poppy_conversation(session_id, owner(&auth), &context)
        .await
    {
        return internal(err);
    }
    let conversation = PoppyConversationRow {
        session_id,
        channel_id: auth.channel.channel.internal_id,
        client_id: auth.session.client_id.clone(),
        user_id: auth.session.user_id.clone(),
        account: None,
        context: context.clone(),
        closed_at: None,
        created_at: chrono::Utc::now(),
    };
    accept(
        &state,
        &auth,
        &conversation,
        &message,
        &hash,
        &context,
        StatusCode::CREATED,
        &query,
    )
    .await
}

/// `POST {endpoint}/{id}/messages`.
pub(super) async fn send(
    State(state): State<PoppyState>,
    Path((channel_id, conversation_id)): Path<(String, String)>,
    Query(query): Query<ReadQuery>,
    OriginalUri(uri): OriginalUri,
    peer: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (auth, conversation) = match open(
        &state,
        &channel_id,
        &conversation_id,
        "POST",
        uri.path(),
        &headers,
        peer,
    )
    .await
    {
        Ok(found) => found,
        Err(response) => return response,
    };
    let body = match parse_body(&body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let message = match parse_message(&body) {
        Ok(message) => message,
        Err(description) => return bad_request(description),
    };
    let hash = content_hash(Some(conversation.session_id), &message.raw);
    match retried(&state, &auth, &message, &hash).await {
        Ok(Some(session_id)) => {
            return answer(&state, &auth, session_id, StatusCode::ACCEPTED, &query).await;
        }
        Ok(None) => {}
        Err(response) => return response,
    }
    if conversation.closed_at.is_some() {
        return closed();
    }
    let context = match &message.context {
        Some(update) => {
            let context = merged_context(&conversation.context, update);
            if let Err(err) = state
                .db
                .set_poppy_conversation_context(conversation.session_id, &context)
                .await
            {
                return internal(err);
            }
            context
        }
        None => conversation.context.clone(),
    };
    accept(
        &state,
        &auth,
        &conversation,
        &message,
        &hash,
        &context,
        StatusCode::ACCEPTED,
        &query,
    )
    .await
}

fn closed() -> Response {
    error(
        StatusCode::CONFLICT,
        "conversation_closed",
        "The conversation is closed",
    )
}

/// The conversation a retry of `message` already went to, when this exact
/// message was accepted before. `409` when its id was used for something else.
async fn retried(
    state: &PoppyState,
    auth: &Authenticated,
    message: &IncomingMessage,
    hash: &[u8],
) -> Result<Option<Uuid>, Response> {
    match state.db.poppy_message(owner(auth), &message.id).await {
        Ok(Some(row)) if row.content_hash == hash => Ok(Some(row.session_id)),
        Ok(Some(_)) => Err(error(
            StatusCode::CONFLICT,
            "message_id_conflict",
            "A message with this id was already accepted with different content",
        )),
        Ok(None) => Ok(None),
        Err(err) => Err(internal(err)),
    }
}

/// Claim the message id, then deliver the message to the agent.
#[allow(clippy::too_many_arguments)]
async fn accept(
    state: &PoppyState,
    auth: &Authenticated,
    conversation: &PoppyConversationRow,
    message: &IncomingMessage,
    hash: &[u8],
    context: &Value,
    status: StatusCode,
    query: &ReadQuery,
) -> Response {
    let session_id = conversation.session_id;
    match state
        .db
        .insert_poppy_message(owner(auth), &message.id, session_id, hash)
        .await
    {
        Ok(true) => {}
        // A concurrent request with the same id won the claim.
        Ok(false) => {
            return match retried(state, auth, message, hash).await {
                Ok(Some(session_id)) => answer(state, auth, session_id, status, query).await,
                Ok(None) => internal(anyhow::anyhow!("Poppy message claim vanished")),
                Err(response) => response,
            };
        }
        Err(err) => return internal(err),
    }
    // A message with only context says nothing to answer.
    if message.text.is_some() || message.data.is_some() {
        let text = render(message, context);
        let mut metadata = vec![
            (events::MESSAGE_ID, json!(message.id)),
            (events::SENDER, json!(message.sender)),
        ];
        if let Some(text) = &message.text {
            metadata.push((events::TEXT, json!(text)));
        }
        if let Some(data) = &message.data {
            metadata.push((events::DATA, data.clone()));
        }
        if let Err(err) = deliver(state, auth, session_id, text, metadata).await {
            if let Err(release) = state
                .db
                .delete_poppy_message(owner(auth), &message.id)
                .await
            {
                tracing::warn!(error = %release, "failed to release a Poppy message id");
            }
            return command_error(err);
        }
        return respond(state, auth, session_id, status, "working", query).await;
    }
    answer(state, auth, session_id, status, query).await
}

/// Start an Everruns session for a new conversation. It runs as the channel,
/// like every other channel session.
async fn create_session(state: &PoppyState, auth: &Authenticated) -> Result<Uuid, Response> {
    let context = &auth.channel.context;
    let channel = &auth.channel.channel;
    let client_name = super::client::load(state, &auth.session.client_id)
        .await
        .ok()
        .and_then(|client| client.name);
    let title = Some(format!(
        "{} conversation",
        client_name.as_deref().unwrap_or("Personal agent")
    ));
    let session = state
        .session_service
        .create_from_app(
            &everruns_core::Caller::internal(context.org_id),
            context.harness_id.uuid(),
            Some(context.agent_internal_id),
            context.agent_id,
            context.historical_app_id,
            Some(channel.internal_id),
            None,
            context.owner_principal_id,
            context.resolved_owner_user_id,
            SessionSource::A2a,
            CreateSessionRequest {
                playground_user_id: None,
                source: None,
                workspace_id: None,
                harness_id: Some(context.harness_id),
                harness_name: None,
                agent_id: context.agent_id,
                agent_name: None,
                virtual_user_id: context.virtual_user_id,
                title,
                goal: None,
                locale: None,
                tags: vec![
                    format!("poppy_channel:{}", channel.public_id),
                    caller_tag(&auth.session.client_id, &auth.session.user_id),
                ],
                model_id: None,
                capabilities: vec![],
                sandbox: None,
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
        .await
        .map_err(internal)?;
    Ok(session.id.uuid())
}

/// A tag naming the (personal agent, User) pair, hashed: the User ID is the
/// agent's opaque per-company data and never needs to appear in a tag.
fn caller_tag(client_id: &str, user_id: &str) -> String {
    let digest = Sha256::new()
        .chain_update(client_id.as_bytes())
        .chain_update([0u8])
        .chain_update(user_id.as_bytes())
        .finalize();
    format!("poppy_caller:{}", &hex::encode(digest)[..32])
}

/// Send one user message to the session: starts a turn, or steers the
/// running one.
async fn deliver(
    state: &PoppyState,
    auth: &Authenticated,
    session_id: Uuid,
    text: String,
    metadata: Vec<(&str, Value)>,
) -> Result<(), CommandError> {
    let context = &auth.channel.context;
    let mut metadata: std::collections::HashMap<String, Value> = metadata
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    metadata.insert("source".into(), json!("poppy_channel"));
    metadata.insert(
        "poppy_channel_id".into(),
        json!(auth.channel.channel.public_id.to_string()),
    );
    state
        .message_service
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: context.org_id,
                user_id: None,
                harness_id: context.harness_id.uuid(),
                agent_id: Some(context.agent_internal_id),
                session_id,
                event_metadata: Some(execution_metadata::channel_message_metadata(
                    context.public_id,
                    context.owner_principal_id,
                    context.virtual_user_id,
                )),
                request_id: None,
            },
            CreateMessageRequest {
                message: InputMessage {
                    role: MessageRole::User,
                    content: vec![everruns_core::InputContentPart::text(text)],
                },
                addressed_participant_id: None,
                controls: None,
                metadata: Some(metadata),
                tags: None,
                external_actor: None,
            },
        )
        .await?;
    Ok(())
}

/// Authenticate and find an owned conversation.
async fn open(
    state: &PoppyState,
    channel_id: &str,
    conversation_id: &str,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    peer: Peer,
) -> Result<(Authenticated, PoppyConversationRow), Response> {
    let auth = authenticate(state, channel_id, method, path, headers, peer).await?;
    let Some(session_id) = parse_conversation_id(conversation_id) else {
        return Err(not_found());
    };
    match state.db.poppy_conversation(session_id, owner(&auth)).await {
        Ok(Some(conversation)) => Ok((auth, conversation)),
        Ok(None) => Err(not_found()),
        Err(err) => Err(internal(err)),
    }
}

/// The answer to an accepted message: the conversation's current state, and
/// its events when the request asked to `wait` (spec 7.3).
async fn answer(
    state: &PoppyState,
    auth: &Authenticated,
    session_id: Uuid,
    status: StatusCode,
    query: &ReadQuery,
) -> Response {
    let current = match current_status(state, auth, session_id).await {
        Ok(current) => current,
        Err(response) => return response,
    };
    respond(state, auth, session_id, status, current, query).await
}

async fn respond(
    state: &PoppyState,
    auth: &Authenticated,
    session_id: Uuid,
    status: StatusCode,
    current: &'static str,
    query: &ReadQuery,
) -> Response {
    let mut body = json!({
        "conversation_id": conversation_id(session_id),
        "status": current,
        "responder": "agent",
    });
    if query.wait.is_some() || query.cursor.is_some() {
        match read(state, auth, session_id, query).await {
            Ok(read) => {
                for (key, value) in read.as_object().into_iter().flatten() {
                    body[key] = value.clone();
                }
            }
            Err(response) => return response,
        }
    }
    json_response(status, &body)
}

/// `working` while a turn runs, `closed` once closed, `idle` otherwise.
async fn current_status(
    state: &PoppyState,
    auth: &Authenticated,
    session_id: Uuid,
) -> Result<&'static str, Response> {
    let conversation = state
        .db
        .poppy_conversation(session_id, owner(auth))
        .await
        .map_err(internal)?
        .ok_or_else(not_found)?;
    if conversation.closed_at.is_some() {
        return Ok("closed");
    }
    let session = state
        .db
        .get_session(
            auth.channel.context.org_id,
            SessionId::from_uuid(session_id),
        )
        .await
        .map_err(internal)?
        .ok_or_else(not_found)?;
    Ok(match session.status.as_str() {
        "active" | "waitingfortoolresults" => "working",
        _ => "idle",
    })
}

/// `GET {endpoint}/{id}/events`.
pub(super) async fn events(
    State(state): State<PoppyState>,
    Path((channel_id, conversation_id)): Path<(String, String)>,
    Query(query): Query<ReadQuery>,
    OriginalUri(uri): OriginalUri,
    peer: Peer,
    headers: HeaderMap,
) -> Response {
    let (auth, conversation) = match open(
        &state,
        &channel_id,
        &conversation_id,
        "GET",
        uri.path(),
        &headers,
        peer,
    )
    .await
    {
        Ok(found) => found,
        Err(response) => return response,
    };
    match read(&state, &auth, conversation.session_id, &query).await {
        Ok(body) => json_response(StatusCode::OK, &body),
        Err(response) => response,
    }
}

/// Read events after the query's cursor, holding up to `wait` seconds for
/// the first one (spec 7.5).
async fn read(
    state: &PoppyState,
    auth: &Authenticated,
    session_id: Uuid,
    query: &ReadQuery,
) -> Result<Value, Response> {
    let cursor = events::parse_cursor(query.cursor.as_deref()).map_err(|()| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_cursor",
            "The cursor is unknown or not from this conversation",
        )
    })?;
    let wait = Duration::from_secs(query.wait.unwrap_or(0).min(MAX_WAIT_SECS));
    let deadline = tokio::time::Instant::now() + wait;
    // Subscribed before the first read, so an event cannot slip between the
    // read and the wait.
    let mut subscription = match wait.is_zero() {
        true => None,
        false => Some(
            state
                .event_delivery
                .subscribe(session_id)
                .await
                .map_err(internal)?,
        ),
    };
    loop {
        let page = read_page(state, auth, session_id, cursor).await?;
        let done = page.has_more || page.events.iter().any(events::answers);
        if done || tokio::time::Instant::now() >= deadline {
            let status = current_status(state, auth, session_id).await?;
            return Ok(json!({
                "conversation_id": conversation_id(session_id),
                "events": page.events,
                "cursor": page.cursor.or_else(|| query.cursor.clone()),
                "has_more": page.has_more,
                "status": status,
                "responder": "agent",
            }));
        }
        // Any new event in the session is worth another read. Without a live
        // stream, read once more at the deadline.
        match subscription.as_mut() {
            Some(live) => {
                if let Ok(None) = tokio::time::timeout_at(deadline, live.recv()).await {
                    subscription = None;
                }
            }
            None => tokio::time::sleep_until(deadline).await,
        }
    }
}

struct Page {
    events: Vec<Value>,
    cursor: Option<String>,
    has_more: bool,
}

async fn read_page(
    state: &PoppyState,
    auth: &Authenticated,
    session_id: Uuid,
    cursor: Cursor,
) -> Result<Page, Response> {
    let Cursor::After(after) = cursor else {
        return Ok(Page {
            events: vec![],
            cursor: None,
            has_more: false,
        });
    };
    let params = crate::storage::ListEventsParams {
        session_id: SessionId::from_uuid(session_id),
        after_sequence: Some(after),
        filter_types: events::EVENT_TYPES.map(str::to_string).to_vec(),
        limit: Some(PAGE + 1),
        ..Default::default()
    };
    let mut rows = state
        .db
        .list_events_advanced(&params)
        .await
        .map_err(internal)?;
    let has_more = rows.len() > PAGE as usize;
    rows.truncate(PAGE as usize);
    let mut events: Vec<Value> = rows.iter().filter_map(events::project).collect();
    // A full page of hidden events still moves the cursor on.
    let mut cursor = events
        .last()
        .and_then(|event| event["id"].as_str().map(str::to_string))
        .or_else(|| {
            has_more
                .then(|| rows.last().map(|row| events::event_id(row.sequence)))
                .flatten()
        });
    if !has_more {
        let conversation = state
            .db
            .poppy_conversation(session_id, owner(auth))
            .await
            .map_err(internal)?
            .ok_or_else(not_found)?;
        if let Some(closed_at) = conversation.closed_at {
            let created_at = closed_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            events.push(json!({
                "id": events::CLOSED_EVENT_ID,
                "type": "state",
                "created_at": created_at,
                "status": "closed",
                "responder": "agent",
            }));
            cursor = Some(events::CLOSED_EVENT_ID.to_string());
        }
    }
    Ok(Page {
        events,
        cursor,
        has_more,
    })
}

/// `POST {endpoint}/{id}/handoff`.
pub(super) async fn handoff(
    State(state): State<PoppyState>,
    Path((channel_id, conversation_id)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    peer: Peer,
    headers: HeaderMap,
) -> Response {
    let (auth, conversation) = match open(
        &state,
        &channel_id,
        &conversation_id,
        "POST",
        uri.path(),
        &headers,
        peer,
    )
    .await
    {
        Ok(found) => found,
        Err(response) => return response,
    };
    if conversation.closed_at.is_some() {
        return closed();
    }
    let note = vec![(events::NOTE, json!("handoff"))];
    if let Err(err) = deliver(
        &state,
        &auth,
        conversation.session_id,
        HANDOFF_NOTE.to_string(),
        note,
    )
    .await
    {
        return command_error(err);
    }
    json_response(
        StatusCode::ACCEPTED,
        &json!({ "status": "working", "responder": "agent" }),
    )
}

/// `POST {endpoint}/{id}/close`. Closing twice is not an error.
pub(super) async fn close(
    State(state): State<PoppyState>,
    Path((channel_id, conversation_id)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    peer: Peer,
    headers: HeaderMap,
) -> Response {
    let (_auth, conversation) = match open(
        &state,
        &channel_id,
        &conversation_id,
        "POST",
        uri.path(),
        &headers,
        peer,
    )
    .await
    {
        Ok(found) => found,
        Err(response) => return response,
    };
    if conversation.closed_at.is_none() {
        if let Err(err) = state
            .db
            .close_poppy_conversation(conversation.session_id)
            .await
        {
            return internal(err);
        }
        // Spec 7.12: closing takes no more messages; a running turn would only
        // answer into a closed conversation.
        if let Err(err) = crate::api::channel_api::cancel_session_turn_for(
            &state.db,
            &state.message_service,
            SessionId::from_uuid(conversation.session_id),
            "poppy conversation closed",
        )
        .await
        {
            tracing::warn!(error = %err, "failed to cancel a closed Poppy conversation's turn");
        }
    }
    json_response(
        StatusCode::OK,
        &json!({ "status": "closed", "responder": "agent" }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(value: Value) -> Result<IncomingMessage, &'static str> {
        parse_message(&json!({ "message": value }))
    }

    #[test]
    fn parses_messages_per_spec() {
        let ok = message(json!({
            "id": "msg_Yq3v8LrT0aWc5NkE", "sender": "agent", "text": "Hi",
            "context": { "locale": "en-US" },
        }))
        .unwrap();
        assert_eq!(ok.sender, "agent");
        assert!(message(json!({ "id": "msg_1", "sender": "agent", "context": {} })).is_ok());
        for bad in [
            json!({ "sender": "agent", "text": "x" }),
            json!({ "id": "msg 1", "sender": "agent", "text": "x" }),
            json!({ "id": "msg_1", "sender": "bot", "text": "x" }),
            json!({ "id": "msg_1", "sender": "agent" }),
            json!({ "id": "msg_1", "sender": "agent", "data": [1] }),
            json!({ "id": "msg_1", "sender": "agent", "text": 5 }),
        ] {
            assert!(message(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn hash_ignores_key_order_but_not_target() {
        let a = json!({ "id": "m", "sender": "agent", "text": "x" });
        let b: Value = serde_json::from_str(r#"{"text":"x","sender":"agent","id":"m"}"#).unwrap();
        assert_eq!(content_hash(None, &a), content_hash(None, &b));
        assert_ne!(content_hash(None, &a), content_hash(Some(Uuid::nil()), &a));
    }

    #[test]
    fn context_merges_and_renders_for_the_model() {
        let merged = merged_context(
            &json!({ "locale": "en-US", "user_available": true }),
            json!({ "user_available": false }).as_object().unwrap(),
        );
        assert_eq!(
            merged,
            json!({ "locale": "en-US", "user_available": false })
        );
        let message = message(json!({
            "id": "m", "sender": "human", "text": "Medium, please.",
            "data": { "size": "M" }, "context": { "user_available": false },
        }))
        .unwrap();
        let text = render(&message, &merged);
        assert!(text.starts_with("The user wrote this message themselves."));
        assert!(text.contains("Medium, please."));
        assert!(text.contains(r#"{"size":"M"}"#));
        assert!(text.contains("\"locale\":\"en-US\""));
    }

    #[test]
    fn conversation_ids_round_trip() {
        let id = Uuid::now_v7();
        assert_eq!(parse_conversation_id(&conversation_id(id)), Some(id));
        assert_eq!(parse_conversation_id("cnv_nope"), None);
        assert_eq!(parse_conversation_id(&id.to_string()), None);
    }
}
