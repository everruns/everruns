// LLM error hook seam
//
// An in-process capability hook for reacting to a *terminal* LLM error (a turn
// that failed and will not be retried). This is the platform seam that lets a
// capability encapsulate error-recovery behavior — augmenting the user-facing
// error copy and/or performing a side effect — without the reason atom hard-
// coding any capability-specific logic. It belongs to the same in-process hook
// family as `ToolCallHook`, `MessageFilterProvider`, and `output_guardrails`
// (typed traits returned from a `Capability::*` seam and invoked in-process with
// host services) — as opposed to the user-hook system (`user_hook_types`), which
// runs user-authored shell commands. The reason atom collects the hooks from the
// active capabilities and invokes each generically.
//
// The first consumer is `usage_limit_auto_continue`, which schedules a
// continuation after a provider usage limit resets, but the seam is deliberately
// provider- and behavior-agnostic so other extensions can be built the same way.

use async_trait::async_trait;
use std::sync::Arc;

use crate::session_services::SessionScheduleStore;
use crate::typed_id::SessionId;
use crate::user_facing_error::UserFacingErrorFields;
use serde_json::Value;

/// Host services made available to error hooks. Extend as new hooks need more
/// surface; today only the session schedule store is exposed (each field is
/// optional so a hook degrades to a no-op when its service is absent).
#[derive(Clone, Default)]
pub struct LlmErrorHookServices {
    pub schedule_store: Option<Arc<dyn SessionScheduleStore>>,
}

/// Context handed to a capability's [`LlmErrorHook`] on a terminal LLM error.
pub struct LlmErrorContext<'a> {
    /// Session whose turn failed.
    pub session_id: SessionId,
    /// Classified user-facing error code (e.g. `provider_usage_limit_reached`).
    pub error_code: &'a str,
    /// Structured fields captured by the classifier (e.g. `resets_at`).
    pub error_fields: &'a UserFacingErrorFields,
    /// The contributing capability's per-agent config JSON (may be `Null`).
    pub config: &'a Value,
    /// Host services the hook may use to perform side effects.
    pub services: &'a LlmErrorHookServices,
}

/// Result of an [`LlmErrorHook`]: extra fields to merge into the user-facing
/// error (for example to unlock capability-specific message copy). The default
/// is a no-op that changes nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LlmErrorHookOutcome {
    /// Fields to merge into the `UserFacingError` before it is rendered/emitted.
    pub extra_error_fields: UserFacingErrorFields,
}

impl LlmErrorHookOutcome {
    /// An outcome that changes nothing.
    pub fn noop() -> Self {
        Self::default()
    }

    /// Add a field to merge into the user-facing error.
    pub fn with_error_field(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.extra_error_fields.insert(key.into(), value.into());
        self
    }
}

/// A capability's reaction to a terminal LLM error. Provided via
/// `Capability::llm_error_hook`. Runs only on the terminal (non-retried)
/// error path, before the user-facing error message is emitted.
#[async_trait]
pub trait LlmErrorHook: Send + Sync {
    async fn on_llm_error(&self, ctx: &LlmErrorContext<'_>) -> LlmErrorHookOutcome;
}
