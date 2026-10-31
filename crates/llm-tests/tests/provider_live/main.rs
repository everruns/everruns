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

mod anthropic_parallel_tool_calls;
mod decision_discovery;
mod discovery;
mod fireworks_chat;
mod meta_model_api;
mod openai_agents_api;
mod openai_astra_compaction;
mod openai_completions_max_tokens;
mod openai_hosted_tools;
mod openai_native_async;
mod openai_parallel_tool_calls;
mod openrouter_chat;
mod openrouter_discovery;
