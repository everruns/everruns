//! A v2 session as the control plane's session API sees it (step 11.6):
//! its reads and commands on its data plane.
//!
//! Decision: the control plane keeps serving the session API features a
//! v1 session's tables answer (the Trace view, question answers, ...)
//! for a v2 session too, with the same code, reading the session's events
//! from its cell and sending the outcome back as a session command. One
//! implementation per feature keeps validation, attribution and policy in
//! one place for both planes.

use everruns_contracts::dataplane::SessionEventRow;
use everruns_contracts::session_commands::{self, SessionCommand};
use everruns_contracts::typed_id::SessionId;
use everruns_core::{INPUT_MESSAGE, OUTPUT_MESSAGE_COMPLETED};
use serde_json::Value;

use super::{ControlPlane, DataPlane, for_org, org_public};
use crate::storage::StorageBackend;

/// Events one `log` page holds (the cell's bound).
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

    /// Bounded source excerpts, projected on the cell before transport.
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

    /// A session command on the session (`tool-results`, ...): the cell's
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
    /// (the cell checks its side: what it declares, it serves).
    #[test]
    fn every_session_route_declares_its_plane() {
        use everruns_contracts::session_commands::SESSION_ROUTES;
        use utoipa::OpenApi as _;

        let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
        let mut undeclared = Vec::new();
        for path in spec["paths"].as_object().unwrap().keys() {
            let Some(rest) = path.strip_prefix("/v1/sessions/{session_id}/") else {
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
