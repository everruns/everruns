// Error types for the agent loop
//
// StoreResultExt: extension trait to replace repeated .map_err(|e| AgentLoopError::store(...))? patterns
// json_val / from_json: helpers to replace repeated serde_json::to_value/from_value boilerplate

use crate::typed_id::{AgentId, HarnessId, SessionId};
use crate::user_facing_error::{
    AttestationRequirement, UserFacingError, UserFacingErrorContext,
    classify_runtime_error_message, codes as user_facing_error_codes,
    parse_attestation_requirement,
};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

/// Result type alias for agent loop operations
pub type Result<T> = std::result::Result<T, AgentLoopError>;

// Stable provider errors remain re-exported here for compatibility; internal
// capability markers stay private to keep this surface focused.
pub use crate::llm_error::{BillingPressureReason, LlmError, LlmErrorKind};
use crate::llm_error::{RejectedProviderCapability, provider_error_code_in};
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
    /// Prefix provider-bound messages without changing structured identifiers.
    pub fn with_provider(mut self, provider: &str) -> Self {
        let prefix = format!("provider '{provider}': ");
        match &mut self {
            AgentLoopError::Llm(error) if !error.message.starts_with(&prefix) => {
                error.message.insert_str(0, &prefix)
            }
            AgentLoopError::RequestTooLarge(message) | AgentLoopError::Configuration(message)
                if !message.starts_with(&prefix) =>
            {
                message.insert_str(0, &prefix)
            }
            // ModelNotAvailable stores a model ID, not a free-form message.
            _ => {}
        }
        self
    }

    /// Create an LLM error with no semantic kind (falls back to string
    /// classification downstream).
    pub fn llm(msg: impl Into<String>) -> Self {
        AgentLoopError::Llm(LlmError::new(LlmErrorKind::Other, msg))
    }

    /// Create an LLM error with a semantic kind assigned at the driver boundary.
    pub fn llm_kind(kind: LlmErrorKind, msg: impl Into<String>) -> Self {
        AgentLoopError::Llm(LlmError::new(kind, msg))
    }

    /// Create an LLM error from an HTTP failure, classifying it and keeping
    /// the status.
    ///
    /// This is the constructor HTTP drivers reach for: it is the one place
    /// that both classifies (via
    /// [`LlmErrorKind::from_provider_status`]) and preserves the status a
    /// consumer would otherwise have to parse back out of the message.
    pub fn llm_http(status: u16, body: &str, msg: impl Into<String>) -> Self {
        Self::llm_http_kind(
            LlmErrorKind::from_provider_status(status, body),
            status,
            body,
            msg,
        )
    }

    /// Create an LLM error from an HTTP failure a driver has already
    /// classified, keeping the status and the provider's error code.
    ///
    /// The counterpart to [`llm_http`](Self::llm_http) for drivers whose
    /// protocol extension classifies more precisely than status and body
    /// alone allow.
    pub fn llm_http_kind(
        kind: LlmErrorKind,
        status: u16,
        body: &str,
        msg: impl Into<String>,
    ) -> Self {
        let mut error = LlmError::new(kind, msg).with_status(status);
        if let Some(code) = provider_error_code_in(body) {
            error = error.with_code(code);
        }
        AgentLoopError::Llm(error)
    }

    /// Create a structured pre-stream provider capability rejection.
    pub fn provider_capability_rejected(
        capability: RejectedProviderCapability,
        status: u16,
        body: &str,
        msg: impl Into<String>,
    ) -> Self {
        let mut error = LlmError::new(LlmErrorKind::InvalidRequest, msg)
            .with_status(status)
            .with_rejected_capability(capability);
        if let Some(code) = provider_error_code_in(body) {
            error = error.with_code(code);
        }
        AgentLoopError::Llm(error)
    }

    /// Whether this error rejects the specified provider capability.
    pub fn rejected_provider_capability(&self, capability: RejectedProviderCapability) -> bool {
        matches!(
            self,
            AgentLoopError::Llm(error)
                if error.rejected_capability == Some(capability)
        )
    }

    /// Attach the HTTP status to an LLM error that was classified elsewhere.
    ///
    /// No-op on non-LLM variants, whose status is implied by the variant and
    /// reported by [`http_status`](Self::http_status).
    #[must_use]
    pub fn with_status(mut self, status: u16) -> Self {
        if let AgentLoopError::Llm(error) = &mut self {
            error.status = Some(status);
        }
        self
    }

    /// Attach the delay the provider asked for before another attempt.
    #[must_use]
    pub fn with_retry_after_secs(mut self, secs: u64) -> Self {
        if let AgentLoopError::Llm(error) = &mut self {
            error.retry_after_secs = Some(secs);
        }
        self
    }

    /// The HTTP status this failure corresponds to, if any.
    ///
    /// Reports the recorded status for [`Llm`](Self::Llm), and the status the
    /// variant itself stands for where Everruns classified the failure
    /// semantically instead of carrying one:
    /// [`ModelNotAvailable`](Self::ModelNotAvailable) is `404` and
    /// [`RequestTooLarge`](Self::RequestTooLarge) is `413`. Everything else is
    /// `None` — a caller putting an API in front of Everruns picks its own
    /// status rather than being handed a guess.
    pub fn http_status(&self) -> Option<u16> {
        match self {
            AgentLoopError::Llm(error) => error.status,
            AgentLoopError::ModelNotAvailable(_) => Some(404),
            AgentLoopError::RequestTooLarge(_) => Some(413),
            _ => None,
        }
    }

    /// The provider's own machine-readable error code, if one was preserved.
    pub fn provider_error_code(&self) -> Option<&str> {
        match self {
            AgentLoopError::Llm(error) => error.code.as_deref(),
            _ => None,
        }
    }

    /// The delay the provider asked for before another attempt, in seconds.
    ///
    /// Reads the recorded `Retry-After` first, then the delay
    /// [`LlmErrorKind::BillingPressure`] carries.
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            AgentLoopError::Llm(error) => error.retry_after_secs.or(match error.kind {
                LlmErrorKind::BillingPressure {
                    retry_after_secs, ..
                } => retry_after_secs,
                _ => None,
            }),
            _ => None,
        }
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
                // A recorded status is the provider's own answer; the string
                // scan below is the fallback for failures that carry none.
                LlmErrorKind::Other => match err.status {
                    Some(status) => status == 429,
                    None => {
                        let msg_lower = err.message.to_ascii_lowercase();
                        msg_lower.contains("(429)")
                            || msg_lower.contains("rate limit")
                            || msg_lower.contains("too many requests")
                    }
                },
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
                LlmErrorKind::Other => match err.status {
                    Some(status) => status == 401 || status == 403,
                    None => err.message.contains("(401)") || err.message.contains("(403)"),
                },
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
                LlmErrorKind::Other => match err.status {
                    Some(status) => status >= 500,
                    None => {
                        let msg = &err.message;
                        msg.contains("(500)")
                            || msg.contains("(502)")
                            || msg.contains("(503)")
                            || msg.contains("(504)")
                            || msg.contains("(529)")
                    }
                },
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
                | LlmErrorKind::BillingPressure { .. }
                | LlmErrorKind::AttestationRequired
                | LlmErrorKind::Refusal
                | LlmErrorKind::MalformedResponse
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

            // The reason phase already attempted permitted compaction before
            // returning overflow. Replaying unchanged input cannot make it fit.
            AgentLoopError::RequestTooLarge(_) => true,

            // MessageStore "not found" errors (deleted messages).
            AgentLoopError::MessageStore(msg) => msg.to_ascii_lowercase().contains("not found"),

            AgentLoopError::Llm(error) if error.kind == LlmErrorKind::Refusal => true,

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
                    LlmErrorKind::BillingPressure { reason, .. } => Some(match reason {
                        BillingPressureReason::InFlightBudgetExhausted => {
                            user_facing_error_codes::PROVIDER_RATE_LIMITED
                        }
                        BillingPressureReason::InsufficientCredits => {
                            user_facing_error_codes::PROVIDER_QUOTA_EXHAUSTED
                        }
                    }),
                    LlmErrorKind::RateLimited => {
                        Some(user_facing_error_codes::PROVIDER_RATE_LIMITED)
                    }
                    LlmErrorKind::Unavailable => {
                        Some(user_facing_error_codes::PROVIDER_UNAVAILABLE)
                    }
                    LlmErrorKind::AttestationRequired => {
                        Some(user_facing_error_codes::PROVIDER_ATTESTATION_REQUIRED)
                    }
                    LlmErrorKind::Refusal => Some(user_facing_error_codes::MODEL_REFUSAL),
                    LlmErrorKind::InvalidRequest
                    | LlmErrorKind::MalformedResponse
                    | LlmErrorKind::Other => None,
                };
                match code {
                    Some(code) => {
                        let error = UserFacingError::new(code)
                            .with_optional_field("provider", context.provider)
                            .with_optional_field("model_id", context.model_id);
                        if code == user_facing_error_codes::PROVIDER_ATTESTATION_REQUIRED {
                            // The attestation variant carries no payload, so
                            // the confirmations and URL are read from the raw
                            // body the driver retained in `message`.
                            parse_attestation_requirement(&err.message)
                                .unwrap_or_else(AttestationRequirement::fallback)
                                .apply_fields(error)
                        } else {
                            crate::provider_limit::apply_provider_limit_fields(
                                error,
                                err,
                                context.retry_after,
                            )
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
/// The file-system tools decide
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
/// untyped implementors keep their current routing.
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
mod tests;
