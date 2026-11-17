//! Realistic answers to the runtime's compaction summarization call.
//!
//! Decisions:
//! - The call is recognised by the runtime's summarization system prompt
//!   ("Summarize the following conversation history"). Without this the call
//!   got the driver's normal one-line answer, instantly, so a compacted
//!   session lost almost all of its context size and no compaction ever took
//!   time.
//! - The summary is about a tenth of the input, between 256 and 1,200 tokens
//!   (the runtime caps the call at 2,000), which is what real summaries of
//!   long histories come to.
//! - Timing follows the call's own latency profile, so tests on the instant
//!   default stay instant. A history that carries `[[llmsim:` scenario markers
//!   is a load test, and there the summary is paced like a hosted model
//!   (0.9 s first token, 15 ms a token, plus prefill): 5 to 25 s, as real
//!   compaction calls take.

use everruns_contracts::driver_registry::{Message, MessageRole};
use llmsim::generator::{LoremGenerator, ResponseGenerator};
use llmsim::latency::{DEFAULT_PREFILL_US_PER_TOKEN, LatencyProfile};

/// Words that open the runtime's summarization prompt.
const SUMMARIZATION_PROMPT: &str = "Summarize the following conversation history";
const MIN_SUMMARY_TOKENS: u32 = 256;
const MAX_SUMMARY_TOKENS: u32 = 1200;

/// Whether this call is the runtime asking for a compaction summary.
pub(crate) fn is_summarization(messages: &[Message]) -> bool {
    messages.iter().any(|message| {
        message.role == MessageRole::System
            && message.content_as_text().contains(SUMMARIZATION_PROMPT)
    })
}

/// Summary length for a prompt of `prompt_tokens`.
pub(crate) fn summary_tokens(prompt_tokens: u32) -> u32 {
    (prompt_tokens / 10).clamp(MIN_SUMMARY_TOKENS, MAX_SUMMARY_TOKENS)
}

/// The summary text.
pub(crate) fn summary_text(
    driver: &crate::LlmSimDriver,
    messages: &[Message],
    tokens: u32,
) -> String {
    LoremGenerator::new(tokens as usize).generate(&driver.to_chat_request(messages))
}

/// Pacing for the summarization call: the call's own profile, or a hosted
/// model's when the history comes from a scenario load test.
pub(crate) fn pacing(messages: &[Message], profile: LatencyProfile) -> LatencyProfile {
    let from_load_test = messages
        .iter()
        .any(|message| message.content_as_text().contains("[[llmsim:"));
    if from_load_test && profile.ttft_mean_ms == 0 {
        LatencyProfile::new(900, 250, 15, 5).with_prefill_us_per_token(DEFAULT_PREFILL_US_PER_TOKEN)
    } else {
        profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_are_a_tenth_of_the_input_within_bounds() {
        assert_eq!(summary_tokens(100), 256);
        assert_eq!(summary_tokens(5_000), 500);
        assert_eq!(summary_tokens(200_000), 1200);
    }

    #[test]
    fn only_the_summarization_prompt_is_recognised() {
        let summarize = Message::text(
            MessageRole::System,
            "<task>\nSummarize the following conversation history. The summary replaces",
        );
        let user = Message::text(
            MessageRole::User,
            "Summarize the following conversation history",
        );
        assert!(is_summarization(&[summarize, user.clone()]));
        assert!(!is_summarization(&[user]));
    }

    #[test]
    fn load_test_histories_get_hosted_pacing_and_tests_stay_instant() {
        let plain = [Message::text(MessageRole::User, "hello")];
        let marked = [Message::text(
            MessageRole::User,
            "{\"text\":\"go [[llmsim:chat-short]]\"}",
        )];
        assert_eq!(pacing(&plain, LatencyProfile::instant()).ttft_mean_ms, 0);
        assert_eq!(pacing(&marked, LatencyProfile::instant()).ttft_mean_ms, 900);
        // An explicit profile is kept either way.
        assert_eq!(
            pacing(&marked, LatencyProfile::fast()).ttft_mean_ms,
            LatencyProfile::fast().ttft_mean_ms
        );
    }
}
