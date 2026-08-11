//! Message token estimation used by execution budgeting.
//!
//! The kernel reports estimated token counts on turn events and decides when a
//! context is over budget, so the estimator lives here rather than with the
//! compaction capability that also consumes it (EVE-884).

use crate::driver_registry::{LlmContentPart, LlmMessage, LlmMessageContent};

/// Estimate token count for an LLM message using char/4 approximation.
///
/// This is intentionally simple. More accurate estimation (tiktoken, etc.) can
/// be swapped in later, but char/4 is sufficient for budget decisions.
pub fn estimate_tokens(msg: &LlmMessage) -> usize {
    let text_len = match &msg.content {
        LlmMessageContent::Text(t) => t.len(),
        LlmMessageContent::Parts(parts) => parts
            .iter()
            .map(|p| match p {
                LlmContentPart::Text { text } => text.len(),
                _ => 50, // images, etc. — rough estimate
            })
            .sum(),
    };

    // Add tool call overhead
    let tool_call_len = msg
        .tool_calls
        .as_ref()
        .map(|calls| {
            calls
                .iter()
                .map(|tc| tc.name.len() + tc.arguments.to_string().len() + 20)
                .sum::<usize>()
        })
        .unwrap_or(0);

    (text_len + tool_call_len) / 4
}

/// Estimate total tokens for a slice of messages.
pub fn estimate_total_tokens(messages: &[LlmMessage]) -> usize {
    messages.iter().map(estimate_tokens).sum()
}
