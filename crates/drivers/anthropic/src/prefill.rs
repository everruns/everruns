//! Assistant-prefill guard, split out of `driver.rs` to keep that file under
//! its size ratchet.

use everruns_provider::driver_registry::{Message, MessageRole};
use everruns_provider::error::{AgentLoopError, Result};

use crate::driver::uses_adaptive_thinking;

/// Reject a request that ends on an assistant turn for models that no longer
/// accept assistant prefill.
///
/// Every adaptive-thinking family (Fable 5.x, Opus 5.5/5/4.8/4.7/4.6, Sonnet 5
/// and 4.6) answers such a request with a 400. Failing here, before the
/// network call and its retries, gives the caller a configuration error that
/// says what to change. Assistant turns earlier in the conversation are
/// ordinary history and stay allowed; system messages are ignored because the
/// driver folds them into the top-level `system` field.
pub(crate) fn reject_trailing_assistant(wire_model: &str, messages: &[Message]) -> Result<()> {
    let last = messages
        .iter()
        .rev()
        .find(|m| m.role != MessageRole::System);
    if uses_adaptive_thinking(wire_model) && last.is_some_and(|m| m.role == MessageRole::Assistant)
    {
        return Err(AgentLoopError::config(format!(
            "{wire_model} does not support assistant message prefill: the conversation \
             must end with a user or tool message. Use structured outputs or a system \
             instruction to shape the response instead."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs(roles: &[MessageRole]) -> Vec<Message> {
        roles
            .iter()
            .map(|r| Message::text(r.clone(), "x"))
            .collect()
    }

    #[test]
    fn a_trailing_assistant_turn_is_rejected_on_adaptive_models() {
        use MessageRole::*;
        for model in ["claude-opus-5-5", "claude-fable-5-1", "claude-sonnet-4-6"] {
            let err = reject_trailing_assistant(model, &msgs(&[User, Assistant])).unwrap_err();
            assert!(err.to_string().contains("prefill"), "{model}: {err}");
            // A trailing system notice does not hide the prefill.
            assert!(reject_trailing_assistant(model, &msgs(&[User, Assistant, System])).is_err());
        }
    }

    #[test]
    fn history_and_older_models_are_allowed() {
        use MessageRole::*;
        let history = msgs(&[System, User, Assistant, User]);
        assert!(reject_trailing_assistant("claude-opus-5-5", &history).is_ok());
        // Budget-based models still accept prefill.
        assert!(reject_trailing_assistant("claude-sonnet-4-5", &msgs(&[User, Assistant])).is_ok());
    }
}
