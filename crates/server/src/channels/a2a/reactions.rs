// Message reactions over the inbound A2A channel.
//
// A2A 1.0 has no reaction primitive. A caller that supports the reactions
// extension (`REACTIONS_EXTENSION_URI`, advertised on the agent card) sends a
// `message/send` naming the task, whose parts carry one `DataPart` under
// `everruns/reaction`: `{messageId, emoji, action}`. `messageId` is the agent's
// reply as the task names it (the artifact id). It is stored as a reaction, the
// same event every other surface writes, and starts no turn of its own: the
// agent hears about it as the usual short note, by its Reactions setting.
//
// The data part is the whole message. A message with text as well stays an
// ordinary message, so a client that does not know the extension cannot have
// its words swallowed by it.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use everruns_core::{ReactionActor, ReactionActorKind};
use serde::Deserialize;
use serde_json::Value;

use super::wire::{self, WireVersion};
use super::{
    AuthorizedA2a, ChannelA2aState, internal_error, rpc_error, rpc_success,
    session_belongs_to_a2a_channel, task_view,
};
use crate::domains::messages::reactions::{Reactor, set_reaction};

/// Extension URI advertised on the agent card.
pub(super) const REACTIONS_EXTENSION_URI: &str = "https://everruns.com/ext/a2a/reactions/v1";

/// Key of the reaction payload inside a `DataPart`.
const REACTION_KEY: &str = "everruns/reaction";

/// Display name of the remote caller in the note the agent reads.
const REMOTE_AGENT_NAME: &str = "Remote agent";

/// One reaction a caller sent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReactionPart {
    pub message_id: String,
    pub emoji: String,
    pub action: ReactionAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ReactionAction {
    Added,
    Removed,
}

/// The reaction a caller put on the message, if any. `Err` when the part is
/// there but malformed.
pub(super) fn parse_reaction_part(
    parts: &[Value],
) -> Result<Option<ReactionPart>, serde_json::Error> {
    for part in parts {
        // The same kind/type/duck-typing tolerance as text and answer parts.
        let kind = part.get("kind").or_else(|| part.get("type"));
        let is_data = match kind {
            Some(kind) => kind.as_str() == Some("data"),
            None => part.get("data").is_some(),
        };
        if !is_data {
            continue;
        }
        if let Some(payload) = part.get("data").and_then(|data| data.get(REACTION_KEY)) {
            return serde_json::from_value(payload.clone()).map(Some);
        }
    }
    Ok(None)
}

/// Store the reaction on the task's session and answer with the task.
pub(super) async fn handle_reaction(
    state: &ChannelA2aState,
    auth: &AuthorizedA2a,
    task_id: Option<&str>,
    reaction: ReactionPart,
    rpc_id: Value,
    version: WireVersion,
) -> Response {
    let invalid = |rpc_id: Value, detail: &str| -> Response {
        (
            StatusCode::OK,
            rpc_error(rpc_id, -32602, format!("Invalid params: {detail}")),
        )
            .into_response()
    };
    let Some(task_id) = task_id else {
        return invalid(
            rpc_id,
            "a reaction must carry message.taskId naming the task the message is in",
        );
    };
    let Ok(session_id) = task_id.parse::<everruns_contracts::typed_id::SessionId>() else {
        return invalid(rpc_id, "message.taskId is not a known task id");
    };
    let not_found = |rpc_id: Value| {
        (StatusCode::OK, rpc_error(rpc_id, -32001, "Task not found")).into_response()
    };
    let session = match state.db.get_session(auth.org_id, session_id).await {
        Ok(Some(session)) => session,
        Ok(None) => return not_found(rpc_id),
        Err(err) => return internal_error(err).into_response(),
    };
    // THREAT[TM-A2A-012]: a key for one channel reacts only inside that
    // channel's own tasks.
    if !session_belongs_to_a2a_channel(&session, auth) {
        return not_found(rpc_id);
    }

    // Attribution is the channel's, never the payload's.
    let reactor = Reactor {
        actor: ReactionActor {
            kind: ReactionActorKind::External,
            id: format!("a2a:{}", auth.channel_public_id),
            name: Some(REMOTE_AGENT_NAME.to_string()),
        },
        user_id: None,
        principal_id: None,
        source: "a2a",
    };
    if let Err(err) = set_reaction(
        &state.message_service,
        &session,
        &reaction.message_id,
        &reaction.emoji,
        reactor,
        reaction.action == ReactionAction::Added,
    )
    .await
    {
        let (status, body): (StatusCode, axum::Json<crate::api::common::ErrorResponse>) =
            err.into();
        if status == StatusCode::NOT_FOUND {
            return invalid(rpc_id, "messageId is not a message of this task");
        }
        if status.is_client_error() {
            let body = body.0;
            return invalid(rpc_id, body.detail.as_deref().unwrap_or(&body.title));
        }
        return internal_error(anyhow::anyhow!("failed to store the reaction")).into_response();
    }

    let task = match task_view::load_task(state, auth, &session).await {
        Ok(task) => task,
        Err(err) => return internal_error(err).into_response(),
    };
    (
        StatusCode::OK,
        rpc_success(rpc_id, wire::send_message_result(version, task)),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data(payload: Value) -> Vec<Value> {
        vec![json!({ "kind": "data", "data": { REACTION_KEY: payload } })]
    }

    #[test]
    fn a_reaction_part_is_read() {
        let parts = data(json!({ "messageId": "message_1", "emoji": "👍", "action": "added" }));
        assert_eq!(
            parse_reaction_part(&parts).unwrap(),
            Some(ReactionPart {
                message_id: "message_1".into(),
                emoji: "👍".into(),
                action: ReactionAction::Added,
            })
        );
    }

    #[test]
    fn other_parts_are_not_reactions() {
        let parts = vec![
            json!({ "kind": "text", "text": "hi" }),
            json!({ "kind": "data", "data": { "everruns/ask_user_answer": {} } }),
        ];
        assert_eq!(parse_reaction_part(&parts).unwrap(), None);
    }

    #[test]
    fn a_malformed_reaction_is_refused() {
        for payload in [
            json!({ "messageId": "message_1", "emoji": "👍" }),
            json!({ "messageId": "message_1", "emoji": "👍", "action": "toggled" }),
            json!({ "messageId": "message_1", "emoji": "👍", "action": "added", "text": "x" }),
        ] {
            assert!(parse_reaction_part(&data(payload)).is_err());
        }
    }
}
