//! Live smoke test for OpenRouter model discovery.
//!
//! Exercises the real `OpenRouterChatDriver::list_models` path against
//! OpenRouter's `/models` endpoint: HTTP fetch, deserialization, and capability
//! profiling.
//!
//! Ignored by default (requires network + `OPENROUTER_API_KEY`); run manually:
//!   `doppler run -- cargo test -p everruns-openrouter --test openrouter_discovery_live -- --ignored --nocapture`

use everruns_openrouter::provider;

#[tokio::test]
#[ignore = "live network + OPENROUTER_API_KEY"]
async fn openrouter_discovers_nemotron_reasoning_profile() {
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .expect("OPENROUTER_API_KEY must be set for the live smoke test");

    let provider = provider("openrouter", api_key);

    let models = provider
        .list_models()
        .await
        .expect("list_models should succeed")
        .expect("OpenRouter should support model listing");

    assert!(!models.is_empty(), "expected a non-empty model catalog");

    // The target model must be discovered with a reasoning-capable profile so
    // the UI exposes the effort selector.
    let nemotron = models
        .iter()
        .find(|m| m.model_id == "nvidia/nemotron-3-super-120b-a12b")
        .expect("nemotron-3-super should be in the OpenRouter catalog");

    let profile = nemotron
        .discovered_profile
        .as_ref()
        .expect("discovered models should carry a capability profile");

    assert!(
        profile.reasoning,
        "Nemotron must be flagged reasoning-capable"
    );
    let effort = profile
        .reasoning_effort
        .as_ref()
        .expect("reasoning models expose an effort config");
    assert!(
        !effort.values.is_empty(),
        "effort config should list selectable levels"
    );

    // At least some of the broader catalog should also be reasoning-capable —
    // guards against a parsing regression that silently drops the field.
    let reasoning_count = models
        .iter()
        .filter(|m| m.discovered_profile.as_ref().is_some_and(|p| p.reasoning))
        .count();
    assert!(
        reasoning_count > 10,
        "expected many reasoning models, got {reasoning_count}"
    );

    eprintln!(
        "OpenRouter discovery: {} models, {} reasoning-capable",
        models.len(),
        reasoning_count
    );
}
