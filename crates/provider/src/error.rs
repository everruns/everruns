// Error types for the agent loop
//
// StoreResultExt: extension trait to replace repeated .map_err(|e| AgentLoopError::store(...))? patterns
// json_val / from_json: helpers to replace repeated serde_json::to_value/from_value boilerplate

use crate::typed_id::{AgentId, HarnessId, SessionId};
use crate::user_facing_error::{
    UserFacingError, UserFacingErrorContext, classify_runtime_error_message,
    codes as user_facing_error_codes, is_provider_quota_message, is_usage_limit_message,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

/// Result type alias for agent loop operations
pub type Result<T> = std::result::Result<T, AgentLoopError>;

/// Semantic classification of an LLM provider error, assigned by the driver
/// at the provider boundary where the HTTP status and response body are still
/// available. Downstream consumers prefer this over re-parsing error strings;
/// `LlmErrorKind::Other` falls back to string classification
/// (`classify_runtime_error_message`) so untyped errors keep working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmErrorKind {
    /// Invalid or missing credentials, or access denied (401/403, bad API key).
    Authentication,
    /// Provider account is out of credits/quota (billing). Non-transient:
    /// needs operator action, unlike a regular rate limit.
    QuotaExhausted,
    /// Transient rate limit (429).
    RateLimited,
    /// Provider outage or unreachable (5xx, 529, network failure).
    Unavailable,
    /// Provider rejected the request shape (4xx that is not auth/quota/429).
    InvalidRequest,
    /// Unclassified; downstream falls back to string classification.
    Other,
}

impl LlmErrorKind {
    /// Classify a provider's stable machine-readable error code.
    pub fn from_provider_code(code: &str) -> Option<Self> {
        let code = code.trim().to_ascii_lowercase();
        match code.as_str() {
            "insufficient_quota"
            | "billing_hard_limit_reached"
            | "credit_balance_too_low"
            | "credit_balance_exhausted" => Some(Self::QuotaExhausted),
            "authentication_error" | "invalid_api_key" | "permission_denied" => {
                Some(Self::Authentication)
            }
            "rate_limit_exceeded" | "rate_limit_error" | "overloaded_error" => {
                Some(Self::RateLimited)
            }
            "server_error"
            | "internal_error"
            | "processing_error"
            | "service_unavailable"
            | "timeout" => Some(Self::Unavailable),
            "invalid_request_error" | "model_not_found" => Some(Self::InvalidRequest),
            _ => None,
        }
    }

    /// Classify a provider HTTP error from status code + response body.
    ///
    /// Quota/billing patterns are checked before the status code because
    /// providers surface exhausted billing under different statuses
    /// (OpenAI: 429 `insufficient_quota`, Anthropic: 400 "credit balance is
    /// too low").
    pub fn from_provider_status(status: u16, body: &str) -> Self {
        if is_provider_quota_message(body) || is_usage_limit_message(body) {
            return LlmErrorKind::QuotaExhausted;
        }
        match status {
            401 | 403 => LlmErrorKind::Authentication,
            429 => LlmErrorKind::RateLimited,
            408 | 409 => LlmErrorKind::Unavailable,
            501 => LlmErrorKind::Other,
            500..=599 => LlmErrorKind::Unavailable,
            400..=499 => LlmErrorKind::InvalidRequest,
            _ => LlmErrorKind::Other,
        }
    }

    /// Keyword-based classification for drivers without an HTTP status at the
    /// error site (e.g. Bedrock SDK errors).
    pub fn from_error_text(text: &str) -> Self {
        if is_provider_quota_message(text) || is_usage_limit_message(text) {
            return LlmErrorKind::QuotaExhausted;
        }
        let lower = text.to_ascii_lowercase();
        if lower.contains("throttlingexception")
            || lower.contains("toomanyrequestsexception")
            || lower.contains("rate limit")
            || lower.contains("too many requests")
        {
            return LlmErrorKind::RateLimited;
        }
        if lower.contains("accessdeniedexception")
            || lower.contains("unrecognizedclientexception")
            || lower.contains("expiredtokenexception")
            || lower.contains("invalidsignatureexception")
            || lower.contains("unauthorized")
        {
            return LlmErrorKind::Authentication;
        }
        if lower.contains("serviceunavailable")
            || lower.contains("service unavailable")
            || lower.contains("internalserverexception")
            || lower.contains("modelnotreadyexception")
        {
            return LlmErrorKind::Unavailable;
        }
        LlmErrorKind::Other
    }
}

/// LLM provider error with a semantic kind attached by the driver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmError {
    pub kind: LlmErrorKind,
    pub message: String,
    /// Retries already consumed below the turn loop.
    #[serde(default)]
    pub retry_attempts: u32,
    /// Backoff time already consumed below the turn loop.
    #[serde(default)]
    pub retry_wait_ms: u64,
    /// Whether a lower provider layer already made the terminal retry decision.
    #[serde(default)]
    pub retry_handled: bool,
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Errors that can occur during agent loop execution
#[derive(Debug, Error)]
pub enum AgentLoopError {
    /// LLM provider error
    #[error("LLM error: {0}")]
    Llm(LlmError),

    /// Request too large error (context length exceeded, token limits, etc.)
    /// Contains the original error message for logging
    #[error("Request too large: {0}")]
    RequestTooLarge(String),

    /// Model not available (404, model not found, access denied for model)
    /// Contains the model_id string that was requested
    #[error("Model not available: {0}")]
    ModelNotAvailable(String),

    /// No explicit, snapshot, or system-default model could be resolved.
    #[error("Model not configured")]
    ModelNotConfigured,

    /// Tool execution error
    #[error("Tool execution error: {0}")]
    ToolExecution(String),

    /// Message store error
    #[error("Message store error: {0}")]
    MessageStore(String),

    /// Event emission error
    #[error("Event emission error: {0}")]
    EventEmission(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Loop terminated due to max iterations
    #[error("Max iterations ({0}) reached")]
    MaxIterationsReached(usize),

    /// Loop was cancelled
    #[error("Loop cancelled")]
    Cancelled,

    /// No messages to process
    #[error("No messages to process")]
    NoMessages,

    /// Agent not found
    #[error("Agent not found: {0}")]
    AgentNotFound(AgentId),

    /// Harness not found
    #[error("Harness not found: {0}")]
    HarnessNotFound(HarnessId),

    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),

    /// Driver not registered for provider type
    #[error(
        "No driver registered for provider type '{0}'. Make sure the driver is registered at startup."
    )]
    DriverNotRegistered(String),
}

impl AgentLoopError {
    /// Prefix a provider-bound error without discarding its semantic variant.
    pub fn with_provider(mut self, provider: &str) -> Self {
        let prefix = format!("provider '{provider}': ");
        match &mut self {
            AgentLoopError::Llm(error) if !error.message.starts_with(&prefix) => {
                error.message.insert_str(0, &prefix)
            }
            AgentLoopError::RequestTooLarge(message)
            | AgentLoopError::ModelNotAvailable(message)
            | AgentLoopError::Configuration(message)
                if !message.starts_with(&prefix) =>
            {
                message.insert_str(0, &prefix)
            }
            _ => {}
        }
        self
    }

    /// Create an LLM error with no semantic kind (falls back to string
    /// classification downstream).
    pub fn llm(msg: impl Into<String>) -> Self {
        AgentLoopError::Llm(LlmError {
            kind: LlmErrorKind::Other,
            message: msg.into(),
            retry_attempts: 0,
            retry_wait_ms: 0,
            retry_handled: false,
        })
    }

    /// Create an LLM error with a semantic kind assigned at the driver boundary.
    pub fn llm_kind(kind: LlmErrorKind, msg: impl Into<String>) -> Self {
        AgentLoopError::Llm(LlmError {
            kind,
            message: msg.into(),
            retry_attempts: 0,
            retry_wait_ms: 0,
            retry_handled: false,
        })
    }

    /// Attach retries already consumed by a lower provider layer. The reason
    /// loop uses this to avoid multiplying attempt budgets across layers.
    pub fn with_retry_metadata(mut self, metadata: &crate::llm_retry::RetryMetadata) -> Self {
        if let AgentLoopError::Llm(error) = &mut self {
            error.retry_attempts = metadata.attempts;
            error.retry_wait_ms = metadata.total_retry_wait.as_millis() as u64;
            error.retry_handled = true;
        }
        self
    }

    /// Number of lower-layer retries already consumed by this failure.
    pub fn llm_retry_attempts(&self) -> u32 {
        match self {
            AgentLoopError::Llm(error) => error.retry_attempts,
            _ => 0,
        }
    }

    /// Whether a lower provider layer already exhausted or rejected recovery.
    pub fn llm_retry_handled(&self) -> bool {
        matches!(self, AgentLoopError::Llm(error) if error.retry_handled)
    }

    /// Get the semantic LLM error kind, if this is an LLM error.
    pub fn llm_error_kind(&self) -> Option<LlmErrorKind> {
        match self {
            AgentLoopError::Llm(err) => Some(err.kind),
            _ => None,
        }
    }

    /// Create a tool execution error
    pub fn tool(msg: impl Into<String>) -> Self {
        AgentLoopError::ToolExecution(msg.into())
    }

    /// Create a message store error
    pub fn store(msg: impl Into<String>) -> Self {
        AgentLoopError::MessageStore(msg.into())
    }

    /// Create an event emission error
    pub fn event(msg: impl Into<String>) -> Self {
        AgentLoopError::EventEmission(msg.into())
    }

    /// Create a configuration error
    pub fn config(msg: impl Into<String>) -> Self {
        AgentLoopError::Configuration(msg.into())
    }

    /// Create an agent not found error
    pub fn agent_not_found(agent_id: AgentId) -> Self {
        AgentLoopError::AgentNotFound(agent_id)
    }

    /// Create a harness not found error
    pub fn harness_not_found(harness_id: HarnessId) -> Self {
        AgentLoopError::HarnessNotFound(harness_id)
    }

    /// Create a session not found error
    pub fn session_not_found(session_id: SessionId) -> Self {
        AgentLoopError::SessionNotFound(session_id)
    }

    /// Create a driver not registered error
    pub fn driver_not_registered(provider_type: impl Into<String>) -> Self {
        AgentLoopError::DriverNotRegistered(provider_type.into())
    }

    /// Create a request too large error
    pub fn request_too_large(msg: impl Into<String>) -> Self {
        AgentLoopError::RequestTooLarge(msg.into())
    }

    /// Create a model not available error
    pub fn model_not_available(model_id: impl Into<String>) -> Self {
        AgentLoopError::ModelNotAvailable(model_id.into())
    }

    /// Create a missing-model configuration error.
    pub fn model_not_configured() -> Self {
        AgentLoopError::ModelNotConfigured
    }

    /// Check if this is a request-too-large error
    pub fn is_request_too_large(&self) -> bool {
        matches!(self, AgentLoopError::RequestTooLarge(_))
    }

    /// Check if this is a model-not-available error
    pub fn is_model_not_available(&self) -> bool {
        matches!(self, AgentLoopError::ModelNotAvailable(_))
    }

    /// Get the model ID if this is a model-not-available error
    pub fn model_not_available_id(&self) -> Option<&str> {
        match self {
            AgentLoopError::ModelNotAvailable(id) => Some(id),
            _ => None,
        }
    }

    /// Check if this is a rate-limit error (semantic kind, or HTTP 429 /
    /// rate-limit keywords for untyped errors)
    pub fn is_rate_limited(&self) -> bool {
        match self {
            AgentLoopError::Llm(err) => match err.kind {
                LlmErrorKind::RateLimited => true,
                LlmErrorKind::Other => {
                    let msg_lower = err.message.to_ascii_lowercase();
                    msg_lower.contains("(429)")
                        || msg_lower.contains("rate limit")
                        || msg_lower.contains("too many requests")
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Check if this is an authentication/authorization error (HTTP 401/403)
    pub fn is_auth_error(&self) -> bool {
        match self {
            AgentLoopError::Llm(err) => match err.kind {
                LlmErrorKind::Authentication => true,
                LlmErrorKind::Other => {
                    err.message.contains("(401)") || err.message.contains("(403)")
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Check if this is a server error (HTTP 5xx or transient provider issue)
    pub fn is_server_error(&self) -> bool {
        match self {
            AgentLoopError::Llm(err) => match err.kind {
                LlmErrorKind::Unavailable => true,
                LlmErrorKind::Other => {
                    let msg = &err.message;
                    msg.contains("(500)")
                        || msg.contains("(502)")
                        || msg.contains("(503)")
                        || msg.contains("(504)")
                        || msg.contains("(529)")
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Check whether an LLM failure is safe to retry.
    ///
    /// Semantic driver classification is authoritative. Untyped legacy errors
    /// retain the message-based fallback until all drivers preserve structure.
    pub fn is_transient_llm_error(&self) -> bool {
        match self {
            AgentLoopError::Llm(err) => match err.kind {
                LlmErrorKind::RateLimited | LlmErrorKind::Unavailable => true,
                LlmErrorKind::Authentication
                | LlmErrorKind::QuotaExhausted
                | LlmErrorKind::InvalidRequest => false,
                LlmErrorKind::Other => crate::llm_retry::is_transient_error_message(&err.message),
            },
            _ => false,
        }
    }

    /// Check if this error is deterministic and should never be retried.
    ///
    /// Non-retryable errors reference data that is permanently gone (e.g. a
    /// deleted message, a missing agent). Retrying will never succeed and only
    /// burns attempts while keeping the workflow stuck.
    ///
    /// Note: the durable worker currently uses string-matching via
    /// `is_non_retryable_task_error` because task errors arrive as strings.
    /// This method provides the typed equivalent for callers that have access
    /// to a structured `AgentLoopError`.
    pub fn is_non_retryable(&self) -> bool {
        match self {
            // Missing data is permanent — the entity was deleted.
            AgentLoopError::AgentNotFound(_)
            | AgentLoopError::HarnessNotFound(_)
            | AgentLoopError::SessionNotFound(_)
            | AgentLoopError::NoMessages
            | AgentLoopError::ModelNotConfigured => true,

            // Config/driver errors won't self-heal within retries.
            AgentLoopError::Configuration(_) | AgentLoopError::DriverNotRegistered(_) => true,

            // MessageStore "not found" errors (deleted messages).
            AgentLoopError::MessageStore(msg) => msg.to_ascii_lowercase().contains("not found"),

            // Everything else is potentially transient.
            _ => false,
        }
    }

    /// Get user-facing error message based on error classification
    pub fn user_facing_message(&self) -> String {
        self.user_facing_error(UserFacingErrorContext::default())
            .fallback_message()
    }

    /// Get structured user-facing error metadata based on error classification.
    pub fn user_facing_error(&self, context: UserFacingErrorContext) -> UserFacingError {
        match self {
            AgentLoopError::ModelNotConfigured => {
                UserFacingError::new(user_facing_error_codes::MODEL_NOT_CONFIGURED)
            }
            AgentLoopError::ModelNotAvailable(model_id) => {
                UserFacingError::new(user_facing_error_codes::MODEL_UNAVAILABLE)
                    .with_field("model_id", model_id)
                    .with_optional_field("provider", context.provider)
            }
            AgentLoopError::RequestTooLarge(_) => {
                UserFacingError::new(user_facing_error_codes::REQUEST_TOO_LARGE)
                    .with_optional_field("provider", context.provider)
                    .with_optional_field("model_id", context.model_id)
            }
            AgentLoopError::MaxIterationsReached(max_iterations) => {
                UserFacingError::new(user_facing_error_codes::MAX_ITERATIONS)
                    .with_field("max_iterations", max_iterations)
            }
            AgentLoopError::Llm(err) => {
                // Prefer the semantic kind the driver assigned at the provider
                // boundary; fall back to string classification for untyped
                // errors so legacy paths keep working.
                let code = match err.kind {
                    LlmErrorKind::Authentication => {
                        Some(user_facing_error_codes::PROVIDER_MISCONFIGURED)
                    }
                    LlmErrorKind::QuotaExhausted => {
                        Some(user_facing_error_codes::PROVIDER_QUOTA_EXHAUSTED)
                    }
                    LlmErrorKind::RateLimited => {
                        Some(user_facing_error_codes::PROVIDER_RATE_LIMITED)
                    }
                    LlmErrorKind::Unavailable => {
                        Some(user_facing_error_codes::PROVIDER_UNAVAILABLE)
                    }
                    LlmErrorKind::InvalidRequest | LlmErrorKind::Other => None,
                };
                match code {
                    Some(code) => {
                        let error = UserFacingError::new(code)
                            .with_optional_field("provider", context.provider)
                            .with_optional_field("model_id", context.model_id);
                        if code == user_facing_error_codes::PROVIDER_RATE_LIMITED {
                            error.with_optional_field("retry_after", context.retry_after)
                        } else {
                            error
                        }
                    }
                    None => classify_runtime_error_message(&err.message, &context),
                }
            }
            _ => UserFacingError::new(user_facing_error_codes::PROCESSING_ERROR)
                .with_optional_field("provider", context.provider)
                .with_optional_field("model_id", context.model_id),
        }
    }
}

// ============================================================================
// Store Result Extension Trait
// ============================================================================

/// Extension trait that converts any `Result<T, E: Display>` into `Result<T, AgentLoopError>`
/// via `AgentLoopError::store(e.to_string())`.
///
/// Replaces the boilerplate pattern:
/// ```ignore
/// .map_err(|e| AgentLoopError::store(e.to_string()))?
/// ```
/// with:
/// ```ignore
/// .store_err()?
/// ```
pub trait StoreResultExt<T> {
    fn store_err(self) -> Result<T>;
}

impl<T, E: std::fmt::Display> StoreResultExt<T> for std::result::Result<T, E> {
    fn store_err(self) -> Result<T> {
        self.map_err(|e| AgentLoopError::store(e.to_string()))
    }
}

// ============================================================================
// SessionFileSystem error classification (EVE-645)
// ============================================================================

/// Typed classification of a `SessionFileSystem` failure.
///
/// The file-system tools (`integrations/filesystem/src/lib.rs`) decide
/// whether a failure is a *tool error* (surfaced to the agent verbatim — bad
/// input it can correct) or an *internal error* (logged, generic copy). They
/// previously made that call with `msg.contains("readonly")` / `"is a
/// directory"` / `"not found"` style sniffs against the stringified error.
///
/// The `SessionFileSystem` trait returns `anyhow::Result<T>` and has 10+
/// implementors across crates, so widening the trait's error type is out of
/// scope. Instead, [`classify_fs_error`] gives a single typed seam: it
/// downcasts to [`FileSystemError`] when an implementor opts in, and otherwise
/// falls back to the legacy substring heuristics in one place. Implementors can
/// migrate to returning `FileSystemError` (via `anyhow::Error::new`)
/// incrementally without changing behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystemErrorClass {
    /// The target (or a path component) does not exist.
    NotFound,
    /// The target is read-only and cannot be written or deleted.
    ReadOnly,
    /// Expected a file but the path is a directory.
    IsADirectory,
    /// Expected a directory but the path is not one.
    NotADirectory,
    /// A non-recursive delete refused a non-empty directory.
    NotEmpty,
    /// No recognized client-correctable condition; treat as internal.
    Other,
}

/// Typed `SessionFileSystem` error. Implementors may return this (wrapped in
/// `anyhow::Error`) so [`classify_fs_error`] resolves the class without string
/// matching. Each variant carries the human-facing message so the file tools
/// can keep surfacing the same text to the agent.
#[derive(Debug, Error)]
pub enum FileSystemError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    ReadOnly(String),
    #[error("{0}")]
    IsADirectory(String),
    #[error("{0}")]
    NotADirectory(String),
    #[error("{0}")]
    NotEmpty(String),
}

impl FileSystemError {
    fn class(&self) -> FileSystemErrorClass {
        match self {
            FileSystemError::NotFound(_) => FileSystemErrorClass::NotFound,
            FileSystemError::ReadOnly(_) => FileSystemErrorClass::ReadOnly,
            FileSystemError::IsADirectory(_) => FileSystemErrorClass::IsADirectory,
            FileSystemError::NotADirectory(_) => FileSystemErrorClass::NotADirectory,
            FileSystemError::NotEmpty(_) => FileSystemErrorClass::NotEmpty,
        }
    }
}

/// Classify a `SessionFileSystem` failure into a [`FileSystemErrorClass`].
///
/// Prefers a typed [`FileSystemError`] in the error chain; falls back to the
/// legacy substring heuristics (the single remaining place they live) so
/// untyped implementors keep their current routing. Behavior is identical to
/// the previous inline `msg.contains(...)` checks in `file_system.rs`:
/// "readonly" and "is a directory" mark client-correctable write failures,
/// "not found" / "not a directory" mark client-correctable read failures, and
/// "not empty" / "recursive" mark client-correctable delete failures.
pub fn classify_fs_error<E>(err: &E) -> FileSystemErrorClass
where
    E: std::error::Error + 'static,
{
    // Prefer a typed FileSystemError anywhere in the source chain so an
    // implementor that opts in is classified without string matching. Works
    // whether the error is a bare FileSystemError or wrapped (e.g. inside
    // `AgentLoopError::Internal(anyhow!(FileSystemError::..))`).
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(current) = source {
        if let Some(typed) = current.downcast_ref::<FileSystemError>() {
            return typed.class();
        }
        source = current.source();
    }

    let msg = err.to_string();
    // Note: real-disk backends emit "read-only" (hyphenated); the legacy check
    // only matched "readonly", so we preserve that exact behavior rather than
    // silently widening it.
    if msg.contains("readonly") {
        FileSystemErrorClass::ReadOnly
    } else if msg.contains("is a directory") {
        FileSystemErrorClass::IsADirectory
    } else if msg.contains("not a directory") {
        FileSystemErrorClass::NotADirectory
    } else if msg.contains("not empty") || msg.contains("recursive") {
        FileSystemErrorClass::NotEmpty
    } else if msg.contains("not found") {
        FileSystemErrorClass::NotFound
    } else {
        FileSystemErrorClass::Other
    }
}

// ============================================================================
// JSON Helpers
// ============================================================================

/// Convert a serializable value to `serde_json::Value`, falling back to `Value::Null` on error.
///
/// Replaces the boilerplate pattern:
/// ```ignore
/// serde_json::to_value(&x).unwrap_or_default()
/// ```
pub fn json_val<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_default()
}

/// Deserialize a `serde_json::Value` into `T`, falling back to `T::default()` on error.
///
/// Replaces the boilerplate pattern:
/// ```ignore
/// serde_json::from_value(v).unwrap_or_default()
/// ```
pub fn from_json<T: DeserializeOwned + Default>(value: serde_json::Value) -> T {
    serde_json::from_value(value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filesystem_typed_errors_win_over_conflicting_messages_and_wrappers() {
        for (error, expected) in [
            (
                FileSystemError::NotFound("readonly".into()),
                FileSystemErrorClass::NotFound,
            ),
            (
                FileSystemError::ReadOnly("not found".into()),
                FileSystemErrorClass::ReadOnly,
            ),
            (
                FileSystemError::IsADirectory("not empty".into()),
                FileSystemErrorClass::IsADirectory,
            ),
            (
                FileSystemError::NotADirectory("is a directory".into()),
                FileSystemErrorClass::NotADirectory,
            ),
            (
                FileSystemError::NotEmpty("not found".into()),
                FileSystemErrorClass::NotEmpty,
            ),
        ] {
            assert_eq!(classify_fs_error(&error), expected);
            let wrapped = AgentLoopError::Internal(
                anyhow::Error::new(error).context("readonly outer failure"),
            );
            assert_eq!(classify_fs_error(&wrapped), expected);
        }
    }

    #[test]
    fn filesystem_legacy_messages_preserve_routing_and_case_boundaries() {
        for (message, expected) in [
            (
                "Cannot modify readonly file: /a",
                FileSystemErrorClass::ReadOnly,
            ),
            (
                "Cannot delete readonly file: /a",
                FileSystemErrorClass::ReadOnly,
            ),
            (
                "write target is a directory: /a",
                FileSystemErrorClass::IsADirectory,
            ),
            (
                "Path is not a directory: /a",
                FileSystemErrorClass::NotADirectory,
            ),
            (
                "workspace root is not a directory: /a",
                FileSystemErrorClass::NotADirectory,
            ),
            ("Directory not found: /a", FileSystemErrorClass::NotFound),
            (
                "Directory is not empty. Use recursive=true to delete",
                FileSystemErrorClass::NotEmpty,
            ),
            (
                "Cannot delete root directory without recursive flag",
                FileSystemErrorClass::NotEmpty,
            ),
            (
                "recursive delete failed for /a: io",
                FileSystemErrorClass::NotEmpty,
            ),
            ("readonly file not found", FileSystemErrorClass::ReadOnly),
            ("file is read-only: /a", FileSystemErrorClass::Other),
            ("NOT FOUND", FileSystemErrorClass::Other),
            ("disk full", FileSystemErrorClass::Other),
        ] {
            assert_eq!(
                classify_fs_error(&AgentLoopError::store(message)),
                expected,
                "{message}"
            );
        }
    }

    #[test]
    fn typed_request_and_model_errors_preserve_identity_and_safe_user_payload() {
        let context = || {
            UserFacingErrorContext::default()
                .with_provider("provider")
                .with_model_id("context-model")
                .with_retry_after(9)
        };
        let request = AgentLoopError::request_too_large("private payload");
        assert_eq!(request.to_string(), "Request too large: private payload");
        assert!(request.is_request_too_large());
        assert!(!request.is_model_not_available());
        assert_eq!(request.model_not_available_id(), None);
        assert_eq!(
            serde_json::to_value(request.user_facing_error(context())).unwrap(),
            json!({"code":"request_too_large","fields":{"provider":"provider","model_id":"context-model"}})
        );
        assert_eq!(
            request.user_facing_message(),
            "The conversation has become too long for the model to process. Please start a new session or reduce the context size."
        );
        let model = AgentLoopError::model_not_available("gpt-99");
        assert!(!model.is_request_too_large());
        assert!(model.is_model_not_available());
        assert_eq!(model.model_not_available_id(), Some("gpt-99"));
        assert_eq!(model.to_string(), "Model not available: gpt-99");
        assert_eq!(
            model.user_facing_message(),
            "The model `gpt-99` is not available. It may have been removed, renamed, or your API key may not have access to it. Please select a different model."
        );
        assert_eq!(
            serde_json::to_value(model.user_facing_error(context())).unwrap(),
            json!({"code":"model_unavailable","fields":{"provider":"provider","model_id":"gpt-99"}})
        );
        for other in [
            AgentLoopError::llm("Request too large: Model not available: gpt-99"),
            AgentLoopError::tool("failed"),
            AgentLoopError::Cancelled,
        ] {
            assert!(!other.is_request_too_large());
            assert!(!other.is_model_not_available());
            assert_eq!(other.model_not_available_id(), None);
        }
    }

    #[test]
    fn semantic_kinds_override_conflicting_text_for_predicates_and_payloads() {
        for (kind, message, predicates, code) in [
            (
                LlmErrorKind::Authentication,
                "(429) rate limit (503)",
                (false, true, false, false),
                "provider_misconfigured",
            ),
            (
                LlmErrorKind::QuotaExhausted,
                "(401) (429) rate limit (503)",
                (false, false, false, false),
                "provider_quota_exhausted",
            ),
            (
                LlmErrorKind::RateLimited,
                "(401) (503) insufficient_quota",
                (true, false, false, true),
                "provider_rate_limited",
            ),
            (
                LlmErrorKind::Unavailable,
                "(401) (429) insufficient_quota",
                (false, false, true, true),
                "provider_unavailable",
            ),
            (
                LlmErrorKind::InvalidRequest,
                "opaque private failure",
                (false, false, false, false),
                "processing_error",
            ),
        ] {
            let error = AgentLoopError::llm_kind(kind, message);
            assert_eq!(error.llm_error_kind(), Some(kind));
            assert_eq!(
                (
                    error.is_rate_limited(),
                    error.is_auth_error(),
                    error.is_server_error(),
                    error.is_transient_llm_error()
                ),
                predicates,
                "{kind:?}"
            );
            let mut fields = json!({"provider":"provider","model_id":"model"});
            if kind == LlmErrorKind::RateLimited {
                fields["retry_after"] = json!(12);
            }
            assert_eq!(
                serde_json::to_value(
                    error.user_facing_error(
                        UserFacingErrorContext::default()
                            .with_provider("provider")
                            .with_model_id("model")
                            .with_retry_after(12)
                    )
                )
                .unwrap(),
                json!({"code":code,"fields":fields})
            );
        }
    }

    #[test]
    fn legacy_predicates_and_user_copy_use_independent_literal_cases() {
        for (message, expected, copy) in [
            (
                "Anthropic API error (429): rate limit exceeded",
                (true, false, false),
                "Rate limited by the AI provider. Please wait a moment.",
            ),
            (
                "Rate limit exceeded (after 2 retries)",
                (true, false, false),
                "Rate limited by the AI provider. Please wait a moment.",
            ),
            (
                "too many requests",
                (true, false, false),
                "Rate limited by the AI provider. Please wait a moment.",
            ),
            (
                "Anthropic API error (401): invalid api key",
                (false, true, false),
                "There is a misconfiguration with the AI provider. Please contact support.",
            ),
            (
                "OpenAI API error (403): forbidden",
                (false, true, false),
                "There is a misconfiguration with the AI provider. Please contact support.",
            ),
            (
                "Anthropic API error (500): internal server error",
                (false, false, true),
                "The AI provider is experiencing issues. Please try again shortly.",
            ),
            (
                "OpenAI API error (503): service unavailable",
                (false, false, true),
                "The AI provider is experiencing issues. Please try again shortly.",
            ),
            (
                "Failed to send request: connection refused",
                (false, false, false),
                "I encountered an error while processing your request. Please try again later.",
            ),
        ] {
            let error = AgentLoopError::llm(message);
            assert_eq!(
                (
                    error.is_rate_limited(),
                    error.is_auth_error(),
                    error.is_server_error()
                ),
                expected,
                "{message}"
            );
            assert_eq!(error.user_facing_message(), copy, "{message}");
        }
        for status in [502, 504, 529] {
            assert!(AgentLoopError::llm(format!("error ({status})")).is_server_error());
        }
        let non_llm = AgentLoopError::tool("(401) (429) (503) rate limit");
        assert_eq!(
            (
                non_llm.is_rate_limited(),
                non_llm.is_auth_error(),
                non_llm.is_server_error(),
                non_llm.is_transient_llm_error()
            ),
            (false, false, false, false)
        );
    }

    #[test]
    fn provider_status_classification_covers_boundaries_and_quota_precedence() {
        for (status, expected) in [
            (200, LlmErrorKind::Other),
            (399, LlmErrorKind::Other),
            (400, LlmErrorKind::InvalidRequest),
            (401, LlmErrorKind::Authentication),
            (403, LlmErrorKind::Authentication),
            (404, LlmErrorKind::InvalidRequest),
            (408, LlmErrorKind::Unavailable),
            (409, LlmErrorKind::Unavailable),
            (429, LlmErrorKind::RateLimited),
            (499, LlmErrorKind::InvalidRequest),
            (500, LlmErrorKind::Unavailable),
            (501, LlmErrorKind::Other),
            (502, LlmErrorKind::Unavailable),
            (503, LlmErrorKind::Unavailable),
            (529, LlmErrorKind::Unavailable),
            (599, LlmErrorKind::Unavailable),
            (600, LlmErrorKind::Other),
        ] {
            assert_eq!(
                LlmErrorKind::from_provider_status(status, "opaque"),
                expected,
                "{status}"
            );
        }
        for message in [
            r#"{"error":{"type":"insufficient_quota"}}"#,
            r#"{"error":{"code":"credit_balance_exhausted"}}"#,
            r#"{"error":{"type":"usage_limit_reached"}}"#,
            "Your credit balance is too low to access the Anthropic API.",
        ] {
            for status in [400, 401, 429, 503] {
                assert_eq!(
                    LlmErrorKind::from_provider_status(status, message),
                    LlmErrorKind::QuotaExhausted,
                    "{status}: {message}"
                );
            }
        }
    }

    #[test]
    fn provider_text_classification_uses_independent_keywords_and_precedence() {
        for (message, expected) in [
            ("ThrottlingException", LlmErrorKind::RateLimited),
            ("TooManyRequestsException", LlmErrorKind::RateLimited),
            ("RATE LIMIT", LlmErrorKind::RateLimited),
            ("too many requests", LlmErrorKind::RateLimited),
            ("AccessDeniedException", LlmErrorKind::Authentication),
            ("UnrecognizedClientException", LlmErrorKind::Authentication),
            ("ExpiredTokenException", LlmErrorKind::Authentication),
            ("InvalidSignatureException", LlmErrorKind::Authentication),
            ("unauthorized", LlmErrorKind::Authentication),
            ("ServiceUnavailableException", LlmErrorKind::Unavailable),
            ("service unavailable", LlmErrorKind::Unavailable),
            ("InternalServerException", LlmErrorKind::Unavailable),
            ("ModelNotReadyException", LlmErrorKind::Unavailable),
            (
                "usage_limit_reached; resets_at=1783767823; throttlingexception",
                LlmErrorKind::QuotaExhausted,
            ),
            ("something else entirely", LlmErrorKind::Other),
        ] {
            assert_eq!(
                LlmErrorKind::from_error_text(message),
                expected,
                "{message}"
            );
        }
    }

    #[test]
    fn provider_prefix_preserves_kind_and_retry_metadata_without_duplication() {
        let metadata = crate::llm_retry::RetryMetadata {
            attempts: 2,
            total_retry_wait: std::time::Duration::from_millis(1234),
            ..Default::default()
        };
        let error = AgentLoopError::llm_kind(LlmErrorKind::Unavailable, "network failure")
            .with_retry_metadata(&metadata)
            .with_provider("custom")
            .with_provider("custom");
        assert_eq!(error.llm_retry_attempts(), 2);
        assert!(error.llm_retry_handled());
        let AgentLoopError::Llm(error) = error else {
            panic!("lost LLM variant")
        };
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            json!({"kind":"unavailable","message":"provider 'custom': network failure","retry_attempts":2,"retry_wait_ms":1234,"retry_handled":true})
        );
        let legacy: LlmError =
            serde_json::from_value(json!({"kind":"other","message":"legacy"})).unwrap();
        assert_eq!(
            serde_json::to_value(legacy).unwrap(),
            json!({"kind":"other","message":"legacy","retry_attempts":0,"retry_wait_ms":0,"retry_handled":false})
        );
        let non_llm = AgentLoopError::Cancelled
            .with_retry_metadata(&metadata)
            .with_provider("custom");
        assert!(matches!(non_llm, AgentLoopError::Cancelled));
        assert_eq!(non_llm.llm_retry_attempts(), 0);
        assert!(!non_llm.llm_retry_handled());
    }

    #[test]
    fn missing_model_and_iteration_limits_have_complete_safe_payloads() {
        let missing = AgentLoopError::model_not_configured();
        assert!(missing.is_non_retryable());
        assert_eq!(
            missing.user_facing_message(),
            "No model is configured for this chat. Choose a model or configure a default model, then try again."
        );
        assert_eq!(
            serde_json::to_value(missing.user_facing_error(UserFacingErrorContext::default()))
                .unwrap(),
            json!({"code":"model_not_configured"})
        );
        assert_eq!(
            serde_json::to_value(
                AgentLoopError::MaxIterationsReached(7)
                    .user_facing_error(UserFacingErrorContext::default())
            )
            .unwrap(),
            json!({"code":"max_iterations","fields":{"max_iterations":7}})
        );
    }

    #[test]
    fn store_adapter_preserves_success_and_exact_error_variant_and_message() {
        let success: std::result::Result<Vec<String>, String> =
            Ok(vec!["first".into(), "second".into()]);
        assert_eq!(success.store_err().unwrap(), ["first", "second"]);
        let failure: std::result::Result<(), std::io::Error> =
            Err(std::io::Error::other("db unavailable"));
        let error = failure.store_err().unwrap_err();
        assert_eq!(error.to_string(), "Message store error: db unavailable");
        assert!(matches!(error,AgentLoopError::MessageStore(message) if message=="db unavailable"));
    }

    #[test]
    fn json_helpers_preserve_structures_and_apply_documented_error_defaults() {
        assert_eq!(json_val(&vec![1, 2, 3]), json!([1, 2, 3]));
        assert_eq!(from_json::<Vec<String>>(json!(["a", "b"])), ["a", "b"]);
        assert_eq!(from_json::<i32>(json!("not a number")), 0);
        struct Fails;
        impl Serialize for Fails {
            fn serialize<S: serde::Serializer>(
                &self,
                _: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("synthetic serialization failure"))
            }
        }
        assert_eq!(json_val(&Fails), serde_json::Value::Null);
    }
}
