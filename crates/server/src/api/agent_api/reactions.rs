// Reactions on the Agent Execution API: a caller reacts to messages in its
// own sessions and reads the reactions there, the agent's included.
//
// Same rules as `/v1` (domains::messages::reactions): the caller is a channel
// consumer, an end user's reaction runs under that end user, and an agent key
// acting as itself reacts as the application.

use axum::{
    Extension, Json,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use everruns_core::{ReactionActor, ReactionActorKind};
use serde::Deserialize;

use super::{Authorized, authorize_by_id, callers_session, internal_error};
use crate::api::channel_api::ChannelApiState;
use crate::api::common::ErrorResponse;
use crate::domains::agent_channels::api_sessions::CallerIdentity;
use crate::domains::messages::reactions::{
    MessageReactions, ReactionChange, Reactor, list_reactions, set_reaction,
};

/// Display name of an agent key acting as itself.
const APPLICATION_ACTOR_NAME: &str = "Application";

/// The caller as a reactor: its actor key and the authority a reaction
/// message it causes runs under.
async fn reactor(state: &ChannelApiState, auth: &Authorized) -> Reactor {
    let source = if auth.channel.channel_type
        == crate::domains::agent_channels::record::ChannelType::WorkChat
    {
        "work_chat"
    } else {
        "api"
    };
    match auth.caller.identity {
        CallerIdentity::Key { key_id } => Reactor {
            actor: ReactionActor {
                kind: ReactionActorKind::Consumer,
                id: format!("agent_key:{key_id}"),
                name: Some(APPLICATION_ACTOR_NAME.to_string()),
            },
            user_id: None,
            principal_id: None,
            source,
        },
        CallerIdentity::EndUser { user, .. } => {
            let name = state
                .db
                .get_virtual_user(auth.context.org_id, user.virtual_user_id)
                .await
                .ok()
                .flatten()
                .map(|row| row.name);
            Reactor {
                actor: ReactionActor {
                    kind: ReactionActorKind::Consumer,
                    id: user.principal_id.to_string(),
                    name,
                },
                user_id: None,
                principal_id: Some(user.principal_id),
                source,
            }
        }
    }
}

/// Path of one reaction: channel, session, message, emoji.
type ReactionPath = (String, String, String, String);

async fn change(
    state: ChannelApiState,
    (channel_id, session_id, message_id, emoji): ReactionPath,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    add: bool,
) -> Response {
    let auth = match authorize_by_id(&state, &channel_id, &headers, connect_info).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let session = match callers_session(&state, &auth, &session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let reactor = reactor(&state, &auth).await;
    match set_reaction(
        &state.message_service,
        &session,
        &message_id,
        &emoji,
        reactor,
        add,
    )
    .await
    {
        Ok(change) => Json(change).into_response(),
        Err(err) => {
            let (status, body): (StatusCode, Json<ErrorResponse>) = err.into();
            (status, body).into_response()
        }
    }
}

#[utoipa::path(
    description = "Put an emoji reaction on a message of one of the caller's sessions. Adding one already there changes nothing. A reaction on the agent's reply reaches the agent as a short note.",
    put,
    path = "/v1/channels/{channel_id}/sessions/{session_id}/messages/{message_id}/reactions/{emoji}",
    params(
        ("channel_id" = String, Path, description = "api channel ID"),
        ("session_id" = String, Path, description = "Session ID"),
        ("message_id" = String, Path, description = "Message ID"),
        ("emoji" = String, Path, description = "One emoji (URL-encoded), or a custom emoji as :name:")
    ),
    responses(
        (status = 200, description = "The reaction is on the message", body = ReactionChange),
        (status = 400, description = "Not one emoji", body = ErrorResponse),
        (status = 401, description = "Missing or invalid credentials", body = ErrorResponse),
        (status = 404, description = "Channel, session or message not found", body = ErrorResponse)
    ),
    tag = "agent-execution"
)]
pub async fn agent_api_add_reaction(
    State(state): State<ChannelApiState>,
    Path(path): Path<ReactionPath>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    change(state, path, headers, connect_info, true).await
}

#[utoipa::path(
    description = "Take the caller's emoji reaction off a message. Removing one that is not there changes nothing.",
    delete,
    path = "/v1/channels/{channel_id}/sessions/{session_id}/messages/{message_id}/reactions/{emoji}",
    params(
        ("channel_id" = String, Path, description = "api channel ID"),
        ("session_id" = String, Path, description = "Session ID"),
        ("message_id" = String, Path, description = "Message ID"),
        ("emoji" = String, Path, description = "One emoji (URL-encoded), or a custom emoji as :name:")
    ),
    responses(
        (status = 200, description = "The reaction is off the message", body = ReactionChange),
        (status = 400, description = "Not one emoji", body = ErrorResponse),
        (status = 401, description = "Missing or invalid credentials", body = ErrorResponse),
        (status = 404, description = "Channel, session or message not found", body = ErrorResponse)
    ),
    tag = "agent-execution"
)]
pub async fn agent_api_remove_reaction(
    State(state): State<ChannelApiState>,
    Path(path): Path<ReactionPath>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    change(state, path, headers, connect_info, false).await
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListReactionsQuery {
    /// Only this message's reactions.
    #[serde(default)]
    message_id: Option<String>,
}

#[utoipa::path(
    description = "Reactions on the messages of one of the caller's sessions, the agent's included. `me` marks the caller's own.",
    get,
    path = "/v1/channels/{channel_id}/sessions/{session_id}/reactions",
    params(
        ("channel_id" = String, Path, description = "api channel ID"),
        ("session_id" = String, Path, description = "Session ID"),
        ListReactionsQuery
    ),
    responses(
        (status = 200, description = "Reactions by message", body = Vec<MessageReactions>),
        (status = 401, description = "Missing or invalid credentials", body = ErrorResponse),
        (status = 404, description = "Channel or session not found", body = ErrorResponse)
    ),
    tag = "agent-execution"
)]
pub async fn agent_api_list_reactions(
    State(state): State<ChannelApiState>,
    Path((channel_id, session_id)): Path<(String, String)>,
    Query(query): Query<ListReactionsQuery>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    let auth = match authorize_by_id(&state, &channel_id, &headers, connect_info).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let session = match callers_session(&state, &auth, &session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let me = reactor(&state, &auth).await.actor;
    match list_reactions(
        &state.db,
        &session,
        query.message_id.as_deref(),
        Some((me.kind, me.id.as_str())),
    )
    .await
    {
        Ok(messages) => Json(messages).into_response(),
        Err(err) => internal_error(err),
    }
}
