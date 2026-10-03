#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live check that an output cap reaches OpenAI Chat Completions.
//!
//! OpenAI deprecated `max_tokens` and current models reject it outright:
//!
//! ```text
//! HTTP 400 Unsupported parameter: 'max_tokens' is not supported with this
//! model. Use 'max_completion_tokens' instead.
//! ```
//!
//! The protocol driver sends `max_completion_tokens` to OpenAI-family hosts for
//! that reason. Only a live request proves it, because the failure is the
//! vendor rejecting a field name — a mock would answer whatever it was told to.
//!
//! Ignored by default (network + `OPENAI_API_KEY`); run manually:
//!
//! ```text
//! doppler run -- cargo test -p everruns-openai --test completions_max_tokens_live -- --ignored --nocapture
//! ```

use everruns_openai::completions_provider;
use everruns_provider::driver_registry::{LlmCallConfig, Message, MessageRole};

/// A capped request must succeed and be capped — not rejected for naming the
/// deprecated field, and not silently uncapped.
#[tokio::test]
#[ignore = "live network + OPENAI_API_KEY"]
async fn a_capped_completion_is_accepted_and_honoured() {
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for the live test");

    let provider = completions_provider("openai-completions", api_key);
    let mut config = LlmCallConfig::new("gpt-6-luna");
    config.max_tokens = Some(16);

    let response = provider
        .chat_completion_non_streaming(
            vec![Message::text(
                MessageRole::User,
                "Write 300 words about the sea.",
            )],
            &config,
        )
        .await
        .expect("a capped Chat Completions request should be accepted");

    eprintln!(
        "finish_reason={:?} completion_tokens={:?}",
        response.metadata.finish_reason, response.metadata.completion_tokens
    );

    // The cap is the point: an accepted-but-ignored field would run to a
    // natural stop well past 16 tokens.
    assert_eq!(
        response.metadata.finish_reason.as_deref(),
        Some("length"),
        "a 16-token cap on a 300-word prompt should stop on length"
    );
}
