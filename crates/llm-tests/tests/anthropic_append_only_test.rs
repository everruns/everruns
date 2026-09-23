#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live check that the Anthropic harness is append-only (EVE-1084).
//!
//! A mid-conversation operator notice (the `infinity_context` hidden-history
//! count, a loop-detection warning) used to be folded into the top-level
//! `system` field. `system` renders ahead of the whole transcript, so a notice
//! that changes between turns re-processes every earlier turn uncached and, on
//! the models with preserved thinking, invalidates the history's thinking
//! blocks. On models that take the role mid-conversation the notice is now sent
//! as a `{"role": "system"}` entry in `messages` instead.
//!
//! The evidence is Anthropic's own accounting: with the notice inline, a second
//! request whose notice text changed still reads the cached prefix
//! (`cache_read_input_tokens > 0`).
//!
//! Run:
//!   cargo test -p everruns-llm-tests --test anthropic_append_only_test --features llm-tests -- --nocapture
//!
//! Required env vars (the test skips when absent):
//!   ANTHROPIC_API_KEY
#![cfg(feature = "llm-tests")]

use everruns_provider::driver_registry::{LlmCallConfig, LlmStreamEvent, PromptCacheConfig};
use everruns_provider::{Message, MessageRole, ProviderEndpoint};
use futures::StreamExt;

/// Long enough to clear Anthropic's minimum cacheable prefix (512 tokens on
/// this family) with room to spare.
///
/// The leading per-run marker keeps each run independent: without it a later
/// run reads the entry an earlier one wrote and the first turn never writes.
fn stable_system_prompt() -> String {
    format!(
        "Session {}. {}",
        uuid::Uuid::new_v4(),
        "You are a meticulous engineering assistant working in a large Rust monorepo. \
         Answer with a single short sentence and never use tools. "
            .repeat(120)
    )
}

fn notice(hidden: usize) -> Message {
    Message::text(
        MessageRole::System,
        format!("[{hidden} earlier messages are not in this context.]"),
    )
}

/// Drive one request and return `(cache_read_tokens, cache_creation_tokens)`.
async fn run_turn(
    driver: &dyn everruns_provider::ChatDriver,
    messages: Vec<Message>,
) -> (u32, u32) {
    let mut config = LlmCallConfig::new("claude-opus-5-5");
    config.max_tokens = Some(64);
    config.prompt_cache = Some(PromptCacheConfig {
        enabled: true,
        ..PromptCacheConfig::default()
    });
    let mut stream = driver
        .chat_completion_stream(&ProviderEndpoint::default(), messages, &config)
        .await
        .expect("request should succeed");
    while let Some(event) = stream.next().await {
        match event.expect("stream item should be readable") {
            LlmStreamEvent::Done(metadata) => {
                return (
                    metadata.cache_read_tokens.unwrap_or(0),
                    metadata.cache_creation_tokens.unwrap_or(0),
                );
            }
            LlmStreamEvent::Error(error) => panic!("stream error: {error:?}"),
            _ => {}
        }
    }
    panic!("stream ended without a Done event");
}

#[tokio::test]
async fn mid_conversation_notice_keeps_the_cached_prefix_on_opus_5_5() {
    let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("skipping: ANTHROPIC_API_KEY is not set");
        return;
    };
    let driver = everruns_anthropic::provider("anthropic", api_key).into_boxed_driver();

    let system_prompt = stable_system_prompt();
    let conversation = |hidden: usize| {
        vec![
            Message::text(MessageRole::System, system_prompt.clone()),
            Message::text(MessageRole::User, "Name one Rust crate."),
            notice(hidden),
            Message::text(MessageRole::Assistant, "serde."),
            Message::text(MessageRole::User, "Name another one."),
        ]
    };

    // Turn one writes the cache.
    let (read_first, created_first) = run_turn(driver.as_ref(), conversation(2)).await;
    println!("turn 1: cache_read={read_first} cache_creation={created_first}");
    assert!(
        created_first > 0,
        "the first turn should write a cache entry, wrote {created_first}"
    );

    // Turn two changes the notice — exactly the edit that used to rewrite the
    // top-level `system` field — and must still read the prefix turn one wrote.
    let (read, created) = run_turn(driver.as_ref(), conversation(7)).await;
    println!("turn 2: cache_read={read} cache_creation={created}");
    assert!(
        read > 0,
        "a changed mid-conversation notice must not evict the cached prefix; \
         cache_read_input_tokens was {read}"
    );
}
