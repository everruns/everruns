//! Neutral command execution contracts.
//!
//! Store-backed context loading, credential-bearing provider resolution, and
//! completion driver creation live in `everruns-host`.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::command::CommandResult;
use crate::driver_registry::LlmResponseStream;
use crate::error::{AgentLoopError, Result};
use crate::message::{Controls, Message};
use crate::typed_id::SessionId;
use crate::user_facing_error::{UserFacingErrorContext, classify_runtime_error_message};

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
    pub messages: Vec<Message>,
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
    pub messages: Vec<Message>,
    /// Per-invocation model and reasoning controls.
    pub controls: Option<Controls>,
    /// Extra provider metadata. The host adds `session_id` itself.
    pub metadata: HashMap<String, String>,
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
