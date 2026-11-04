//! Neutral command execution contracts.
//!
//! Store-backed context loading, credential-bearing provider resolution, and
//! completion driver creation live in `everruns_contracts::runtime::host`.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::runtime::command::CommandResult;
use crate::runtime::driver_registry::LlmResponseStream;
use crate::runtime::error::{AgentLoopError, Result};
use crate::runtime::message::{Controls, RuntimeMessage};
use crate::runtime::typed_id::SessionId;
use crate::runtime::user_facing_error::{UserFacingErrorContext, classify_runtime_error_message};

/// Credential-free snapshot of the session's assembled turn context for
/// command execution.
///
/// The host applies capability message filters and prompt contributions before
/// producing this view. Provider credentials, endpoints, and persisted session
/// records never cross the contract boundary.
#[derive(Debug, Clone)]
pub struct CommandTurnContext {
    /// Session the command is executing against.
    pub session_id: SessionId,
    /// Conversation messages after capability message filters.
    pub messages: Vec<RuntimeMessage>,
    /// Merged system prompt including capability contributions.
    pub system_prompt: String,
    /// Resolved model name, without credentials.
    pub model: String,
    /// Resolved provider integration kind for user-facing error classification.
    pub provider_type: String,
    /// Locale resolved from message controls or session defaults.
    pub resolved_locale: Option<String>,
}

/// Request for a tool-less, out-of-band completion against the session model.
#[derive(Debug, Clone, Default)]
pub struct SessionCompletionRequest {
    /// System prompts sent in order; empty entries are skipped.
    pub system_prompts: Vec<String>,
    /// Conversation messages to complete against.
    pub messages: Vec<RuntimeMessage>,
    /// Per-invocation model and reasoning controls.
    pub controls: Option<Controls>,
    /// Extra provider metadata, such as `purpose`. Validated with
    /// [`Self::validate_metadata`]; the host adds `session_id` itself and
    /// merges it with its own metadata via [`completion_metadata`].
    pub metadata: HashMap<String, String>,
}

/// Provider metadata keys only the executing host stamps, from its own
/// authority (session, harness, turn and model identity). A request carrying
/// one is invalid rather than silently overwritten, so a caller can never
/// spoof the correlation ids providers and traces record.
pub const RESERVED_COMPLETION_METADATA_KEYS: &[&str] = &[
    "session_id",
    "harness_id",
    "turn_id",
    "exec_id",
    "org_id",
    "agent_id",
    "model_id",
];

/// Most request metadata entries a completion may carry. Providers cap
/// metadata (OpenAI accepts 16 pairs), and the host adds its own.
pub const MAX_COMPLETION_METADATA_ENTRIES: usize = 8;
/// Longest request metadata key, in bytes.
pub const MAX_COMPLETION_METADATA_KEY_LEN: usize = 64;
/// Longest request metadata value, in bytes.
pub const MAX_COMPLETION_METADATA_VALUE_LEN: usize = 256;

impl SessionCompletionRequest {
    /// Check the request metadata is bounded, well formed and free of
    /// [`RESERVED_COMPLETION_METADATA_KEYS`].
    ///
    /// Keys are 1..=64 bytes of ASCII lowercase letters, digits, `_`, `-` or
    /// `.`; values are at most 256 bytes without control characters.
    pub fn validate_metadata(&self) -> Result<()> {
        if self.metadata.len() > MAX_COMPLETION_METADATA_ENTRIES {
            return Err(AgentLoopError::config(format!(
                "completion metadata has {} entries; at most {MAX_COMPLETION_METADATA_ENTRIES} \
                 are allowed",
                self.metadata.len()
            )));
        }
        for (key, value) in &self.metadata {
            let well_formed = !key.is_empty()
                && key.len() <= MAX_COMPLETION_METADATA_KEY_LEN
                && key.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-' | b'.')
                });
            if !well_formed {
                return Err(AgentLoopError::config(format!(
                    "completion metadata key {key:?} must be 1-{MAX_COMPLETION_METADATA_KEY_LEN} \
                     lowercase ASCII letters, digits, '_', '-' or '.'"
                )));
            }
            if RESERVED_COMPLETION_METADATA_KEYS.contains(&key.as_str()) {
                return Err(AgentLoopError::config(format!(
                    "completion metadata key {key:?} is reserved for the host"
                )));
            }
            if value.len() > MAX_COMPLETION_METADATA_VALUE_LEN
                || value.chars().any(char::is_control)
            {
                return Err(AgentLoopError::config(format!(
                    "completion metadata value for {key:?} must be at most \
                     {MAX_COMPLETION_METADATA_VALUE_LEN} bytes without control characters"
                )));
            }
        }
        Ok(())
    }
}

/// Assemble the provider metadata of an out-of-band completion.
///
/// Precedence, lowest first: the host's own request metadata (embedder
/// metadata a turn's model calls carry too), then the request's metadata
/// (its `purpose`, say: the more specific statement wins), then the
/// authority-owned `session_id`, which nothing overrides. Turn model calls
/// follow the same order with their own authority keys.
pub fn completion_metadata<'a>(
    host: impl IntoIterator<Item = (&'a String, &'a String)>,
    request: &HashMap<String, String>,
    session_id: SessionId,
) -> HashMap<String, String> {
    let mut metadata: HashMap<String, String> = host
        .into_iter()
        .filter(|(key, _)| !RESERVED_COMPLETION_METADATA_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    metadata.extend(
        request
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    metadata.insert("session_id".to_string(), session_id.to_string());
    metadata
}

/// Successful command completion result.
#[derive(Debug, Clone)]
pub struct SessionCompletion {
    /// Trimmed, non-empty completion text.
    pub text: String,
}

/// Streaming command completion result.
pub struct SessionCompletionStream {
    /// Provider stream events for progressive output.
    pub events: LlmResponseStream,
    /// Credential-free provider/model identity for classifying stream errors.
    pub context: UserFacingErrorContext,
}

impl std::fmt::Debug for SessionCompletionStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCompletionStream")
            .field("context", &self.context)
            .finish()
    }
}

/// Command completion failure.
#[derive(Debug)]
pub enum SessionCompletionError {
    /// Request-level failure, such as an unknown model override.
    InvalidRequest(AgentLoopError),
    /// The host does not implement streaming completion.
    StreamingUnsupported,
    /// Provider/runtime failure with safe classification context.
    Completion {
        /// Formatted error chain.
        error: String,
        /// Credential-free provider/model identity.
        context: UserFacingErrorContext,
    },
}

impl SessionCompletionError {
    /// Convert provider failures into a stable command result while allowing
    /// invalid requests to remain hard errors.
    pub fn into_command_result(self) -> Result<CommandResult> {
        match self {
            Self::InvalidRequest(error) => Err(error),
            Self::StreamingUnsupported => Err(AgentLoopError::config(
                "command host does not support streaming completions",
            )),
            Self::Completion { error, context } => {
                let classified = classify_runtime_error_message(&error, &context);
                Ok(CommandResult {
                    success: false,
                    message: classified.fallback_message(),
                    error_code: Some(classified.code.clone()),
                    error_fields: classified.error_fields(),
                })
            }
        }
    }
}

/// Host facilities available to capability command implementations.
///
/// Completions are out-of-band: this contract does not persist messages or
/// events. Hosts without these facilities use [`DisabledCommandHost`].
#[async_trait]
pub trait CommandHost: Send + Sync {
    async fn dreaming_proposals(&self, _accept: bool, _discard: bool) -> Result<String> {
        Err(AgentLoopError::config("dreaming review unavailable"))
    }

    async fn dream(&self) -> Result<bool> {
        Err(AgentLoopError::config(
            "host does not support dreaming commands",
        ))
    }

    /// Assemble the credential-free context a main turn would see.
    async fn turn_context(&self) -> Result<CommandTurnContext>;

    /// Run a tool-less completion against the resolved session model.
    async fn completion(
        &self,
        request: SessionCompletionRequest,
    ) -> std::result::Result<SessionCompletion, SessionCompletionError>;

    /// Stream a tool-less completion. The default advertises an unsupported
    /// capability so commands can fall back to [`Self::completion`].
    async fn completion_stream(
        &self,
        _request: SessionCompletionRequest,
    ) -> std::result::Result<SessionCompletionStream, SessionCompletionError> {
        Err(SessionCompletionError::StreamingUnsupported)
    }
}

/// Stub used by hosts that do not provide context-aware command facilities.
pub struct DisabledCommandHost;

#[async_trait]
impl CommandHost for DisabledCommandHost {
    async fn turn_context(&self) -> Result<CommandTurnContext> {
        Err(AgentLoopError::config(
            "command host does not provide turn-context access",
        ))
    }

    async fn completion(
        &self,
        _request: SessionCompletionRequest,
    ) -> std::result::Result<SessionCompletion, SessionCompletionError> {
        Err(SessionCompletionError::InvalidRequest(
            AgentLoopError::config("command host does not provide session completions"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(pairs: &[(&str, &str)]) -> SessionCompletionRequest {
        SessionCompletionRequest {
            metadata: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn metadata_validation_rejects_reserved_malformed_and_unbounded_entries() {
        request(&[("purpose", "completion_review"), ("command", "btw")])
            .validate_metadata()
            .expect("curated keys are accepted");
        for reserved in RESERVED_COMPLETION_METADATA_KEYS {
            assert!(request(&[(reserved, "spoof")]).validate_metadata().is_err());
        }
        let long_key = "k".repeat(65);
        for bad_key in ["", "Purpose", "has space", long_key.as_str()] {
            assert!(request(&[(bad_key, "x")]).validate_metadata().is_err());
        }
        assert!(
            request(&[("purpose", "line\nbreak")])
                .validate_metadata()
                .is_err()
        );
        let long_value = "v".repeat(257);
        assert!(
            request(&[("purpose", long_value.as_str())])
                .validate_metadata()
                .is_err()
        );
        let keys: Vec<String> = (0..=MAX_COMPLETION_METADATA_ENTRIES)
            .map(|index| format!("k{index}"))
            .collect();
        let many: Vec<(&str, &str)> = keys.iter().map(|key| (key.as_str(), "v")).collect();
        assert!(request(&many).validate_metadata().is_err());
    }

    #[test]
    fn completion_metadata_orders_host_then_request_then_authority() {
        let session_id = SessionId::new();
        let host: HashMap<String, String> = [
            ("purpose", "host-default"),
            ("tenant", "acme"),
            ("session_id", "spoofed"),
            ("turn_id", "spoofed"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
        let metadata = completion_metadata(
            &host,
            &request(&[("purpose", "completion_review")]).metadata,
            session_id,
        );
        assert_eq!(metadata["purpose"], "completion_review");
        assert_eq!(metadata["tenant"], "acme");
        assert_eq!(metadata["session_id"], session_id.to_string());
        assert!(
            !metadata.contains_key("turn_id"),
            "a completion has no turn; host values never stand in for authority keys"
        );
    }
}
