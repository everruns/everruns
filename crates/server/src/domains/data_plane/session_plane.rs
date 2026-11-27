//! A v2 session as the control plane's session API sees it (step 11.6):
//! its reads and commands on its data plane.
//!
//! Decision: the control plane keeps serving the session API features a
//! v1 session's tables answer (the Trace view, question answers, ...)
//! for a v2 session too, with the same code, reading the session's events
//! from its shard and sending the outcome back as a session command. One
//! implementation per feature keeps validation, attribution and policy in
//! one place for both planes.

use everruns_contracts::dataplane::SessionEventRow;
use everruns_contracts::session_commands::{self, SessionCommand};
use everruns_contracts::typed_id::SessionId;
use everruns_core::{INPUT_MESSAGE, OUTPUT_MESSAGE_COMPLETED};
use serde_json::Value;

use super::{ControlPlane, DataPlane, for_org, org_public};
use crate::storage::StorageBackend;

/// Events one `log` page holds (the shard's bound).
const LOG_PAGE: usize = 1_000;
/// Events an export reads at most.
pub const MAX_LOG: usize = 200_000;

/// One of an org's sessions on its data plane.
pub struct SessionPlane {
    pub plane: Box<dyn DataPlane>,
    pub org: String,
    pub session: SessionId,
}

impl SessionPlane {
    /// `session` of `org_id`, when it runs on a data plane.
    pub async fn of(
        db: &std::sync::Arc<StorageBackend>,
        org_id: i64,
        session: SessionId,
    ) -> anyhow::Result<Option<Self>> {
        Self::with(&ControlPlane::from_env(db.clone()), org_id, session).await
    }

    /// [`SessionPlane::of`] through `cp`.
    pub async fn with(
        cp: &ControlPlane,
        org_id: i64,
        session: SessionId,
    ) -> anyhow::Result<Option<Self>> {
        if !super::sessions::runs_on_data_plane(cp, org_id, session).await? {
            return Ok(None);
        }
        Ok(Some(Self {
            plane: for_org(cp, org_id).await?,
            org: org_public(org_id),
            session,
        }))
    }

    /// The session's committed events in `from..=to`, of `types` (all
    /// when empty), at most the `last` of them when given.
    pub async fn events(
        &self,
        from: i32,
        to: i32,
        types: &[&str],
        last: Option<usize>,
    ) -> anyhow::Result<Vec<SessionEventRow>> {
        self.select(from, to, types, &[], last).await
    }

    /// Bounded source excerpts, projected on the shard before transport.
    pub async fn event_previews(
        &self,
        from: i32,
        to: i32,
        chars: usize,
    ) -> anyhow::Result<Vec<everruns_core::compaction_observation::CompactionEvidenceExcerpt>> {
        let query = format!("from={from}&to={to}&preview_chars={chars}");
        let value = self
            .plane
            .session_read(&self.org, self.session, &session_commands::EVENTS, &query)
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    /// The events carrying the user and agent messages with these ids
    /// (`input.message`, `output.message.completed`).
    pub async fn messages(&self, ids: &[String]) -> anyhow::Result<Vec<SessionEventRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let types = [INPUT_MESSAGE, OUTPUT_MESSAGE_COMPLETED];
        self.select(1, i32::MAX, &types, ids, None).await
    }

    async fn select(
        &self,
        from: i32,
        to: i32,
        types: &[&str],
        messages: &[String],
        last: Option<usize>,
    ) -> anyhow::Result<Vec<SessionEventRow>> {
        let query = events_query(from, to, types, messages, last);
        let value = self
            .plane
            .session_read(&self.org, self.session, &session_commands::EVENTS, &query)
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    /// The session's whole committed log (an export), in public form,
    /// without deltas, at most [`MAX_LOG`] events.
    pub async fn log(&self) -> anyhow::Result<Vec<everruns_core::events::Event>> {
        let mut all: Vec<everruns_core::events::Event> = Vec::new();
        loop {
            let after = all.last().and_then(|e| e.sequence).unwrap_or(0);
            let value = self
                .plane
                .session_read(
                    &self.org,
                    self.session,
                    &session_commands::LOG,
                    &format!("after={after}"),
                )
                .await?;
            let page: Vec<everruns_core::events::Event> = serde_json::from_value(value)?;
            let short = page.len() < LOG_PAGE;
            all.extend(page);
            if short || all.len() >= MAX_LOG {
                return Ok(all);
            }
        }
    }

    /// A read of the session other than its events (`trace`).
    pub async fn read(&self, read: &'static SessionCommand) -> anyhow::Result<Value> {
        self.plane
            .session_read(&self.org, self.session, read, "")
            .await
    }

    /// Admit a person's message `text` on the session, once per `key`
    /// (retry-safe: the same key is the same message).
    pub async fn say(&self, key: &str, text: &str) -> anyhow::Result<Value> {
        let id = everruns_contracts::typed_id::MessageId::from_uuid(uuid::Uuid::new_v5(
            &self.session.uuid(),
            format!("everruns.said.{key}").as_bytes(),
        ));
        let body = serde_json::json!({ "id": id, "text": text });
        self.command(&session_commands::MESSAGES, &body).await
    }

    /// Consume the current channel approval against its canonical log, never
    /// the control plane's v1 event tables. The shard compares the read position
    /// and retains the claim before another callback can consume the same ask.
    pub async fn claim_channel_approval(
        &self,
        claim_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<bool> {
        for _ in 0..4 {
            let rows = self.events(1, i32::MAX, &[], Some(128)).await?;
            let Some((approval_sequence, expected_last_sequence)) =
                approval_snapshot(&rows, turn_id)
            else {
                return Ok(false);
            };
            let reply = self
                .command(
                    &session_commands::CLAIM_APPROVAL,
                    &serde_json::json!({
                        "approval_sequence": approval_sequence,
                        "expected_last_sequence": expected_last_sequence,
                        "turn_id": turn_id,
                        "claim_id": claim_id,
                    }),
                )
                .await?;
            match (reply["claimed"].as_bool(), reply["retry"].as_bool()) {
                (Some(true), Some(false)) => return Ok(true),
                (Some(false), Some(false)) => return Ok(false),
                (Some(false), Some(true)) => continue,
                _ => anyhow::bail!("invalid channel approval claim response"),
            }
        }
        anyhow::bail!("channel approval log changed during claim")
    }

    /// A session command on the session (`tool-results`, ...): the shard's
    /// answer.
    pub async fn command(
        &self,
        command: &'static SessionCommand,
        body: &Value,
    ) -> anyhow::Result<Value> {
        self.plane
            .session_command(&self.org, self.session, command, body)
            .await
    }
}

/// The same freshness window the v1 transaction checks. Missing history is a
/// refusal, not evidence that a card is current. Its position is checked again
/// by the shard; a read followed by an unconditional write would race input.
fn approval_snapshot(rows: &[SessionEventRow], turn_id: &str) -> Option<(i32, i32)> {
    let approval = rows.iter().rposition(|row| {
        row.event_type == "tool.completed"
            && row.data["tool_name"] == "request_approval"
            && row.context["turn_id"].as_str() == Some(turn_id)
    })?;
    if rows[approval + 1..].iter().any(|row| {
        row.event_type == INPUT_MESSAGE
            || row.event_type == "slack.approval.consumed"
            || (row.event_type == "tool.completed" && row.data["tool_name"] == "request_approval")
    }) {
        return None;
    }
    let sequence = rows[approval].sequence;
    let last = rows.last()?.sequence;
    (sequence > 0 && last >= sequence).then_some((sequence, last))
}

/// The message an event carries into a seeded conversation: a person's or
/// the agent's message with its text and media only. Tool calls and their
/// results stay behind: a seeded call without its result would be refused
/// by a model provider. Shared by a fork (`fork_plane.rs`) and a move
/// (`moves.rs`).
pub fn conversation_message(
    event_type: &str,
    data: &Value,
) -> Option<everruns_core::message::RuntimeMessage> {
    use everruns_core::message::ContentPart;
    if event_type != INPUT_MESSAGE && event_type != OUTPUT_MESSAGE_COMPLETED {
        return None;
    }
    let mut message: everruns_core::message::RuntimeMessage =
        serde_json::from_value(data["message"].clone()).ok()?;
    message.content.retain(|part| {
        matches!(
            part,
            ContentPart::Text(_)
                | ContentPart::Image(_)
                | ContentPart::ImageFile(_)
                | ContentPart::File(_)
        )
    });
    (!message.content.is_empty()).then_some(message)
}

/// The `events` read's query (built apart from the read: the serializer is
/// not `Send`).
fn events_query(
    from: i32,
    to: i32,
    types: &[&str],
    messages: &[String],
    last: Option<usize>,
) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("from", &from.to_string());
    query.append_pair("to", &to.to_string());
    if !types.is_empty() {
        query.append_pair("types", &types.join(","));
    }
    if !messages.is_empty() {
        query.append_pair("messages", &messages.join(","));
    }
    if let Some(last) = last {
        query.append_pair("last", &last.to_string());
    }
    query.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval_row(
        sequence: i32,
        kind: &str,
        turn: Option<&str>,
        tool: Option<&str>,
    ) -> SessionEventRow {
        SessionEventRow {
            id: format!("event-{sequence}"),
            sequence,
            event_type: kind.into(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({"turn_id": turn}),
            data: serde_json::json!({"tool_name": tool}),
        }
    }

    #[test]
    fn channel_approval_snapshot_requires_the_unanswered_requesting_turn() {
        let request = approval_row(
            10,
            "tool.completed",
            Some("turn-a"),
            Some("request_approval"),
        );
        let tail = approval_row(12, "turn.completed", Some("turn-a"), None);
        assert_eq!(
            approval_snapshot(&[request.clone(), tail], "turn-a"),
            Some((10, 12))
        );
        assert_eq!(
            approval_snapshot(std::slice::from_ref(&request), "turn-b"),
            None
        );
        assert_eq!(approval_snapshot(&[], "turn-a"), None);
        for later in [
            approval_row(13, INPUT_MESSAGE, Some("turn-b"), None),
            approval_row(
                13,
                "tool.completed",
                Some("turn-b"),
                Some("request_approval"),
            ),
            approval_row(13, "slack.approval.consumed", Some("turn-a"), None),
        ] {
            assert_eq!(approval_snapshot(&[request.clone(), later], "turn-a"), None);
        }
        assert_eq!(
            approval_snapshot(
                &[approval_row(
                    10,
                    "tool.completed",
                    None,
                    Some("request_approval")
                )],
                "turn-a"
            ),
            None
        );
    }

    #[test]
    fn an_events_query_names_what_it_asks() {
        assert_eq!(events_query(1, 9, &[], &[], None), "from=1&to=9");
        assert_eq!(
            events_query(2, 3, &["a.b", "c"], &["message_1".into()], Some(4)),
            "from=2&to=3&types=a.b%2Cc&messages=message_1&last=4"
        );
    }

    /// Every session route the API serves says which plane serves it for a
    /// v2 session, so the edge never sends a new one to the wrong plane
    /// (the shard checks its side: what it declares, it serves).
    #[test]
    fn every_session_route_declares_its_plane() {
        use everruns_contracts::session_commands::SESSION_ROUTES;
        use utoipa::OpenApi as _;

        let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
        let mut undeclared = Vec::new();
        for path in spec["paths"].as_object().unwrap().keys() {
            let Some(rest) = path.strip_prefix("/v1/platform/sessions/{session_id}/") else {
                continue;
            };
            let parts: Vec<&str> = rest.split('/').collect();
            if !SESSION_ROUTES
                .iter()
                .any(|(segment, _)| *segment == parts[0])
            {
                undeclared.push(path.clone());
            }
        }
        assert!(
            undeclared.is_empty(),
            "declare the plane of these routes in crates/contracts/src/session_commands.rs: {undeclared:?}"
        );
    }
}
