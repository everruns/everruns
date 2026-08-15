//! Curated, application-facing session history values.

use std::fmt;
use std::str::FromStr;
use std::time::SystemTime;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use everruns_core::Message;
use everruns_host::{
    EventCursor, EventHistory, EventHistoryReadLimit, EventHistoryReadRequest, EventLogError,
    MAX_EVENT_HISTORY_PAGE_SIZE,
};

use crate::{ContentPart, MessageRole, SessionExecution, SessionId};

const MAX_CURSOR_TOKEN_LEN: usize = 4096;
const CURSOR_PREFIX: &str = "eh1.";
const DEFAULT_HISTORY_PAGE_SIZE: usize = 100;

/// Owned builder for one bounded, stable session-history snapshot.
///
/// Obtain it with [`Session::history`](crate::Session::history). The default
/// page size is 100 messages and the maximum is 256. Pages are ordered oldest
/// first by canonical event sequence.
#[derive(Clone)]
pub struct HistoryQuery {
    execution: std::sync::Arc<dyn SessionExecution>,
    session_id: SessionId,
    limit: usize,
    cursor: Option<EventCursor>,
}

impl HistoryQuery {
    pub(crate) fn new(
        execution: std::sync::Arc<dyn SessionExecution>,
        session_id: SessionId,
    ) -> Self {
        Self {
            execution,
            session_id,
            limit: DEFAULT_HISTORY_PAGE_SIZE,
            cursor: None,
        }
    }

    /// Select the maximum number of messages returned by [`page`](Self::page).
    ///
    /// Valid limits are 1 through 256. The typed error includes the allowed
    /// maximum so applications do not need a public backend constant.
    pub fn limit(mut self, limit: usize) -> Result<Self, HistoryError> {
        if limit == 0 || limit > MAX_EVENT_HISTORY_PAGE_SIZE {
            return Err(HistoryError::InvalidLimit {
                requested: limit,
                maximum: MAX_EVENT_HISTORY_PAGE_SIZE,
            });
        }
        self.limit = limit;
        Ok(self)
    }

    /// Continue the stable snapshot represented by `cursor`.
    ///
    /// Session binding and token compatibility are validated immediately.
    /// Snapshot retention is validated by the configured event reader when the
    /// page is read.
    pub fn after(mut self, cursor: HistoryCursor) -> Result<Self, HistoryError> {
        let event_cursor = cursor.decode()?;
        if event_cursor.session_id() != self.session_id {
            return Err(HistoryError::CrossSessionCursor);
        }
        self.cursor = Some(event_cursor);
        Ok(self)
    }

    /// Read one bounded history page.
    ///
    /// The first page fixes an event high-watermark. Continuations cannot see
    /// concurrent appends, so they neither skip nor duplicate messages. Start a
    /// new query to observe events committed after that snapshot.
    pub async fn page(&self) -> Result<HistoryPage, HistoryError> {
        self.execution.ensure_cataloged().await?;
        let agent = self.execution.agent_snapshot();
        let backends = agent
            .shared_backends()
            .await
            .map_err(|error| error.history_error())?;
        let limit = EventHistoryReadLimit::new(self.limit).map_err(map_event_error)?;
        let mut request = EventHistoryReadRequest::new(self.session_id, limit);
        if let Some(cursor) = &self.cursor {
            request = request.with_cursor(cursor.clone());
        }
        let page = EventHistory::new(backends.event_log.clone())
            .read_page(request)
            .await
            .map_err(map_event_error)?;
        Ok(HistoryPage {
            session_id: self.session_id,
            messages: page
                .messages
                .into_iter()
                .map(SessionMessage::from)
                .collect(),
            next_cursor: page.next_cursor.map(HistoryCursor::encode).transpose()?,
        })
    }

    /// Lazily walk the snapshot one bounded page at a time.
    pub fn pages(self) -> HistoryPages {
        HistoryPages {
            query: Some(self),
            finished: false,
        }
    }
}

impl fmt::Debug for HistoryQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryQuery")
            .field("session_id", &self.session_id)
            .field("limit", &self.limit)
            .field("has_cursor", &self.cursor.is_some())
            .finish()
    }
}

/// Lazy, fused reader for all pages in one stable history snapshot.
///
/// This convenience remains bounded in memory: each call reads at most the
/// query's selected page size and does not retain prior pages.
#[derive(Debug)]
pub struct HistoryPages {
    query: Option<HistoryQuery>,
    finished: bool,
}

impl HistoryPages {
    /// Read the next bounded page, or `None` after the snapshot is exhausted.
    ///
    /// End behavior is fused. After returning `None`, later calls return `None`
    /// without consulting persistence again. An empty first page is returned
    /// once as `Some(page)`, followed by `None`.
    pub async fn next_page(&mut self) -> Result<Option<HistoryPage>, HistoryError> {
        if self.finished {
            return Ok(None);
        }
        let query = self
            .query
            .take()
            .expect("unfinished page reader has a query");
        let page = match query.page().await {
            Ok(page) => page,
            Err(error) => {
                // Preserve the query so a transient backend failure never
                // turns a later retry into a panic.
                self.query = Some(query);
                return Err(error);
            }
        };
        match page.next_cursor.clone() {
            Some(cursor) => self.query = Some(query.after(cursor)?),
            None => self.finished = true,
        }
        Ok(Some(page))
    }
}

/// One bounded page of a session conversation.
///
/// Messages are ordered oldest-first by canonical event sequence. A page never
/// implies that the complete history was loaded: continue with
/// [`next_cursor`](Self::next_cursor) when it is present.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HistoryPage {
    /// The session this page belongs to.
    pub session_id: SessionId,
    /// Messages in canonical event-sequence order, oldest first.
    pub messages: Vec<SessionMessage>,
    /// Opaque continuation cursor, or `None` when the snapshot is exhausted.
    pub next_cursor: Option<HistoryCursor>,
}

impl HistoryPage {
    /// Number of messages in this page.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether this page contains no messages.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Iterate over messages in canonical event-sequence order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &SessionMessage> {
        self.messages.iter()
    }
}

/// One application-facing message in a [`HistoryPage`].
///
/// This projection carries transcript fields while keeping stored session
/// records, provider controls, internal metadata, and host backend types out of
/// the Framework API.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SessionMessage {
    /// Opaque message identifier.
    pub id: String,
    /// Conversational role of the message.
    pub role: MessageRole,
    /// Structured text, image, tool-call, and tool-result content.
    pub content: Vec<ContentPart>,
    /// Timestamp recorded by the canonical event.
    ///
    /// History ordering is based on event sequence, never this timestamp.
    pub created_at: SystemTime,
}

impl SessionMessage {
    /// Concatenate this message's text content parts in content order.
    ///
    /// Images, tool calls, and tool results remain available through
    /// [`content`](Self::content) and do not contribute text here.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(ContentPart::as_text)
            .collect()
    }
}

impl From<Message> for SessionMessage {
    fn from(message: Message) -> Self {
        Self {
            id: message.id.to_string(),
            role: message.role,
            content: message.content,
            created_at: message.created_at.into(),
        }
    }
}

/// Opaque continuation token for a stable session-history snapshot.
///
/// Cursors are bound to the session and query that created them. They are safe
/// to move between tasks or persist as strings with [`Display`](fmt::Display),
/// then restore with [`FromStr`]. The token contents and encoding are not a
/// public contract.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct HistoryCursor(String);

impl HistoryCursor {
    pub(crate) fn from_token(token: String) -> Result<Self, HistoryCursorParseError> {
        validate_cursor_token(&token)?;
        Ok(Self(token))
    }

    fn encode(cursor: EventCursor) -> Result<Self, HistoryError> {
        let encoded = serde_json::to_vec(&cursor).map_err(|_| HistoryError::IncompatibleCursor)?;
        Self::from_token(format!(
            "{CURSOR_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(encoded)
        ))
        .map_err(|_| HistoryError::IncompatibleCursor)
    }

    fn decode(&self) -> Result<EventCursor, HistoryError> {
        let encoded = self
            .0
            .strip_prefix(CURSOR_PREFIX)
            .ok_or(HistoryError::IncompatibleCursor)?;
        if encoded.is_empty() {
            return Err(HistoryError::InvalidCursor);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| HistoryError::InvalidCursor)?;
        serde_json::from_slice(&bytes).map_err(|_| HistoryError::InvalidCursor)
    }
}

impl fmt::Debug for HistoryCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryCursor")
            .field("token", &"<redacted>")
            .field("len", &self.0.len())
            .finish()
    }
}

impl fmt::Display for HistoryCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for HistoryCursor {
    type Err = HistoryCursorParseError;

    fn from_str(token: &str) -> Result<Self, Self::Err> {
        Self::from_token(token.to_owned())
    }
}

fn validate_cursor_token(token: &str) -> Result<(), HistoryCursorParseError> {
    // THREAT[TM-DOS-035]: reject oversized or delimiter-bearing tokens before
    // base64/JSON decoding or any event-reader work.
    if token.is_empty()
        || token.len() > MAX_CURSOR_TOKEN_LEN
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
    {
        return Err(HistoryCursorParseError(()));
    }
    Ok(())
}

/// A string could not be parsed as a [`HistoryCursor`].
///
/// The error intentionally omits the rejected token so logs and debug output do
/// not disclose opaque cursor contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryCursorParseError(());

impl fmt::Display for HistoryCursorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid history cursor")
    }
}

impl std::error::Error for HistoryCursorParseError {}

/// Why a bounded session-history request could not be completed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HistoryError {
    /// The requested page size was zero or exceeded the documented maximum.
    InvalidLimit {
        /// Rejected page size.
        requested: usize,
        /// Largest page size accepted by this Framework version.
        maximum: usize,
    },
    /// The cursor token was malformed.
    InvalidCursor,
    /// The cursor belongs to another session.
    CrossSessionCursor,
    /// The backend no longer retains the cursor snapshot.
    ExpiredCursor,
    /// The cursor was created by an incompatible query or Framework version.
    IncompatibleCursor,
    /// The bounded projection safety limit was reached before the page could
    /// determine its next message boundary.
    HistoryTooLarge,
    /// No materialized session exists for the requested id.
    SessionNotFound {
        /// Missing Framework session identity.
        session_id: SessionId,
    },
    /// The configured history backend could not be reached or opened.
    Unavailable,
    /// The canonical event history is internally inconsistent or malformed.
    Corrupt,
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit { requested, maximum } => {
                write!(
                    f,
                    "history page limit must be between 1 and {maximum}; got {requested}"
                )
            }
            Self::InvalidCursor => f.write_str("invalid history cursor"),
            Self::CrossSessionCursor => f.write_str("history cursor belongs to another session"),
            Self::ExpiredCursor => f.write_str("history cursor snapshot has expired"),
            Self::IncompatibleCursor => {
                f.write_str("history cursor is incompatible with this query")
            }
            Self::HistoryTooLarge => {
                f.write_str("session history exceeds the bounded replay limit")
            }
            Self::SessionNotFound { session_id } => {
                write!(f, "session {session_id} was not found")
            }
            Self::Unavailable => f.write_str("session history is unavailable"),
            Self::Corrupt => f.write_str("session history is corrupt"),
        }
    }
}

impl std::error::Error for HistoryError {}

/// Why an existing Framework session could not be resumed.
///
/// This error keeps event-log and host backend details private while preserving
/// the application decisions that callers can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResumeError {
    /// No materialized session exists for the requested id.
    SessionNotFound {
        /// Missing Framework session identity.
        session_id: SessionId,
    },
    /// The configured session backend could not be reached or opened.
    Unavailable,
    /// The canonical event history is internally inconsistent or malformed.
    Corrupt,
    /// The provider recorded for this session was not registered on the Agent.
    WorkspaceProviderUnavailable {
        /// Stable open provider id needed to reopen the head.
        provider_id: String,
    },
    /// The recorded provider could not reopen the exact workspace head.
    WorkspaceUnavailable,
    /// The provider returned a different workspace/head than was recorded.
    WorkspaceMismatch,
    /// The persisted opaque workspace binding could not be decoded.
    WorkspaceBindingCorrupt,
}

impl fmt::Display for ResumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionNotFound { session_id } => {
                write!(f, "session {session_id} was not found")
            }
            Self::Unavailable => f.write_str("session history is unavailable"),
            Self::Corrupt => f.write_str("session history is corrupt"),
            Self::WorkspaceProviderUnavailable { provider_id } => {
                write!(f, "workspace provider {provider_id} is unavailable")
            }
            Self::WorkspaceUnavailable => f.write_str("recorded workspace head is unavailable"),
            Self::WorkspaceMismatch => f.write_str("workspace provider reopened a different head"),
            Self::WorkspaceBindingCorrupt => f.write_str("persisted workspace binding is corrupt"),
        }
    }
}

impl std::error::Error for ResumeError {}

fn map_event_error(error: EventLogError) -> HistoryError {
    match error {
        EventLogError::CrossSessionCursor { .. } => HistoryError::CrossSessionCursor,
        EventLogError::IncompatibleCursor { .. } => HistoryError::IncompatibleCursor,
        EventLogError::ExpiredCursor { .. } => HistoryError::ExpiredCursor,
        EventLogError::Corruption { .. } => HistoryError::Corrupt,
        EventLogError::InvalidRead { .. } => HistoryError::HistoryTooLarge,
        EventLogError::Backend { .. } | EventLogError::InvalidAppend { .. } => {
            HistoryError::Unavailable
        }
        _ => HistoryError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use everruns_host::EventLogError;

    use super::{HistoryCursor, HistoryError, ResumeError, map_event_error};

    #[test]
    fn cursor_round_trips_as_an_opaque_string() {
        let cursor = HistoryCursor::from_str("eh1.c2Vzc2lvbg.c25hcHNob3Q").unwrap();

        assert_eq!(cursor.to_string(), "eh1.c2Vzc2lvbg.c25hcHNob3Q");
        assert_eq!(
            HistoryCursor::from_str(&cursor.to_string()).unwrap(),
            cursor
        );
        assert!(!format!("{cursor:?}").contains("c2Vzc2lvbg"));
    }

    #[test]
    fn cursor_rejects_empty_whitespace_control_and_oversized_tokens() {
        assert!(HistoryCursor::from_str("").is_err());
        assert!(HistoryCursor::from_str("token with spaces").is_err());
        assert!(HistoryCursor::from_str("token\n").is_err());
        assert!(HistoryCursor::from_str("token/with/path-delimiters").is_err());
        assert!(HistoryCursor::from_str(&"x".repeat(4097)).is_err());
    }

    #[test]
    fn invalid_limit_reports_the_allowed_maximum() {
        let error = HistoryError::InvalidLimit {
            requested: 1001,
            maximum: 1000,
        };

        assert_eq!(
            error.to_string(),
            "history page limit must be between 1 and 1000; got 1001"
        );
    }

    #[test]
    fn resume_not_found_keeps_the_framework_session_id() {
        let session_id = crate::SessionId::new();
        let error = ResumeError::SessionNotFound { session_id };

        assert_eq!(
            error.to_string(),
            format!("session {session_id} was not found")
        );
    }

    #[test]
    fn host_cursor_and_replay_failures_keep_distinct_framework_errors() {
        assert_eq!(
            map_event_error(EventLogError::ExpiredCursor {
                detail: "expired".into(),
            }),
            HistoryError::ExpiredCursor
        );
        assert_eq!(
            map_event_error(EventLogError::IncompatibleCursor {
                detail: "incompatible".into(),
            }),
            HistoryError::IncompatibleCursor
        );
        assert_eq!(
            map_event_error(EventLogError::InvalidRead {
                detail: "bounded replay".into(),
            }),
            HistoryError::HistoryTooLarge
        );
    }
}
