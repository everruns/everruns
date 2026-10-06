// Change intent: why a caller is changing something, and through which door.
//
// Decision: the reason is invocation metadata, not a command param. Every
// surface carries it beside the params (the `/v1/commands` envelope, the
// `Everruns-Change-Reason` header on REST, a global `--reason` on the command
// line, a gRPC field) and lands it here, on `Ctx::change_intent`. The command
// never sees it; `Command::run` records it. That keeps ~170 mutating commands
// free of a field they would all have to declare, document and ignore.
//
// Decision: REST reaches this through a task-local set by an HTTP layer rather
// than through `Ctx` construction. REST handlers build their `Ctx` in dozens of
// places; threading a header through each would be the kind of per-site wiring
// that silently misses one. The layer runs around every request and
// `Command::run` reads the task-local when the `Ctx` carries no intent of its
// own, so an adapter that sets one explicitly (`/v1/commands`, MCP, gRPC)
// always wins.
//
// See knowledge/execution/change-reasons-and-manager-context.md.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domains::common::CommandError;

/// Request header carrying a change reason on REST routes, UTF-8
/// percent-encoded.
pub const CHANGE_REASON_HEADER: &str = "everruns-change-reason";

/// Request header carrying the manager context revision the caller read before
/// a change, on REST routes.
pub const CONTEXT_REVISION_HEADER: &str = "everruns-context-revision";

/// Param names the command shells treat as invocation metadata rather than as
/// command params (`update_agent --id a --reason "..."`). No command may
/// declare a param with one of these names.
pub const RESERVED_PARAMS: &[&str] = &["reason", "context_revision"];

/// Longest accepted reason, in characters.
pub const MAX_REASON_CHARS: usize = 1000;

/// The surface a change arrived through. Set by the adapter, never by the
/// caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSurface {
    /// A REST route (the UI and REST clients).
    Api,
    /// `POST /v1/commands/{name}` (the CLI).
    Commands,
    /// MCP `execute`.
    Mcp,
    /// The Platform capability inside a session (Platform Chat and agents).
    Platform,
    /// The worker's command transport (an agent runtime).
    Worker,
    /// A server-internal caller with no outside surface.
    Internal,
}

impl ChangeSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Commands => "commands",
            Self::Mcp => "mcp",
            Self::Platform => "platform",
            Self::Worker => "worker",
            Self::Internal => "internal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "api" => Self::Api,
            "commands" => Self::Commands,
            "mcp" => Self::Mcp,
            "platform" => Self::Platform,
            "worker" => Self::Worker,
            "internal" => Self::Internal,
            _ => return None,
        })
    }
}

/// Why and how a mutation was invoked. Everything here except `reason` is
/// derived by the server; the reason is caller text and never authoritative.
#[derive(Debug, Clone, Default)]
pub struct ChangeIntent {
    /// The caller's reason, as sent (validated by `Command::run`).
    pub reason: Option<String>,
    pub surface: Option<ChangeSurface>,
    /// Correlation id of the HTTP request, when there was one.
    pub request_id: Option<String>,
    pub idempotency_key: Option<String>,
    /// The session through which an agent made the change.
    pub via_session_id: Option<uuid::Uuid>,
    /// The agent of that session, as its public id.
    pub via_agent_id: Option<String>,
    /// The manager context revision of the changed entity the caller read
    /// before changing it (`--context-revision`).
    pub context_revision: Option<i64>,
    /// Set by `history restore` on the update it runs: the revision it brings
    /// back, so the change records as `restored`.
    pub restoring: Option<i64>,
    /// Where `Command::run` leaves non-fatal notices for the adapter to show.
    pub notices: Notices,
}

/// Non-fatal notices a command run leaves for its adapter, such as "this
/// entity has manager context you did not acknowledge". Shared by every clone
/// of an intent, so the adapter that created it reads what the run wrote.
#[derive(Debug, Clone, Default)]
pub struct Notices(std::sync::Arc<parking_lot::Mutex<Vec<String>>>);

impl Notices {
    pub fn push(&self, notice: impl Into<String>) {
        self.0.lock().push(notice.into());
    }

    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock())
    }
}

impl ChangeIntent {
    pub fn on(surface: ChangeSurface) -> Self {
        Self {
            surface: Some(surface),
            ..Self::default()
        }
    }

    pub fn with_reason(mut self, reason: Option<String>) -> Self {
        if reason.is_some() {
            self.reason = reason;
        }
        self
    }

    /// Fill what this intent leaves unset from `outer`: an adapter that sets
    /// the surface still keeps the request id and header reason the HTTP
    /// layer captured.
    pub fn or(mut self, outer: Option<&ChangeIntent>) -> Self {
        let Some(outer) = outer else {
            return self;
        };
        if self.reason.is_none() {
            self.reason = outer.reason.clone();
        }
        if self.surface.is_none() {
            self.surface = outer.surface;
        }
        if self.request_id.is_none() {
            self.request_id = outer.request_id.clone();
        }
        if self.idempotency_key.is_none() {
            self.idempotency_key = outer.idempotency_key.clone();
        }
        if self.via_session_id.is_none() {
            self.via_session_id = outer.via_session_id;
        }
        if self.via_agent_id.is_none() {
            self.via_agent_id = outer.via_agent_id.clone();
        }
        if self.context_revision.is_none() {
            self.context_revision = outer.context_revision;
        }
        self
    }
}

/// Check a caller's reason: trimmed, 1 to [`MAX_REASON_CHARS`] characters, no
/// control characters except newline and tab, and nothing shaped like a
/// credential. Rejected rather than redacted: a reason is short and the caller
/// can rephrase it, while a redacted one would read as an explanation that was
/// never given.
pub fn validate_reason(raw: &str) -> Result<String, CommandError> {
    let reason = raw.trim();
    if reason.is_empty() {
        return Err(
            CommandError::bad_request("Change reason must not be empty; omit it instead")
                .with_code("invalid_change_reason"),
        );
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        return Err(CommandError::bad_request(format!(
            "Change reason is longer than {MAX_REASON_CHARS} characters"
        ))
        .with_code("invalid_change_reason"));
    }
    if reason
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(
            CommandError::bad_request("Change reason must not contain control characters")
                .with_code("invalid_change_reason"),
        );
    }
    // THREAT[TM-AGENT-016]: history is readable by everyone with entity read
    // access and is replayed into agent context; a credential pasted into a
    // reason would outlive its rotation there.
    if let Some(label) = crate::credential_shape::credential_format(reason) {
        return Err(CommandError::bad_request(format!(
            "Change reason looks like it contains {label}. Reasons are stored in entity history; \
             describe the change without the credential"
        ))
        .with_code("invalid_change_reason"));
    }
    Ok(reason.to_string())
}

tokio::task_local! {
    static HTTP_INTENT: ChangeIntent;
}

/// The intent the HTTP layer captured for the request this task serves.
pub fn current_http_intent() -> Option<ChangeIntent> {
    HTTP_INTENT.try_with(Clone::clone).ok()
}

/// Run `future` with `intent` as the request's captured intent. Exposed for
/// tests; production code reaches it through [`http_change_intent_layer`].
pub async fn scope_http_intent<F: std::future::Future>(
    intent: ChangeIntent,
    future: F,
) -> F::Output {
    HTTP_INTENT.scope(intent, future).await
}

/// HTTP layer that captures the change reason and context revision headers
/// and the request id for every command a request runs.
///
/// An undecodable header is kept as its lossy text so `Command::run` rejects it
/// with the same error every surface gets, rather than this layer inventing a
/// second one.
pub async fn http_change_intent_layer(req: Request, next: Next) -> Response {
    let reason = req
        .headers()
        .get(CHANGE_REASON_HEADER)
        .map(|value| decode_header_reason(value.as_bytes()));
    let context_revision = req
        .headers()
        .get(CONTEXT_REVISION_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok());
    let request_id = req
        .extensions()
        .get::<crate::middleware::RequestId>()
        .map(|id| id.0.clone());
    let intent = ChangeIntent {
        reason,
        surface: Some(ChangeSurface::Api),
        request_id,
        context_revision,
        ..ChangeIntent::default()
    };
    HTTP_INTENT.scope(intent, next.run(req)).await
}

fn decode_header_reason(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    match urlencoding::decode(&text) {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => text.into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_trimmed() {
        assert_eq!(validate_reason("  kid friendly  ").unwrap(), "kid friendly");
    }

    #[test]
    fn empty_long_control_and_credential_reasons_are_rejected() {
        for reason in [
            "   ",
            &"x".repeat(MAX_REASON_CHARS + 1),
            "bell\u{7}",
            "rotate sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345",
        ] {
            let err = validate_reason(reason).expect_err(reason);
            assert_eq!(err.code.as_deref(), Some("invalid_change_reason"));
        }
    }

    #[test]
    fn newlines_and_the_maximum_length_are_accepted() {
        validate_reason("line one\nline two\tindented").unwrap();
        validate_reason(&"é".repeat(MAX_REASON_CHARS)).unwrap();
    }

    #[test]
    fn the_header_is_percent_decoded() {
        assert_eq!(
            decode_header_reason(b"make%20it%20kid%20friendly%20%E2%9C%93"),
            "make it kid friendly ✓"
        );
        assert_eq!(decode_header_reason(b"plain words"), "plain words");
    }

    #[test]
    fn an_explicit_intent_keeps_what_it_set_and_borrows_the_rest() {
        let outer = ChangeIntent {
            reason: Some("from header".into()),
            surface: Some(ChangeSurface::Api),
            request_id: Some("req-1".into()),
            ..ChangeIntent::default()
        };
        let merged = ChangeIntent::on(ChangeSurface::Commands)
            .with_reason(Some("from envelope".into()))
            .or(Some(&outer));
        assert_eq!(merged.reason.as_deref(), Some("from envelope"));
        assert_eq!(merged.surface, Some(ChangeSurface::Commands));
        assert_eq!(merged.request_id.as_deref(), Some("req-1"));
    }

    #[tokio::test]
    async fn the_task_local_is_visible_inside_the_scope_only() {
        assert!(current_http_intent().is_none());
        let seen = scope_http_intent(ChangeIntent::on(ChangeSurface::Api), async {
            current_http_intent().and_then(|intent| intent.surface)
        })
        .await;
        assert_eq!(seen, Some(ChangeSurface::Api));
    }
}
