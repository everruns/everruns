// The server assigns a durable starter identity, including requests from older
// browser bundles. Browser-local checks cannot arbitrate between clients.

use super::types::CreateSessionRequest;
use crate::domains::common::{CommandError, classify_anyhow};

pub(crate) const PLATFORM_CHAT_STARTER_TAG: &str = "platform-chat-starter";
/// Unique index that settles concurrent starter creation
/// (migrations/144_platform_chat_starter_unique.sql).
pub(crate) const PLATFORM_CHAT_STARTER_UNIQUE_INDEX: &str =
    "idx_sessions_platform_chat_starter_owner";

/// Map a session-create failure. Losing the starter race (another tab, a
/// stale cache, a retried request whose response was lost) is the expected
/// outcome the unique index exists for, and the UI adopts the existing thread
/// on 409. Answer exactly as the generic uniqueness path does, but log at
/// debug: an unknown 23505 still reaches `classify_anyhow`'s warn (EVERRUNS-24).
pub(crate) fn classify_create_session_error(error: anyhow::Error) -> CommandError {
    if crate::errors::violated_unique_constraint(&error).as_deref()
        == Some(PLATFORM_CHAT_STARTER_UNIQUE_INDEX)
    {
        // THREAT[TM-API-005]: Keep storage diagnostics in server logs only.
        tracing::debug!(error = ?error, "platform chat starter already exists");
        return CommandError::conflict(crate::errors::ALREADY_EXISTS_DETAIL)
            .with_code(crate::errors::ALREADY_EXISTS_CODE);
    }
    classify_anyhow(error)
}

pub(crate) fn mark_platform_chat_starter(
    req: &mut CreateSessionRequest,
    harness_name: &str,
    is_platform_chat_agent: bool,
) -> Result<(), CommandError> {
    let is_starter = is_platform_chat_agent
        && harness_name == "generic"
        && matches!(req.title.as_deref(), Some("Platform Chat" | "Chat"))
        && req.tags.iter().any(|tag| tag == "chat")
        && req.parent_session_id.is_none()
        && req.forked_from_session_id.is_none();
    let has_marker = req.tags.iter().any(|tag| tag == PLATFORM_CHAT_STARTER_TAG);
    if has_marker && !is_starter {
        return Err(CommandError::bad_request(
            "platform-chat-starter is reserved for the Platform Chat starter",
        ));
    }
    if is_starter && !has_marker {
        req.tags.push(PLATFORM_CHAT_STARTER_TAG.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_platform_chat_create_gets_starter_identity() {
        let mut req: CreateSessionRequest = serde_json::from_value(json!({
            "source": "chat", "title": "Platform Chat", "tags": ["chat"]
        }))
        .unwrap();
        mark_platform_chat_starter(&mut req, "generic", true).unwrap();
        assert_eq!(req.tags, ["chat", PLATFORM_CHAT_STARTER_TAG]);

        let mut older_req: CreateSessionRequest = serde_json::from_value(json!({
            "title": "Platform Chat", "tags": ["chat"]
        }))
        .unwrap();
        mark_platform_chat_starter(&mut older_req, "generic", true).unwrap();
        assert!(
            older_req
                .tags
                .contains(&PLATFORM_CHAT_STARTER_TAG.to_string())
        );

        let mut ordinary: CreateSessionRequest = serde_json::from_value(json!({
            "source": "chat", "tags": ["chat"]
        }))
        .unwrap();
        mark_platform_chat_starter(&mut ordinary, "generic", true).unwrap();
        assert_eq!(ordinary.tags, ["chat"]);

        ordinary.tags.push(PLATFORM_CHAT_STARTER_TAG.to_string());
        assert!(mark_platform_chat_starter(&mut ordinary, "generic", false).is_err());
    }

    /// Runs `f` under a subscriber that records WARN+ output on this thread.
    fn capture_warnings(f: impl FnOnce() -> CommandError) -> (CommandError, String) {
        use std::sync::{Arc, Mutex};
        #[derive(Clone, Default)]
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Sink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let sink = Sink::default();
        let writer = sink.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let result = tracing::subscriber::with_default(subscriber, f);
        let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        (result, logged)
    }

    fn duplicate_key(constraint: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "error returned from database: duplicate key value violates unique constraint \"{constraint}\""
        )
        .context("create session")
    }

    fn assert_already_exists(err: &CommandError) {
        assert!(matches!(
            &err.kind,
            crate::domains::common::CommandErrorKind::Conflict(detail)
                if detail == crate::errors::ALREADY_EXISTS_DETAIL
        ));
        assert_eq!(
            err.code.as_deref(),
            Some(crate::errors::ALREADY_EXISTS_CODE)
        );
    }

    #[test]
    fn starter_conflict_is_409_already_exists_without_warning() {
        let (err, logged) = capture_warnings(|| {
            classify_create_session_error(duplicate_key(PLATFORM_CHAT_STARTER_UNIQUE_INDEX))
        });
        assert_already_exists(&err);
        assert!(logged.is_empty(), "expected no WARN, got: {logged}");
    }

    #[test]
    fn unknown_unique_conflict_still_warns_with_same_response() {
        let (err, logged) =
            capture_warnings(|| classify_create_session_error(duplicate_key("sessions_pkey")));
        assert_already_exists(&err);
        assert!(logged.contains("database uniqueness conflict"), "{logged}");
    }
}
