#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live provider tests: each one calls a real vendor API and needs its key.
//!
//! Decision: these never skip. A missing key fails the test, so a run that
//! reports green actually reached every vendor; nothing here is `#[ignore]`d
//! or returns early. They live in this unpublished crate, which only the
//! nightly `Live Provider Matrix` job tests (with Doppler secrets), instead of
//! next to the drivers, whose suite runs keyless on every PR.
//!
//! ```text
//! doppler run -- cargo test -p everruns-llm-tests --test provider_live -- --nocapture
//! ```

// Shared with the fake-API suite in the `everruns` crate.
#[path = "../../../everruns/tests/host/agents_api_support/mod.rs"]
mod agents_api_support;

mod anthropic_message_threads;
mod anthropic_parallel_tool_calls;
mod decision_discovery;
mod discovery;
// Paused 2026-10-31: the Fireworks account is suspended (spending limit), so
// every call returns 412. Restore `fireworks_chat.rs` from git history and
// re-add its `mod` once billing is restored.
mod meta_model_api;
mod openai_agents_api;
mod openai_astra_compaction;
mod openai_completions_max_tokens;
mod openai_hosted_tools;
mod openai_images;
mod openai_native_async;
mod openai_parallel_tool_calls;
mod openrouter_chat;
mod openrouter_discovery;

/// Attempts a live call gets when it dies in transport: the live matrix's
/// budget (three attempts).
pub(crate) const TRANSPORT_ATTEMPTS: u32 = 3;

/// Whether a failed attempt died in transport rather than in the behavior under
/// test. Nightlies lost runs to OpenAI dropping the stream mid-body ("error
/// decoding response body": native async on 2026-10-31, hosted web search on
/// 2026-11-02); the matrix already retries that class
/// (`llm_test_matrix::is_transient_transport_error`). Kept narrow: a provider
/// or contract error still fails on the first attempt.
pub(crate) fn is_transient_transport(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "transport error",
        "error decoding response body",
        "connection reset",
        "connection closed",
        "unexpected eof",
        "incomplete message",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}
