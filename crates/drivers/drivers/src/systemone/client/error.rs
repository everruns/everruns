//! Errors returned by the TypeSafe client.

/// Everything that can go wrong asking TypeSafe for a judgment.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No API key was configured.
    #[error("TypeSafe API key is missing; set {0} or pass one to the client builder")]
    MissingApiKey(&'static str),

    /// The request never reached a verdict: DNS, TLS, connection, or timeout.
    #[error("TypeSafe request failed: {0}")]
    Transport(String),

    /// TypeSafe answered with a non-success status.
    ///
    /// The message is taken from the response body's `message`/`error`/`detail`
    /// field when present and is length-capped; raw bodies are never surfaced,
    /// because upstream errors can echo request headers.
    #[error("TypeSafe API error (HTTP {status}){}", .message.as_deref().map(|m| format!(": {m}")).unwrap_or_default())]
    Api {
        /// HTTP status code.
        status: u16,
        /// Sanitized upstream message, when the body carried one.
        message: Option<String>,
    },

    /// The response was not the documented shape.
    #[error("invalid response from TypeSafe: {0}")]
    Decode(String),

    /// A request was rejected before it was sent.
    #[error("invalid TypeSafe request: {0}")]
    InvalidRequest(String),

    /// An answer was read under an id that the response does not carry.
    #[error("no answer for question '{0}'")]
    UnknownAnswer(String),

    /// An answer was read as the wrong primitive.
    #[error("question '{id}' answered as {actual}, read as {expected}")]
    AnswerType {
        /// The question id that was read.
        id: String,
        /// The primitive the caller asked for.
        expected: &'static str,
        /// The primitive the answer actually carries.
        actual: &'static str,
    },
}

impl Error {
    /// Whether retrying the identical request could succeed.
    ///
    /// True for transport failures, rate limits (429), overload (529), and 5xx.
    /// Never true for 401 or 422: those need a different key or a different
    /// request.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) => true,
            Self::Api { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }

    /// The HTTP status, when the failure came back from the API.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }
}

/// Result alias for client calls.
pub type Result<T> = std::result::Result<T, Error>;
