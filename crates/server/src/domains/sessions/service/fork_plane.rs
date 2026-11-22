//! Forking a session that runs on a data plane (data plane v2, step 11.6).
//!
//! Decisions:
//! - The parent's history is in its shard, so the fork reads it from there
//!   (the `log` read) and seeds the child, which runs on the same org's
//!   data plane, with the conversation up to the cut: the people's and the
//!   agent's messages with their text and media. Tool calls and their
//!   results stay behind: a seeded call without its result would be
//!   refused by a model provider.
//! - The parent's values and secrets are copied into the child's storage on
//!   its shard; a secret is opened with the org's data key and sealed again
//!   for the child, and the sandbox's state stays with the parent. Its
//!   files are copied as v1 copies a workspace: every file, from one
//!   `snapshot` of the parent's files, or file by file when the workspace
//!   is larger than a snapshot carries. Databases stay, as on v1.
//! - Branching from a message stops where v1 stops: at the end of the turn
//!   the message started, or at the message itself.

use everruns_contracts::session_commands;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use everruns_core::message::RuntimeMessage;

use crate::domains::data_plane::ControlPlane;
use crate::domains::data_plane::session_plane::SessionPlane;
use crate::domains::session_storage::plane::PlaneStorage;

const MESSAGES: &[&str] = &["input.message", "output.message.completed"];
const TURN_ENDS: &[&str] = &["turn.completed", "turn.failed", "turn.cancelled"];

/// `session` of `org_id` on its data plane, if it runs on one.
pub(crate) async fn plane_of(
    db: &std::sync::Arc<crate::storage::StorageBackend>,
    org_id: i64,
    session: SessionId,
) -> anyhow::Result<Option<SessionPlane>> {
    SessionPlane::with(&ControlPlane::from_env(db.clone()), org_id, session).await
}

fn message_id(event: &Event) -> Option<String> {
    let data = serde_json::to_value(&event.data).ok()?;
    data["message"]["id"].as_str().map(str::to_owned)
}

/// Where a fork from `message_id` stops in `events` (sequence order).
pub(crate) fn cut_sequence(events: &[Event], message_id_wanted: &str) -> Option<i32> {
    let at = events.iter().find(|e| {
        MESSAGES.contains(&e.event_type.as_str())
            && message_id(e).as_deref() == Some(message_id_wanted)
    })?;
    let from = at.sequence?;
    Some(
        events
            .iter()
            .filter(|e| TURN_ENDS.contains(&e.event_type.as_str()))
            .filter_map(|e| e.sequence)
            .find(|sequence| *sequence >= from)
            .unwrap_or(from),
    )
}

/// The conversation `events` hold up to `cut`: messages with their text
/// and media only.
pub(crate) fn conversation(events: &[Event], cut: Option<i32>) -> Vec<RuntimeMessage> {
    events
        .iter()
        .filter(|e| cut.is_none_or(|cut| e.sequence.is_some_and(|s| s <= cut)))
        .filter_map(|e| {
            let data = serde_json::to_value(&e.data).ok()?;
            crate::domains::data_plane::session_plane::conversation_message(&e.event_type, &data)
        })
        .collect()
}

/// The sequence a fork of `parent` from `message_id` stops at.
pub(crate) async fn cut_on_plane(
    parent: &SessionPlane,
    message_id: &str,
) -> anyhow::Result<Option<i32>> {
    Ok(cut_sequence(&parent.log().await?, message_id))
}

/// Seed `child` with `parent`'s conversation up to `cut` and copy its
/// values and secrets; the fork's sequence (the last event copied from).
pub(crate) async fn seed_fork(
    db: &std::sync::Arc<crate::storage::StorageBackend>,
    org_id: i64,
    parent: &SessionPlane,
    child: SessionId,
    cut: Option<i32>,
) -> anyhow::Result<Option<i32>> {
    let events = parent.log().await?;
    let fork_sequence = events
        .iter()
        .filter_map(|e| e.sequence)
        .filter(|s| cut.is_none_or(|cut| *s <= cut))
        .max();
    let messages = conversation(&events, cut);
    let Some(child_plane) = plane_of(db, org_id, child).await? else {
        anyhow::bail!("the fork {child} does not run on the data plane");
    };
    if !messages.is_empty() {
        let command = uuid::Uuid::new_v5(&child.uuid(), b"everruns.fork");
        child_plane
            .command(
                &session_commands::SEED,
                &serde_json::json!({ "command": command, "messages": messages }),
            )
            .await?;
    }
    let (Some(from), Some(to)) = (
        PlaneStorage::on(db, org_id, parent.session).await?,
        PlaneStorage::on(db, org_id, child).await?,
    ) else {
        return Ok(fork_sequence);
    };
    let listed = from
        .listed()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    for (key, value) in listed.values {
        to.set_value(&key, &value)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }
    // Secrets are opened here and sealed again for the child: each is bound
    // to its session. The sandbox's state stays: the child gets its own.
    let secrets = from
        .secrets()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    for (name, value) in secrets {
        if name == everruns_contracts::session_sandbox::SESSION_SANDBOX_SECRET_NAME {
            continue;
        }
        to.set_secret(&name, &value)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }
    crate::domains::data_plane::moves::copy_files(parent, &child_plane).await?;
    Ok(fork_sequence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::events::{
        EventContext, InputMessageData, OutputMessageCompletedData, TurnCompletedData,
    };
    use everruns_core::message::ContentPart;

    fn at(mut event: Event, sequence: i32) -> Event {
        event.sequence = Some(sequence);
        event
    }

    #[test]
    fn a_fork_takes_the_conversation_up_to_the_end_of_the_cut_turn() {
        let session = SessionId::new();
        let ask = RuntimeMessage::user("first question");
        let mut answer = RuntimeMessage::assistant("first answer");
        answer.content.push(ContentPart::tool_call(
            "call_1",
            "search",
            serde_json::json!({}),
        ));
        let later = RuntimeMessage::user("second question");
        let events = vec![
            at(
                Event::new(
                    session,
                    EventContext::empty(),
                    InputMessageData::new(ask.clone()),
                ),
                1,
            ),
            at(
                Event::new(
                    session,
                    EventContext::empty(),
                    OutputMessageCompletedData::new(answer),
                ),
                2,
            ),
            at(
                Event::new(session, EventContext::empty(), TurnCompletedData::default()),
                3,
            ),
            at(
                Event::new(session, EventContext::empty(), InputMessageData::new(later)),
                4,
            ),
        ];
        let cut = cut_sequence(&events, &ask.id.to_string());
        assert_eq!(cut, Some(3));
        assert_eq!(cut_sequence(&events, "message_missing"), None);
        let seeded = conversation(&events, cut);
        assert_eq!(seeded.len(), 2);
        assert!(
            seeded[1]
                .content
                .iter()
                .all(|part| matches!(part, ContentPart::Text(_)))
        );
        assert_eq!(conversation(&events, None).len(), 3);
    }
}
