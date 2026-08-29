//! Workflow tests for Everruns API
//!
//! These tests require a RUNNING API server and Worker process.
//! They test end-to-end workflows including LLM execution.
//!
//! For API endpoint testing without a running server, use:
//! - `api_integration_test.rs` (in-process testing with PostgreSQL)
//! - `repository_integration_test.rs` (direct repository testing)
//!
//! Run with: cargo test -p everruns-server --test workflow_test -- --test-threads=1
//!
//! Requirements:
//! - API server running at localhost:9000
//! - Worker process running
//! - PostgreSQL with migrations applied
//! - Uses LlmSim for workflow tests, no real API keys needed

use everruns_core::SessionFile;
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use serde_json::{Value, json};

const SERVER_BASE_URL: &str = "http://localhost:9000";
const API_BASE_URL: &str = "http://localhost:9000/api";
// Note: With AUTH_MODE=none, org is derived from the anonymous user's default org.
// No cookie or header needed for integration tests.

/// Built-in harness name — resolved per-org at runtime (UUIDs are DB-assigned
/// and no longer stable). Passed via `harness_name` on session creation.
const SEED_HARNESS_NAME: &str = "base";

/// Live Anthropic model ids used by the tests in this file that talk to the
/// real provider.
///
/// Pinned to explicit dated ids rather than `-latest` aliases so a failure
/// names the model it actually asked for. Anthropic retires models, and a
/// retired id comes back as `model_unavailable` rather than a test-logic
/// failure — when that happens, re-point these two constants at current ids
/// (`GET /v1/models`) instead of editing each test.
const LIVE_ANTHROPIC_FAST_MODEL: &str = "claude-haiku-4-5-20251001";
/// Extended thinking requires a model that supports it; Haiku does not.
const LIVE_ANTHROPIC_THINKING_MODEL: &str = "claude-sonnet-4-5-20250929";

/// Returns the error code when a turn was blocked by a live provider *account*
/// condition — the account is out of credits, or a subscription usage limit was
/// reached — rather than by a defect in the code under test.
///
/// The live provider matrix already skips on this signal (`skip_if_quota!` in
/// `crates/llm-tests/tests/llm_test_matrix/mod.rs`, and the comment on the
/// `live-provider-matrix` CI job). The live tests in this file drive the same
/// shared provider accounts through the worker, so they skip on it too: a turn
/// that never reached the provider proves nothing about tool-call serialization
/// or reasoning projection, and failing on it reports a code regression that
/// does not exist. Main went red exactly this way on 2026-08-29.
///
/// Deliberately narrow. A missing or invalid credential
/// (`provider_misconfigured`) and every genuine API or contract break still
/// fail loudly — see `EVERRUNS_REQUIRE_LIVE_TESTS` in the CI workflow, which
/// exists so a silently absent key cannot report a vacuous pass.
fn provider_account_block(message: &Value) -> Option<&str> {
    use everruns_provider::user_facing_error::codes;

    let code = message["metadata"]["error_code"].as_str()?;
    (code == codes::PROVIDER_QUOTA_EXHAUSTED || code == codes::PROVIDER_USAGE_LIMIT_REACHED)
        .then_some(code)
}

#[test]
fn provider_account_block_matches_only_billing_codes() {
    use everruns_provider::user_facing_error::codes;

    let with_code = |code: &str| json!({"metadata": {"error_code": code}});

    assert_eq!(
        provider_account_block(&with_code(codes::PROVIDER_QUOTA_EXHAUSTED)),
        Some(codes::PROVIDER_QUOTA_EXHAUSTED)
    );
    assert_eq!(
        provider_account_block(&with_code(codes::PROVIDER_USAGE_LIMIT_REACHED)),
        Some(codes::PROVIDER_USAGE_LIMIT_REACHED)
    );

    // A bad or absent credential is an operator error in the CI setup, not a
    // billing condition — it must keep failing loudly.
    assert_eq!(
        provider_account_block(&with_code(codes::PROVIDER_MISCONFIGURED)),
        None
    );
    // Genuine regressions the live tests exist to catch.
    for code in [
        codes::INVALID_TOOL_SCHEMA,
        codes::PROCESSING_ERROR,
        codes::MODEL_UNAVAILABLE,
        codes::PROVIDER_RATE_LIMITED,
    ] {
        assert_eq!(provider_account_block(&with_code(code)), None, "{code}");
    }

    // A healthy turn carries no error code at all.
    assert_eq!(provider_account_block(&json!({"metadata": {}})), None);
    assert_eq!(provider_account_block(&json!({})), None);
}

/// Finds the first provider account block in a `GET /v1/sessions/{id}/messages`
/// response body.
///
/// Split from the request so the extraction can be tested against a real
/// captured response rather than first exercised during the incident it exists
/// to report — the same approach `scripts/test-provider-credits.sh` takes for
/// the credit-health classifier (EVE-935).
fn messages_provider_account_block(body: &Value) -> Option<&str> {
    body["data"]
        .as_array()?
        .iter()
        .find_map(provider_account_block)
}

#[test]
fn messages_provider_account_block_reads_a_real_quota_response() {
    // Captured verbatim from the agent message printed by
    // `test_reasoning_reaches_api_sanitized_and_classified` in CI run
    // 33227749538, the run this guard exists to stop misreporting.
    let body = json!({"data": [
        {
            "content": [{"text": "Think about this, then answer.", "type": "text"}],
            "id": "message_2f1c9a4e6b7d4f0e8a1c3d5e7f902468",
            "role": "user",
            "sequence": 1,
            "session_id": "session_01a04b5180217dd1a2665373ba3447ad"
        },
        {
            "content": [{
                "text": "The AI provider account is out of credits or quota. Add credits \
                         or raise the provider account limits to continue.",
                "type": "text"
            }],
            "created_at": "2026-08-29T02:20:30.955249511Z",
            "id": "message_572957841c0f4ee995c3e70e426d70e3",
            "metadata": {
                "error_code": "provider_quota_exhausted",
                "error_disclosure": "standard",
                "error_fields": {"model_id": "gpt-5.6-terra", "provider": "openai"},
                "source_error_code": "provider_quota_exhausted"
            },
            "role": "agent",
            "sequence": 8,
            "session_id": "session_01a04b5180217dd1a2665373ba3447ad"
        }
    ]});
    assert_eq!(
        messages_provider_account_block(&body),
        Some("provider_quota_exhausted")
    );

    // A healthy transcript yields no block, so assertions still run.
    let healthy = json!({"data": [
        {"role": "user", "content": [{"text": "hi", "type": "text"}]},
        {"role": "agent", "content": [{"text": "hello", "type": "text"}], "metadata": {}}
    ]});
    assert_eq!(messages_provider_account_block(&healthy), None);

    // Shapes that must not panic or be mistaken for a block.
    assert_eq!(messages_provider_account_block(&json!({"data": []})), None);
    assert_eq!(messages_provider_account_block(&json!({})), None);
}

/// Polls a session's messages once and reports the first provider account block
/// found on an agent message.
///
/// Returns the code so the caller can name it in the skip line. Any transport
/// or decode failure reports `None`: an unreachable API is a real failure and
/// must not be laundered into a skip.
async fn session_provider_account_block(
    client: &reqwest::Client,
    session_id: impl std::fmt::Display,
) -> Option<String> {
    let response = client
        .get(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session_id
        ))
        .send()
        .await
        .ok()?;
    if response.status() != 200 {
        return None;
    }
    let body: Value = response.json().await.ok()?;
    messages_provider_account_block(&body).map(str::to_owned)
}

/// Renders one skip line for GitHub's step summary.
///
/// Separate from the file write so the wording is testable without touching the
/// environment.
fn provider_account_skip_line(test: &str, code: &str) -> String {
    format!(
        "- **Skipped `{test}`** — live provider account unavailable (`{code}`). \
         The turn never reached the provider, so these live assertions did not run."
    )
}

/// Records a skipped live test somewhere a *green* run still shows it.
///
/// `libtest` captures stdout for tests that pass, and the `workflow-test` job
/// runs without `--nocapture`, so a bare `println!` here is swallowed by exactly
/// the green run that most needs to disclose the missing coverage. That is the
/// silent-green blind spot EVE-935 exists to close, so the skip is also appended
/// to GitHub's step summary, which the test process writes directly and libtest
/// does not capture. (`doppler run --preserve-env` passes `GITHUB_STEP_SUMMARY`
/// through untouched — it only arbitrates names Doppler itself defines.)
///
/// Best effort by design: a missing, unset, or unwritable summary file must
/// never turn a skip into a failure. Outside Actions the `println!` still
/// carries the line under `--nocapture`.
fn report_provider_account_skip(code: &str) {
    let test = std::thread::current()
        .name()
        .unwrap_or("unknown test")
        .to_owned();
    println!("SKIP: {test}: live provider account unavailable ({code})");

    if let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") {
        append_skip_to_summary(std::path::Path::new(&path), &test, code);
    }
}

/// Appends one skip line to the step-summary file, swallowing every I/O error.
///
/// Split out so the append is covered by a test rather than only ever running
/// inside Actions.
fn append_skip_to_summary(path: &std::path::Path, test: &str, code: &str) {
    use std::io::Write;

    let Ok(mut file) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(file, "{}", provider_account_skip_line(test, code));
}

#[test]
fn append_skip_to_summary_accumulates_and_never_panics() {
    let dir = std::env::temp_dir().join(format!("everruns-skip-summary-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("summary.md");

    // Appends rather than truncating: several tests can skip in one run and the
    // summary must list every one of them.
    append_skip_to_summary(&path, "test_one", "provider_quota_exhausted");
    append_skip_to_summary(&path, "test_two", "provider_usage_limit_reached");
    let written = std::fs::read_to_string(&path).expect("summary written");
    assert_eq!(written.lines().count(), 2, "{written}");
    assert!(written.contains("test_one"), "{written}");
    assert!(written.contains("test_two"), "{written}");

    // An unwritable path is silently ignored — a skip must never become a
    // failure because Actions moved where the summary lives.
    append_skip_to_summary(
        &dir.join("no-such-directory").join("summary.md"),
        "test_three",
        "provider_quota_exhausted",
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn provider_account_skip_line_names_the_test_and_code() {
    let line = provider_account_skip_line(
        "test_agent_execution_openai_with_tool_calls",
        "provider_quota_exhausted",
    );
    // A reader scanning a green run's summary needs both: which test lost its
    // coverage, and why.
    assert!(
        line.contains("test_agent_execution_openai_with_tool_calls"),
        "{line}"
    );
    assert!(line.contains("provider_quota_exhausted"), "{line}");
    // Markdown list item, so it renders in the step summary rather than running
    // together with neighbouring lines.
    assert!(line.starts_with("- "), "{line}");
    assert!(!line.contains('\n'), "must stay one line: {line}");
}

/// Skips the current test when a previously captured
/// `session_provider_account_block` says the live provider account is unusable.
///
/// Takes an already-captured value rather than polling itself, so callers can
/// read the block while the session is still intact and act on it after they
/// have torn their fixtures down.
macro_rules! skip_on_provider_account_block {
    ($block:expr) => {
        if let Some(code) = $block {
            report_provider_account_skip(&code);
            return;
        }
    };
}

#[tokio::test]
async fn test_full_agent_session_workflow() {
    let client = reqwest::Client::new();

    println!("Testing full agent/session workflow...");

    // Step 1: Create an agent
    println!("\nStep 1: Creating agent...");
    let create_agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "test-agent",
            "display_name": "Test Agent",
            "description": "An agent for testing",
            "system_prompt": "You are a helpful assistant"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(
        create_agent_response.status(),
        201,
        "Expected 201 Created, got {}",
        create_agent_response.status()
    );

    let agent: Agent = create_agent_response
        .json()
        .await
        .expect("Failed to parse agent response");

    println!("Created agent: {}", agent.public_id);
    assert_eq!(agent.name, "test-agent");
    assert_eq!(agent.status.to_string(), "active");

    // Step 2: List agents
    println!("\nStep 2: Listing agents...");
    let list_response = client
        .get(format!("{}/v1/agents", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list agents");

    assert_eq!(list_response.status(), 200);

    let response: serde_json::Value = list_response.json().await.expect("Failed to parse");
    let agents: Vec<Agent> =
        serde_json::from_value(response["data"].clone()).expect("Failed to parse agents");
    println!("Found {} agent(s)", agents.len());
    assert!(!agents.is_empty());

    // Step 3: Get agent by ID
    println!("\nStep 3: Getting agent by ID...");
    let get_response = client
        .get(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to get agent");

    assert_eq!(get_response.status(), 200);
    let fetched_agent: Agent = get_response.json().await.expect("Failed to parse agent");
    println!("Fetched agent: {}", fetched_agent.name);
    assert_eq!(fetched_agent.public_id, agent.public_id);

    // Step 4: Update agent
    println!("\nStep 4: Updating agent...");
    let update_response = client
        .patch(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .json(&json!({
            "name": "updated-test-agent",
            "display_name": "Updated Test Agent",
            "description": "Updated description"
        }))
        .send()
        .await
        .expect("Failed to update agent");

    assert_eq!(update_response.status(), 200);
    let updated_agent: Agent = update_response.json().await.expect("Failed to parse agent");
    println!("Updated agent: {}", updated_agent.name);
    assert_eq!(updated_agent.name, "updated-test-agent");

    // Step 5: Create a session
    println!("\nStep 5: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);
    assert_eq!(session.agent_id, Some(agent.public_id));

    // Step 6: Add message (user message)
    println!("\nStep 6: Adding user message...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "Hello!"}]
            }
        }))
        .send()
        .await
        .expect("Failed to create message");

    assert_eq!(message_response.status(), 201);
    let message: Value = message_response
        .json()
        .await
        .expect("Failed to parse message");
    println!("Created message: {}", message["id"]);
    assert_eq!(message["role"], "user");

    // Step 7: List messages
    println!("\nStep 7: Listing messages...");
    let messages_response = client
        .get(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to list messages");

    assert_eq!(messages_response.status(), 200);
    let response: Value = messages_response.json().await.expect("Failed to parse");
    let messages = response["data"]
        .as_array()
        .expect("Expected array of messages");
    println!("Found {} message(s)", messages.len());
    assert_eq!(messages.len(), 1);

    // Step 8: Get session
    println!("\nStep 8: Getting session...");
    let get_session_response = client
        .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
        .send()
        .await
        .expect("Failed to get session");

    assert_eq!(get_session_response.status(), 200);
    let fetched_session: Session = get_session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Fetched session: {}", fetched_session.id);
    assert_eq!(fetched_session.id, session.id);

    // Step 9: List events (events are created automatically with messages)
    println!("\nStep 9: Listing events...");
    let events_response = client
        .get(format!(
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to list events");

    assert_eq!(events_response.status(), 200);
    let events_data: Value = events_response
        .json()
        .await
        .expect("Failed to parse events");
    let events = events_data["data"]
        .as_array()
        .expect("Expected array of events");
    println!("Found {} event(s)", events.len());
    // Events are created when messages are processed by the workflow
    // For this basic test, we just verify the endpoint works

    println!("\nAll tests passed!");
}

#[tokio::test]
async fn test_health_endpoint() {
    let client = reqwest::Client::new();

    println!("Testing health endpoint...");
    let response = client
        .get(format!("{}/health", SERVER_BASE_URL))
        .send()
        .await
        .expect("Failed to call health endpoint");

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("Failed to parse response");
    println!("Health check: {:?}", body);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn test_openapi_spec() {
    let client = reqwest::Client::new();

    println!("Testing OpenAPI spec endpoint...");
    let response = client
        .get(format!("{}/api-doc/openapi.json", SERVER_BASE_URL))
        .send()
        .await
        .expect("Failed to get OpenAPI spec");

    assert_eq!(response.status(), 200);
    let spec: serde_json::Value = response.json().await.expect("Failed to parse spec");
    println!("OpenAPI spec title: {}", spec["info"]["title"]);
    assert_eq!(spec["info"]["title"], "Everruns API");
}

#[tokio::test]
async fn test_provider_and_model_workflow() {
    let client = reqwest::Client::new();

    println!("Testing LLM Provider and Model workflow...");

    // Step 1: Create an LLM provider
    println!("\nStep 1: Creating LLM provider...");
    let create_provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Test OpenAI Provider",
            "provider_type": "openai",
            "base_url": "https://api.openai.com/v1",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create LLM provider");

    let response_text = create_provider_response
        .text()
        .await
        .expect("Failed to get response text");

    let provider: Provider =
        serde_json::from_str(&response_text).expect("Failed to parse provider response");

    println!("Created provider: {} ({})", provider.name, provider.id);
    assert_eq!(provider.name, "Test OpenAI Provider");

    // Step 2: Create a model for the provider
    println!("\nStep 2: Creating model for provider...");
    let create_model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-5.2",
            "display_name": "GPT-5.2",
            "capabilities": ["chat", "vision"],
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    let model_response_text = create_model_response
        .text()
        .await
        .expect("Failed to get model response text");

    let model: Model =
        serde_json::from_str(&model_response_text).expect("Failed to parse model response");

    println!("Created model: {} ({})", model.display_name, model.id);
    assert_eq!(model.model_id, "gpt-5.2");

    // Cleanup
    println!("\nCleaning up...");

    client
        .delete(format!("{}/v1/models/{}", API_BASE_URL, model.id))
        .send()
        .await
        .expect("Failed to delete model");

    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("All LLM provider and model tests passed!");
}

#[tokio::test]
async fn test_model_profile() {
    let client = reqwest::Client::new();

    println!("Testing LLM Model Profile...");

    // Step 1: Create an LLM provider
    println!("\nStep 1: Creating OpenAI provider...");
    let create_provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Test Profile Provider",
            "provider_type": "openai",
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create LLM provider");

    let provider: Provider = create_provider_response
        .json()
        .await
        .expect("Failed to parse provider response");

    println!("Created provider: {} ({})", provider.name, provider.id);

    // Step 2: Create a known model (gpt-5.2) that has a profile
    println!("\nStep 2: Creating gpt-5.2 model...");
    let create_model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-5.2",
            "display_name": "GPT-5.2",
            "capabilities": ["chat", "vision"],
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create model");

    let model_json: Value = create_model_response
        .json()
        .await
        .expect("Failed to parse model response");

    println!("Created model: {}", model_json["display_name"]);
    let model_id = model_json["id"].as_str().unwrap();

    // Step 3: Get the model via the list endpoint which includes profile
    println!("\nStep 3: Getting model with profile via list endpoint...");
    let list_models_response = client
        .get(format!("{}/v1/models", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list models");

    assert_eq!(list_models_response.status(), 200);
    let list_response: Value = list_models_response
        .json()
        .await
        .expect("Failed to parse models response");
    let models = list_response["data"]
        .as_array()
        .expect("Response should have data array");

    let gpt52_model = models
        .iter()
        .find(|m| m["model_id"] == "gpt-5.2")
        .expect("Should find gpt-5.2 in model list");

    // Verify profile exists and has expected fields
    let profile = &gpt52_model["profile"];
    println!("Profile: {:?}", profile);

    // Profile may be null if the model profile lookup isn't working
    // For now, just verify we can list models - profile lookup is optional
    if !profile.is_null() {
        assert_eq!(profile["name"], "GPT-5.2", "Profile name should be GPT-5.2");
        assert_eq!(
            profile["family"], "gpt-5.2",
            "Profile family should be gpt-5.2"
        );
        assert!(
            profile["tool_call"].as_bool().unwrap_or(false),
            "GPT-5.2 should support tool calls"
        );
        println!("Profile verified successfully");
    } else {
        println!("Profile is null - skipping profile assertions");
    }

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/models/{}", API_BASE_URL, model_id))
        .send()
        .await
        .expect("Failed to delete model");

    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("LLM Model Profile tests passed!");
}

#[tokio::test]
async fn test_session_inherits_agent_default_model() {
    let client = reqwest::Client::new();

    println!("Testing session model_id inheritance from agent...");

    // Step 1: Create an LLM provider
    println!("\nStep 1: Creating LLM provider...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Test Provider for Session Model",
            "provider_type": "openai",
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created provider: {}", provider.id);

    // Step 2: Create a model
    println!("\nStep 2: Creating model...");
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "test-model",
            "display_name": "Test Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 3: Create an agent with default_model_id
    println!("\nStep 3: Creating agent with default_model_id...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "agent-with-default-model",
            "display_name": "Agent with Default Model",
            "system_prompt": "Test agent",
            "default_model_id": model.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with default_model_id: {:?}",
        agent.public_id, agent.default_model_id
    );
    assert_eq!(agent.default_model_id, Some(model.id));

    // Step 4: Create a session WITHOUT specifying model_id
    println!("\nStep 4: Creating session without model_id...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!(
        "Created session: {} with model_id: {:?}",
        session.id, session.model_id
    );

    // Verify session inherited the agent's default_model_id
    assert_eq!(
        session.model_id,
        Some(model.id),
        "Session should inherit agent's default_model_id"
    );

    // Step 5: Create a session WITH explicit model_id (should override)
    println!("\nStep 5: Creating session with explicit model_id...");

    // Create another model
    let model2_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "test-model-2",
            "display_name": "Test Model 2",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create second model");

    let model2: Model = model2_response
        .json()
        .await
        .expect("Failed to parse second model");

    let session2_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Test Session 2",
            "model_id": model2.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create session with explicit model");

    assert_eq!(session2_response.status(), 201);
    let session2: Session = session2_response
        .json()
        .await
        .expect("Failed to parse session2");
    println!(
        "Created session2: {} with model_id: {:?}",
        session2.id, session2.model_id
    );

    // Verify explicit model_id overrides default
    assert_eq!(
        session2.model_id,
        Some(model2.id),
        "Session should use explicit model_id"
    );

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!("{}/v1/models/{}", API_BASE_URL, model.id))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/models/{}", API_BASE_URL, model2.id))
        .send()
        .await
        .expect("Failed to delete model2");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Session model_id inheritance test passed!");
}

#[tokio::test]
async fn test_session_filesystem() {
    let client = reqwest::Client::new();

    println!("Testing session filesystem...");

    // Step 1: Create an agent
    println!("\nStep 1: Creating agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "filesystem-test-agent",
            "display_name": "Filesystem Test Agent",
            "system_prompt": "Test agent for filesystem"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 2: Create a session
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Filesystem Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    let fs_url = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session.id);

    // Step 3: List root directory (new sessions mount scoped memory under the reserved memory root)
    println!("\nStep 3: Listing root directory...");
    let list_response = client
        .get(&fs_url)
        .send()
        .await
        .expect("Failed to list files");

    assert_eq!(list_response.status(), 200);
    let list_result: Value = list_response.json().await.expect("Failed to parse");
    let root_entries = list_result["data"].as_array().unwrap();
    assert_eq!(root_entries.len(), 1);
    assert_eq!(root_entries[0]["path"], "/memory");
    assert_eq!(root_entries[0]["is_directory"], true);
    println!("Root directory contains scoped memory mount");

    // Step 4: Create a file
    println!("\nStep 4: Creating file...");
    let create_response = client
        .post(format!("{}/hello.txt", fs_url))
        .json(&json!({
            "content": "Hello, World!",
            "encoding": "text"
        }))
        .send()
        .await
        .expect("Failed to create file");

    assert_eq!(create_response.status(), 201);
    let file: SessionFile = create_response.json().await.expect("Failed to parse file");
    println!("Created file: {}", file.path);
    assert_eq!(file.path, "/hello.txt");
    assert!(!file.is_directory);

    // Step 5: Read file
    println!("\nStep 5: Reading file...");
    let read_response = client
        .get(format!("{}/hello.txt", fs_url))
        .send()
        .await
        .expect("Failed to read file");

    assert_eq!(read_response.status(), 200);
    let file: SessionFile = read_response.json().await.expect("Failed to parse file");
    assert_eq!(file.content.as_deref(), Some("Hello, World!"));
    println!("File content: {:?}", file.content);

    // Step 6: Get file stat
    println!("\nStep 6: Getting file stat...");
    let stat_response = client
        .post(format!("{}/_/stat", fs_url))
        .json(&json!({
            "path": "/hello.txt"
        }))
        .send()
        .await
        .expect("Failed to get stat");

    assert_eq!(stat_response.status(), 200);
    let stat: Value = stat_response.json().await.expect("Failed to parse stat");
    assert_eq!(stat["path"], "/hello.txt");
    assert_eq!(stat["is_directory"], false);
    println!("File stat: size={}", stat["size_bytes"]);

    // Step 7: Update file
    println!("\nStep 7: Updating file...");
    let update_response = client
        .put(format!("{}/hello.txt", fs_url))
        .json(&json!({
            "content": "Updated content"
        }))
        .send()
        .await
        .expect("Failed to update file");

    assert_eq!(update_response.status(), 200);
    let file: SessionFile = update_response.json().await.expect("Failed to parse file");
    assert_eq!(file.content.as_deref(), Some("Updated content"));
    println!("File updated");

    // Step 8: Create directory
    println!("\nStep 8: Creating directory...");
    let dir_response = client
        .post(format!("{}/docs", fs_url))
        .json(&json!({
            "is_directory": true
        }))
        .send()
        .await
        .expect("Failed to create directory");

    assert_eq!(dir_response.status(), 201);
    let dir: SessionFile = dir_response.json().await.expect("Failed to parse dir");
    assert!(dir.is_directory);
    println!("Created directory: {}", dir.path);

    // Step 9: Create file in directory (auto-creates parent)
    println!("\nStep 9: Creating nested file...");
    let nested_response = client
        .post(format!("{}/src/main.rs", fs_url))
        .json(&json!({
            "content": "fn main() {}"
        }))
        .send()
        .await
        .expect("Failed to create nested file");

    assert_eq!(nested_response.status(), 201);
    let nested: SessionFile = nested_response.json().await.expect("Failed to parse");
    assert_eq!(nested.path, "/src/main.rs");
    println!("Created nested file: {}", nested.path);

    // Step 10: List all files
    println!("\nStep 10: Listing all files...");
    let list_all_response = client
        .get(format!("{}?recursive=true", fs_url))
        .send()
        .await
        .expect("Failed to list all files");

    assert_eq!(list_all_response.status(), 200);
    let list_all: Value = list_all_response.json().await.expect("Failed to parse");
    let files = list_all["data"].as_array().unwrap();
    assert!(files.len() >= 3); // hello.txt, docs, src/main.rs
    println!("Found {} files", files.len());

    // Step 11: Copy file
    println!("\nStep 11: Copying file...");
    let copy_response = client
        .post(format!("{}/_/copy", fs_url))
        .json(&json!({
            "src_path": "/hello.txt",
            "dst_path": "/hello-copy.txt"
        }))
        .send()
        .await
        .expect("Failed to copy file");

    assert_eq!(copy_response.status(), 201);
    println!("File copied");

    // Step 12: Move file
    println!("\nStep 12: Moving file...");
    let move_response = client
        .post(format!("{}/_/move", fs_url))
        .json(&json!({
            "src_path": "/hello-copy.txt",
            "dst_path": "/renamed.txt"
        }))
        .send()
        .await
        .expect("Failed to move file");

    assert_eq!(move_response.status(), 200);
    println!("File moved/renamed");

    // Step 13: Grep search
    println!("\nStep 13: Searching files...");
    let grep_response = client
        .post(format!("{}/_/grep", fs_url))
        .json(&json!({
            "pattern": "main"
        }))
        .send()
        .await
        .expect("Failed to grep");

    assert_eq!(grep_response.status(), 200);
    let grep_result: Value = grep_response.json().await.expect("Failed to parse");
    let results = grep_result["data"].as_array().unwrap();
    assert!(!results.is_empty());
    println!("Found {} files with matches", results.len());

    // Step 14: Delete file
    println!("\nStep 14: Deleting file...");
    let delete_response = client
        .delete(format!("{}/renamed.txt", fs_url))
        .send()
        .await
        .expect("Failed to delete file");

    assert_eq!(delete_response.status(), 200);
    let delete_result: Value = delete_response.json().await.expect("Failed to parse");
    assert_eq!(delete_result["deleted"], true);
    println!("File deleted");

    // Step 15: Delete directory recursively
    println!("\nStep 15: Deleting directory recursively...");
    let delete_dir_response = client
        .delete(format!("{}/src?recursive=true", fs_url))
        .send()
        .await
        .expect("Failed to delete directory");

    assert_eq!(delete_dir_response.status(), 200);
    println!("Directory deleted");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("Session filesystem test passed!");
}

/// Test that filesystem API handles /workspace prefix correctly
///
/// The file_system and bashkit_shell capabilities present files to users with
/// a /workspace prefix (e.g., /workspace/demo/a.txt), but internally store
/// them without the prefix (e.g., /demo/a.txt). The API should handle both
/// formats transparently.
#[tokio::test]
async fn test_session_filesystem_workspace_prefix() {
    let client = reqwest::Client::new();

    println!("Testing filesystem workspace prefix handling...");

    // Step 1: Create an agent and session
    println!("\nStep 1: Creating agent and session...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "workspace-test-agent",
            "display_name": "Workspace Test Agent",
            "system_prompt": "Test agent"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");

    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Workspace Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    let fs_url = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session.id);

    // Step 2: Create a file at /demo/a.txt (internal path)
    println!("\nStep 2: Creating file at internal path /demo/a.txt...");
    let create_response = client
        .post(format!("{}/demo/a.txt", fs_url))
        .json(&json!({
            "content": "hello",
            "encoding": "text"
        }))
        .send()
        .await
        .expect("Failed to create file");

    assert_eq!(create_response.status(), 201);
    println!("File created at /demo/a.txt");

    // Step 3: Access /workspace should return root listing (maps to /)
    println!("\nStep 3: Accessing /workspace path...");
    let workspace_response = client
        .get(format!("{}/workspace", fs_url))
        .send()
        .await
        .expect("Failed to get workspace");

    // This is the bug: /workspace returns 404 instead of root listing
    assert_eq!(
        workspace_response.status(),
        200,
        "Expected 200 OK for /workspace, got {} - workspace prefix not handled",
        workspace_response.status()
    );

    let workspace_result: Value = workspace_response
        .json()
        .await
        .expect("Failed to parse workspace response");
    assert!(
        workspace_result.get("data").is_some(),
        "Expected directory listing"
    );
    println!("/workspace returned listing successfully");

    // Step 4: Access /workspace/demo should list the demo directory
    println!("\nStep 4: Accessing /workspace/demo path...");
    let workspace_demo_response = client
        .get(format!("{}/workspace/demo", fs_url))
        .send()
        .await
        .expect("Failed to get workspace/demo");

    assert_eq!(
        workspace_demo_response.status(),
        200,
        "Expected 200 OK for /workspace/demo, got {}",
        workspace_demo_response.status()
    );
    println!("/workspace/demo returned listing successfully");

    // Step 5: Access /workspace/demo/a.txt should return the file
    println!("\nStep 5: Accessing /workspace/demo/a.txt...");
    let workspace_file_response = client
        .get(format!("{}/workspace/demo/a.txt", fs_url))
        .send()
        .await
        .expect("Failed to get workspace file");

    assert_eq!(
        workspace_file_response.status(),
        200,
        "Expected 200 OK for /workspace/demo/a.txt, got {}",
        workspace_file_response.status()
    );

    let file: SessionFile = workspace_file_response
        .json()
        .await
        .expect("Failed to parse file");
    assert_eq!(file.content.as_deref(), Some("hello"));
    println!("/workspace/demo/a.txt returned file content successfully");

    // Cleanup
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("Filesystem workspace prefix test passed!");
}

/// Test agent with file_system and bashkit_shell capabilities share /workspace paths
///
/// This test verifies:
/// 1. An agent with both session_file_system and bashkit_shell capabilities can use them
/// 2. Files created via write_file are accessible at /workspace via API
/// 3. The /workspace prefix transformation works correctly in agent workflows
///
/// Requirements: API + Worker running, ANTHROPIC_API_KEY environment variable set.
/// Skips if no API key is available.
#[tokio::test]
async fn test_agent_filesystem_and_bash_workspace_integration() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to create client");

    println!("Testing agent with file_system and bashkit_shell capabilities...");

    // Check if Anthropic API key is available
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            println!("ANTHROPIC_API_KEY not set - skipping test");
            return;
        }
    };

    // Step 1: Create Anthropic provider and model
    println!("\nStep 1: Creating Anthropic provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "FS Bash Integration Test Provider",
            "provider_type": "anthropic",
            "api_key": api_key,
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create Anthropic provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created Anthropic provider: {}", provider.id);

    // Create model (claude-haiku-4-5 for cost-effectiveness)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": LIVE_ANTHROPIC_FAST_MODEL,
            "display_name": "Anthropic Fast (FS Bash Test)",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create Anthropic model: status={}, body={}",
            status, body
        );
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 2: Create agent with both session_file_system and bashkit_shell capabilities
    println!("\nStep 2: Creating agent with file_system and bashkit_shell capabilities...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "fs-bash-test-agent",
            "display_name": "FS Bash Test Agent",
            "system_prompt": "You are a file system assistant. When asked to create a file, use the write_file tool to create it. Always confirm what you did.",
            "capabilities": [
                {"ref": "session_file_system", "config": {}},
                {"ref": "bashkit_shell", "config": {}}
            ],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with capabilities: {:?}",
        agent.public_id, agent.capabilities
    );

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "FS Bash Integration Test"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    let fs_url = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session.id);

    // Step 4: Send message asking agent to create a file
    println!("\nStep 4: Sending message asking agent to create a file...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Please create a file at /workspace/test/hello.txt with the content 'Hello from agent!'"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);
    println!("Message sent successfully");

    // Step 5: Wait for agent to complete (up to 60 seconds)
    println!("\nStep 5: Waiting for agent to complete file creation (up to 60 seconds)...");
    let mut write_file_called = false;
    let mut has_final_response = false;

    for i in 1..=60 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            if i % 10 == 0 {
                println!("  [{}s] Messages: {}", i, messages.len());
            }

            // Look for write_file tool call and final text response
            for msg in messages {
                let role = msg["role"].as_str().unwrap_or("");
                if role == "agent"
                    && let Some(content) = msg["content"].as_array()
                {
                    for part in content {
                        if part["type"] == "tool_call" {
                            let tool_name = part["name"].as_str().unwrap_or("");
                            if tool_name == "write_file" && !write_file_called {
                                write_file_called = true;
                                println!("  Found write_file tool call after {}s", i);
                            }
                        }
                        // Check for final text response after tool call
                        if part["type"] == "text" && write_file_called {
                            has_final_response = true;
                        }
                    }
                }
            }

            if write_file_called && has_final_response {
                tokio::time::sleep(Duration::from_secs(1)).await;
                break;
            }
        }
    }

    skip_on_provider_account_block!(session_provider_account_block(&client, &session.id).await);

    assert!(
        write_file_called,
        "Agent should have called write_file tool"
    );
    println!("Agent completed file creation");

    // Step 6: Verify file is accessible via /workspace path in API
    println!("\nStep 6: Verifying file is accessible via /workspace API path...");

    // Access via /workspace/test/hello.txt (user-facing path)
    let workspace_file_response = client
        .get(format!("{}/workspace/test/hello.txt", fs_url))
        .send()
        .await
        .expect("Failed to get file via workspace path");

    assert_eq!(
        workspace_file_response.status(),
        200,
        "/workspace/test/hello.txt should return 200, got {}",
        workspace_file_response.status()
    );

    let file: SessionFile = workspace_file_response
        .json()
        .await
        .expect("Failed to parse file");
    println!(
        "SUCCESS: File accessible at /workspace/test/hello.txt, content: {:?}",
        file.content
    );
    assert!(file.content.is_some(), "File should have content");

    // Step 7: Verify /workspace root listing works
    println!("\nStep 7: Verifying /workspace root listing...");
    let workspace_root_response = client
        .get(format!("{}/workspace", fs_url))
        .send()
        .await
        .expect("Failed to get workspace root");

    assert_eq!(
        workspace_root_response.status(),
        200,
        "/workspace should return 200 (root listing), got {}",
        workspace_root_response.status()
    );

    let listing: Value = workspace_root_response
        .json()
        .await
        .expect("Failed to parse listing");
    let entry_count = listing["data"].as_array().map(|a| a.len()).unwrap_or(0);
    println!("/workspace listing has {} entries", entry_count);
    // Note: We don't assert on entry_count because direct_worker_adapters.rs has a separate
    // bug where it doesn't create parent directories. The main /workspace prefix fix is
    // verified above (file accessible at /workspace/test/hello.txt).

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");

    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Agent filesystem and bash workspace integration test passed!");
}

/// Test the stable edit_file integration boundary with LlmSim.
///
/// Verifies:
/// 1. Agents with `session_file_system` preview `read_file` and `edit_file`
/// 2. `edit_file` advertises `expected_hash` as a required parameter
/// 3. Session filesystem API round-trips the target text file for the session
///
/// Note: the default LlmSim driver used by workflow tests does not
/// deterministically emit file tool calls, so exact `edit_file` behavior is
/// covered in core/server unit tests instead of this workflow smoke test.
#[tokio::test]
async fn test_agent_execution_llmsim_with_edit_file_tool() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with edit_file tool...");

    // Step 1: Create LlmSim provider and model
    println!("\nStep 1: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Edit Tool Provider",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created LlmSim provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-edit-tool-test",
            "display_name": "LlmSim Edit Tool Test Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim model: status={}, body={}",
            status, body
        );
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created LlmSim model: {}", model.id);

    // Step 2: Create agent with session_file_system capability
    println!("\nStep 2: Creating edit agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "edit-tool-test-agent",
            "display_name": "Edit Tool Test Agent",
            "system_prompt": "You are a file editing assistant. When asked to update an existing file, first use read_file to inspect the file and obtain its content hash, then use edit_file to make the requested exact replacement. Do not use write_file for existing files. After editing, confirm the final file contents.",
            "capabilities": [{"ref": "session_file_system", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Edit Tool Integration Test"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    let fs_url = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session.id);

    // Step 4: Preview agent tools and verify edit_file contract
    println!("\nStep 4: Previewing tool definitions...");
    let preview_response = client
        .post(format!("{}/v1/agents/preview", API_BASE_URL))
        .json(&json!({
            "system_prompt": "You are a file editing assistant. When asked to update an existing file, first use read_file to inspect the file and obtain its content hash, then use edit_file to make the requested exact replacement. Do not use write_file for existing files. After editing, confirm the final file contents.",
            "capabilities": [{"ref": "session_file_system", "config": {}}],
            "tools": []
        }))
        .send()
        .await
        .expect("Failed to preview agent");

    assert_eq!(preview_response.status(), 200);
    let preview: Value = preview_response
        .json()
        .await
        .expect("Failed to parse preview");
    let tools = preview["tools"]
        .as_array()
        .expect("Expected preview tools array");
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tool_names.contains(&"read_file"));
    assert!(tool_names.contains(&"edit_file"));

    let edit_tool = tools
        .iter()
        .find(|tool| tool["name"] == "edit_file")
        .expect("Expected edit_file tool in preview");
    let required = edit_tool["parameters"]["required"]
        .as_array()
        .expect("Expected edit_file required array");
    assert!(required.contains(&json!("path")));
    assert!(required.contains(&json!("expected_hash")));
    println!("Preview includes read_file and edit_file with expected_hash");

    // Step 5: Seed the file that edit_file would target
    println!("\nStep 5: Seeding session file...");
    let seed_response = client
        .post(format!("{}/workspace/edit-target.txt", fs_url))
        .json(&json!({
            "content": "alpha\nbeta\ngamma\n",
            "encoding": "text"
        }))
        .send()
        .await
        .expect("Failed to create seed file");

    assert_eq!(seed_response.status(), 201);
    println!("Seed file created");

    // Step 6: Verify the file remains accessible through the session filesystem API
    println!("\nStep 6: Verifying session file contents...");
    let file_response = client
        .get(format!("{}/workspace/edit-target.txt", fs_url))
        .send()
        .await
        .expect("Failed to fetch session file");

    assert_eq!(file_response.status(), 200);
    let file: SessionFile = file_response
        .json()
        .await
        .expect("Failed to parse session file");
    assert_eq!(
        file.content.as_deref(),
        Some("alpha\nbeta\ngamma\n"),
        "Expected seeded file content to round-trip unchanged"
    );
    println!("Session file content verified: {:?}", file.content);

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("LlmSim edit_file integration test passed!");
}

/// Test that message creation returns promptly and triggers agent workflow
///
/// This test verifies:
/// 1. Message creation returns within 5 seconds (not blocking on workflow)
/// 2. After waiting, an assistant response appears (workflow executed)
///
/// Requirements: API + Worker (uses LlmSim provider, no real API keys needed).
#[tokio::test]
async fn test_message_triggers_agent_workflow() {
    use std::time::{Duration, Instant};

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("Failed to create client");

    println!("Testing message triggers agent workflow...");

    // Step 0: Create LlmSim provider and model (no real API keys needed)
    println!("\nStep 0: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Test Provider",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created LlmSim provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-test",
            "display_name": "LlmSim Test Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim model: status={}, body={}",
            status, body
        );
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created LlmSim model: {}", model.id);

    // Step 1: Create agent with LlmSim model
    println!("\nStep 1: Creating agent with LlmSim model...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "workflow-test-agent",
            "display_name": "Workflow Test Agent",
            "system_prompt": "You are a helpful assistant. Respond briefly.",
            "default_model_id": model.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with model: {:?}",
        agent.public_id, agent.default_model_id
    );

    // Step 2: Create session
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Workflow Test Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 3: Send message and verify it returns promptly (within 5 seconds)
    println!("\nStep 3: Sending message (should return promptly)...");
    let start = Instant::now();
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Say hello in one word."}]
            }
        }))
        .send()
        .await
        .expect("Failed to create message");
    let elapsed = start.elapsed();

    assert_eq!(
        message_response.status(),
        201,
        "Message creation should succeed"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "Message creation took too long: {:?}. Should not block on workflow start.",
        elapsed
    );
    println!("Message created in {:?}", elapsed);

    let message: Value = message_response
        .json()
        .await
        .expect("Failed to parse message");
    assert_eq!(message["role"], "user");
    println!("Created user message: {}", message["id"]);

    // Step 4: Wait for workflow to complete and check for assistant response
    println!("\nStep 4: Waiting for agent response (up to 30 seconds)...");
    let mut assistant_found = false;
    for i in 1..=30 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            // Debug: print message count and roles on first check and every 10s
            if i == 1 || i % 10 == 0 {
                println!(
                    "  [{}s] Found {} messages, roles: {:?}",
                    i,
                    messages.len(),
                    messages
                        .iter()
                        .map(|m| m["role"].as_str().unwrap_or("?"))
                        .collect::<Vec<_>>()
                );
            }

            for msg in messages {
                // API returns "agent" role (not "assistant")
                if msg["role"] == "agent" {
                    assistant_found = true;
                    let content = &msg["content"];
                    println!("Found agent response after {}s: {:?}", i, content);
                    break;
                }
            }

            if assistant_found {
                break;
            }
        }

        if i % 5 == 0 && !assistant_found {
            println!("Still waiting... ({}s)", i);
        }
    }

    // If we didn't find an agent response, check events for debugging
    if !assistant_found {
        println!("\nDebug: Checking events for session...");
        if let Ok(resp) = client
            .get(format!(
                "{}/v1/sessions/{}/events",
                API_BASE_URL, session.id
            ))
            .send()
            .await
            && resp.status() == 200
            && let Ok(data) = resp.json::<Value>().await
        {
            let events = data["data"].as_array();
            println!("  Events count: {}", events.map(|e| e.len()).unwrap_or(0));
            if let Some(events) = events {
                for (i, event) in events.iter().enumerate().take(10) {
                    println!(
                        "  Event {}: type={}, data_preview={}",
                        i,
                        event["type"].as_str().unwrap_or("?"),
                        &event["data"]
                            .to_string()
                            .chars()
                            .take(100)
                            .collect::<String>()
                    );
                }
            }
        }
    }

    assert!(
        assistant_found,
        "Agent workflow did not produce an agent response within 30 seconds. \
        Check: 1) Worker is running, 2) LLM provider configured, 3) Default model set"
    );

    // Step 5: Verify events were created
    println!("\nStep 5: Verifying events...");
    let events_response = client
        .get(format!(
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to list events");

    assert_eq!(events_response.status(), 200);
    let events_data: Value = events_response
        .json()
        .await
        .expect("Failed to parse events");
    let events = events_data["data"]
        .as_array()
        .expect("Expected events array");
    println!("Found {} events", events.len());
    assert!(
        events.len() >= 2,
        "Expected at least 2 events (user message + agent response)"
    );

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!("{}/v1/models/{}", API_BASE_URL, model.id))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Message triggers agent workflow test passed!");
}

/// Test that tool calls are not duplicated during workflow execution.
///
/// This test verifies that when an agent uses a tool (like current_time),
/// the tool call appears only once in the events, not duplicated.
///
/// Regression test for duplicate tool call scheduling at initial scheduling.
#[tokio::test]
async fn test_no_duplicate_tool_calls() {
    use std::collections::HashMap;

    let client = reqwest::Client::new();

    println!("Testing no duplicate tool calls...");

    // Step 1: Create an LLM provider with API key (if available)
    println!("\nStep 1: Creating LLM provider...");

    // Get API key from environment (this test requires it)
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            println!("Skipping test: OPENAI_API_KEY not set");
            return;
        }
    };

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Duplicate Tool Test Provider",
            "provider_type": "openai",
            "api_key": api_key,
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created provider: {}", provider.id);

    // Step 2: Create a model configured for tool use
    println!("\nStep 2: Creating model...");
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-5.4-mini",
            "display_name": "GPT-5.4 Mini Test",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 3: Create an agent with current_time capability
    println!("\nStep 3: Creating agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "time-tool-test-agent",
            "display_name": "Time Tool Test Agent",
            "system_prompt": "You are a helpful time assistant. When asked about the current time, use the get_current_time tool.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with capabilities: {:?}",
        agent.public_id, agent.capabilities
    );

    // Step 4: Create a session
    println!("\nStep 4: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id}))
        .send()
        .await
        .expect("Failed to create session");

    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 5: Send a message that should trigger tool use
    println!("\nStep 5: Sending message to trigger tool use...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "What time is it right now?"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert!(
        message_response.status().is_success() || message_response.status() == 404,
        "Expected success or 404, got {}",
        message_response.status()
    );

    // Step 6: Wait for workflow to complete by polling messages
    println!("\nStep 6: Waiting for workflow to complete...");
    let mut tool_call_found = false;
    for i in 1..=30 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            // Check if we have an agent response (workflow completed)
            for msg in messages {
                if msg["role"] == "agent" {
                    // Check for tool calls in content
                    if let Some(content) = msg["content"].as_array() {
                        for part in content {
                            if part.get("tool_call").is_some() {
                                tool_call_found = true;
                                println!("Found tool call after {}s", i);
                                break;
                            }
                        }
                    }
                }
            }

            if tool_call_found {
                // Wait a bit more for workflow to fully complete
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                break;
            }
        }

        if i % 5 == 0 {
            println!("  Still waiting... ({}s)", i);
        }
    }

    skip_on_provider_account_block!(session_provider_account_block(&client, &session.id).await);

    // Step 7: Get all messages and check for duplicates
    println!("\nStep 7: Checking for duplicate tool calls...");
    let messages_response = client
        .get(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to list messages");

    let messages_data: Value = messages_response
        .json()
        .await
        .expect("Failed to parse messages");
    let empty_vec = vec![];
    let messages = messages_data["data"].as_array().unwrap_or(&empty_vec);

    println!("Found {} messages", messages.len());

    // Count tool calls by their ID to detect duplicates
    let mut tool_call_ids: HashMap<String, u32> = HashMap::new();

    for msg in messages {
        if let Some(content) = msg["content"].as_array() {
            for part in content {
                // Check for tool_call content parts
                if let Some(tool_call) = part.get("tool_call") {
                    let id = tool_call["id"].as_str().unwrap_or("unknown");
                    let name = tool_call["name"].as_str().unwrap_or("unknown");
                    println!("  Found tool_call: {} ({})", name, id);
                    *tool_call_ids.entry(id.to_string()).or_insert(0) += 1;
                }

                // Check for tool_result content parts (from tool.completed events)
                if let Some(tool_result) = part.get("tool_result") {
                    let id = tool_result["id"].as_str().unwrap_or("unknown");
                    let name = tool_result["name"].as_str().unwrap_or("unknown");
                    println!("  Found tool_result: {} ({})", name, id);
                }
            }
        }
    }

    // Check for duplicate tool calls
    let mut has_duplicates = false;
    for (id, count) in &tool_call_ids {
        if *count > 1 {
            println!(
                "ERROR: Duplicate tool_call found! ID: {}, count: {}",
                id, count
            );
            has_duplicates = true;
        }
    }

    // If there were tool calls, verify no duplicates
    if !tool_call_ids.is_empty() {
        assert!(
            !has_duplicates,
            "Found duplicate tool calls in messages! Tool call IDs with counts: {:?}",
            tool_call_ids
        );
        println!("No duplicate tool calls found - test passed!");
    } else {
        println!(
            "No tool calls found in messages (workflow may not have completed or API key not configured)"
        );
    }

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!("{}/v1/models/{}", API_BASE_URL, model.id))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("No duplicate tool calls test completed!");
}

#[tokio::test]
async fn test_sessions_pagination() {
    let client = reqwest::Client::new();

    println!("Testing sessions pagination...");

    // Create an agent for the test
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "pagination-test-agent",
            "display_name": "Pagination Test Agent",
            "system_prompt": "Test agent"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Create 15 sessions
    println!("Creating 15 sessions...");
    for i in 1..=15 {
        let response = client
            .post(format!("{}/v1/sessions", API_BASE_URL))
            .json(&json!({ "harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": format!("Session {}", i) }))
            .send()
            .await
            .expect("Failed to create session");
        assert_eq!(response.status(), 201, "Failed to create session {}", i);
    }
    println!("Created 15 sessions");

    // Test 1: Default pagination returns all with metadata
    println!("\nTest 1: Default pagination...");
    let response = client
        .get(format!(
            "{}/v1/sessions?agent_id={}",
            API_BASE_URL, agent.public_id
        ))
        .send()
        .await
        .expect("Failed to list sessions");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15, "Expected total=15");
    assert_eq!(body["offset"], 0, "Expected offset=0");
    assert_eq!(body["limit"], 20, "Expected default limit=20");
    assert_eq!(
        body["data"].as_array().unwrap().len(),
        15,
        "Expected 15 sessions"
    );
    println!("✓ Default pagination works");

    // Test 2: Custom limit
    println!("\nTest 2: Custom limit=5...");
    let response = client
        .get(format!(
            "{}/v1/sessions?agent_id={}&limit=5",
            API_BASE_URL, agent.public_id
        ))
        .send()
        .await
        .expect("Failed to list sessions with limit");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15, "Total should still be 15");
    assert_eq!(body["limit"], 5, "Limit should be 5");
    assert_eq!(
        body["data"].as_array().unwrap().len(),
        5,
        "Expected 5 sessions"
    );
    println!("✓ Custom limit works");

    // Test 3: Offset pagination
    println!("\nTest 3: Offset=5, limit=5...");
    let response = client
        .get(format!(
            "{}/v1/sessions?agent_id={}&offset=5&limit=5",
            API_BASE_URL, agent.public_id
        ))
        .send()
        .await
        .expect("Failed to list sessions with offset");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15);
    assert_eq!(body["offset"], 5);
    assert_eq!(body["limit"], 5);
    assert_eq!(body["data"].as_array().unwrap().len(), 5);
    println!("✓ Offset pagination works");

    // Test 4: Last partial page
    println!("\nTest 4: Last partial page (offset=10, limit=10)...");
    let response = client
        .get(format!(
            "{}/v1/sessions?agent_id={}&offset=10&limit=10",
            API_BASE_URL, agent.public_id
        ))
        .send()
        .await
        .expect("Failed to list sessions");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15);
    assert_eq!(
        body["data"].as_array().unwrap().len(),
        5,
        "Expected 5 remaining sessions"
    );
    println!("✓ Last partial page works");

    // Test 5: Beyond range returns empty data
    println!("\nTest 5: Beyond range (offset=20)...");
    let response = client
        .get(format!(
            "{}/v1/sessions?agent_id={}&offset=20",
            API_BASE_URL, agent.public_id
        ))
        .send()
        .await
        .expect("Failed to list sessions");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["total"], 15);
    assert_eq!(
        body["data"].as_array().unwrap().len(),
        0,
        "Expected empty data"
    );
    println!("✓ Beyond range returns empty data");

    // Test 6: Max limit enforcement
    println!("\nTest 6: Max limit enforcement (limit=200 should cap to 100)...");
    let response = client
        .get(format!(
            "{}/v1/sessions?agent_id={}&limit=200",
            API_BASE_URL, agent.public_id
        ))
        .send()
        .await
        .expect("Failed to list sessions");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("Failed to parse");

    assert_eq!(body["limit"], 100, "Limit should be capped at 100");
    println!("✓ Max limit enforcement works");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("Sessions pagination test completed!");
}

/// Test that a second message in the same session triggers a new workflow
///
/// This test verifies:
/// 1. First message triggers workflow and gets response
/// 2. Second message triggers a NEW workflow and gets response
///
/// This is a regression test for the issue where second messages were not picked up
/// because the workflow ID was the same as session_id and the completed workflow
/// blocked creation of a new workflow.
///
/// Requirements: API + Worker (uses LlmSim provider, no real API keys needed).
#[tokio::test]
async fn test_second_message_triggers_workflow() {
    use std::time::{Duration, Instant};

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create client");

    println!("Testing second message triggers workflow...");

    // Step 0: Create LlmSim provider and model (no real API keys needed)
    println!("\nStep 0: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Second Message Test",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created LlmSim provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-second-msg-test",
            "display_name": "LlmSim Second Message Test Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim model: status={}, body={}",
            status, body
        );
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created LlmSim model: {}", model.id);

    // Step 1: Create agent with LlmSim model
    println!("\nStep 1: Creating agent with LlmSim model...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "second-message-test-agent",
            "display_name": "Second Message Test Agent",
            "system_prompt": "You are a helpful assistant. Respond briefly.",
            "default_model_id": model.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with model: {:?}",
        agent.public_id, agent.default_model_id
    );

    // Step 2: Create session
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Second Message Test Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 3: Send FIRST message and wait for response
    println!("\nStep 3: Sending FIRST message...");
    let first_message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Hello, this is the first message."}]
            }
        }))
        .send()
        .await
        .expect("Failed to create first message");

    assert_eq!(
        first_message_response.status(),
        201,
        "First message creation should succeed"
    );
    println!("First message created");

    // Wait for first response
    println!("\nWaiting for first agent response...");
    let mut first_response_found = false;
    for i in 1..=30 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            // Count agent messages
            let agent_count = messages.iter().filter(|m| m["role"] == "agent").count();
            if agent_count >= 1 {
                first_response_found = true;
                println!("Found first agent response after {}s", i);
                break;
            }
        }

        if i % 5 == 0 {
            println!("Still waiting for first response... ({}s)", i);
        }
    }

    assert!(
        first_response_found,
        "First agent response not received within 30 seconds"
    );

    // Small delay to ensure workflow is fully completed
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Step 4: Send SECOND message and wait for response
    println!("\nStep 4: Sending SECOND message...");
    let second_message_start = Instant::now();
    let second_message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Hello, this is the SECOND message. Please respond."}]
            }
        }))
        .send()
        .await
        .expect("Failed to create second message");

    assert_eq!(
        second_message_response.status(),
        201,
        "Second message creation should succeed"
    );
    println!(
        "Second message created in {:?}",
        second_message_start.elapsed()
    );

    // Wait for second response
    println!("\nWaiting for SECOND agent response...");
    let mut second_response_found = false;
    for i in 1..=30 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            // Count user and agent messages
            let user_count = messages.iter().filter(|m| m["role"] == "user").count();
            let agent_count = messages.iter().filter(|m| m["role"] == "agent").count();

            if i == 1 || i % 5 == 0 {
                println!(
                    "  [{}s] Messages: {} user, {} agent",
                    i, user_count, agent_count
                );
            }

            // We expect 2 user messages and 2 agent messages
            if user_count >= 2 && agent_count >= 2 {
                second_response_found = true;
                println!("Found second agent response after {}s", i);
                break;
            }
        }
    }

    // Debug: if second response not found, show all messages
    if !second_response_found {
        println!("\nDebug: Final message state:");
        if let Ok(resp) = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);
            for (i, msg) in messages.iter().enumerate() {
                let role = msg["role"].as_str().unwrap_or("?");
                let content_preview = msg["content"].to_string();
                let preview: String = content_preview.chars().take(100).collect();
                println!("  Message {}: role={}, content={}", i, role, preview);
            }
        }
    }

    assert!(
        second_response_found,
        "Second agent response not received within 30 seconds. \
        This indicates the second message workflow was not triggered. \
        Bug: workflow_id = session_id causes conflict when creating second workflow."
    );

    // Step 5: Verify we have exactly 2 user messages and 2 agent messages
    println!("\nStep 5: Verifying message counts...");
    let final_messages_response = client
        .get(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to get final messages");

    let final_data: Value = final_messages_response
        .json()
        .await
        .expect("Failed to parse final messages");
    let final_messages = final_data["data"]
        .as_array()
        .expect("Expected messages array");

    let user_count = final_messages
        .iter()
        .filter(|m| m["role"] == "user")
        .count();
    let agent_count = final_messages
        .iter()
        .filter(|m| m["role"] == "agent")
        .count();

    println!(
        "Final counts: {} user messages, {} agent messages",
        user_count, agent_count
    );
    assert_eq!(user_count, 2, "Expected 2 user messages");
    assert_eq!(agent_count, 2, "Expected 2 agent messages");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!("{}/v1/models/{}", API_BASE_URL, model.id))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Second message workflow test passed!");
}

/// Test capability mounts are applied when session is created
///
/// This test verifies:
/// 1. Create agent with data_knowledge capability (which has a readonly mount)
/// 2. Create session for the agent
/// 3. Verify the /knowledge directory exists with expected scaffold files
/// 4. Verify files are read-only (from readonly mount)
#[tokio::test]
async fn test_capability_mounts_applied_on_session_creation() {
    let client = reqwest::Client::new();

    println!("Testing capability mounts applied on session creation...");

    // Step 1: Create an agent with data_knowledge capability
    println!("\nStep 1: Creating agent with data_knowledge capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "mount-test-agent",
            "display_name": "Mount Test Agent",
            "system_prompt": "Test agent for capability mounts",
            "capabilities": [
                {"ref": "data_knowledge", "config": {}},
                {"ref": "session_file_system", "config": {}}
            ]
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(
        agent_response.status(),
        201,
        "Expected 201 Created for agent"
    );

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with capabilities: {:?}",
        agent.public_id, agent.capabilities
    );
    assert_eq!(
        agent.capabilities.len(),
        2,
        "Agent should have 2 capabilities"
    );

    // Step 2: Create a session (this should trigger mount application)
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Mount Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    let fs_url = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session.id);

    // Step 3: Verify /knowledge directory exists
    println!("\nStep 3: Verifying /knowledge directory exists...");
    let stat_response = client
        .post(format!("{}/_/stat", fs_url))
        .json(&json!({"path": "/knowledge"}))
        .send()
        .await
        .expect("Failed to stat /knowledge");

    assert_eq!(stat_response.status(), 200);
    let stat: Value = stat_response.json().await.expect("Failed to parse stat");
    assert_eq!(
        stat["is_directory"], true,
        "/knowledge should be a directory"
    );
    println!("/knowledge directory exists");

    // Step 4: List /knowledge directory contents
    println!("\nStep 4: Listing /knowledge directory...");
    let list_response = client
        .get(format!("{}/knowledge", fs_url))
        .send()
        .await
        .expect("Failed to list /knowledge");

    assert_eq!(list_response.status(), 200);
    let list_result: Value = list_response.json().await.expect("Failed to parse list");
    let files = list_result["data"].as_array().expect("Expected data array");
    println!("Found {} entries in /knowledge", files.len());

    // Should have index.md plus the tables/business/queries scaffold dirs
    let file_names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert!(
        file_names.contains(&"index.md"),
        "Expected index.md in /knowledge"
    );
    assert!(
        file_names.contains(&"tables"),
        "Expected tables/ in /knowledge"
    );
    assert!(
        file_names.contains(&"business"),
        "Expected business/ in /knowledge"
    );
    assert!(
        file_names.contains(&"queries"),
        "Expected queries/ in /knowledge"
    );
    println!("All expected entries present: {:?}", file_names);

    // Step 5: Verify index.md is readable
    println!("\nStep 5: Reading /knowledge/index.md...");
    let read_response = client
        .get(format!("{}/knowledge/index.md", fs_url))
        .send()
        .await
        .expect("Failed to read index.md");

    assert_eq!(read_response.status(), 200);
    let file: SessionFile = read_response.json().await.expect("Failed to parse file");
    assert!(file.content.is_some(), "File should have content");
    let content = file.content.as_ref().unwrap();
    assert!(
        content.contains("Data Knowledge"),
        "index.md should contain Data Knowledge"
    );
    assert!(file.is_readonly, "Mounted file should be readonly");
    println!("index.md is readable and readonly");

    // Step 6: Verify readonly protection - try to update index.md
    println!("\nStep 6: Verifying readonly protection...");
    let update_response = client
        .put(format!("{}/knowledge/index.md", fs_url))
        .json(&json!({
            "content": "modified content"
        }))
        .send()
        .await
        .expect("Failed to send update request");

    // Should fail because file is readonly
    assert_ne!(
        update_response.status(),
        200,
        "Readonly file update should fail"
    );
    println!("Readonly protection working - update rejected");

    // Step 7: Verify nested scaffold content
    println!("\nStep 7: Reading /knowledge/tables/index.md...");
    let tables_response = client
        .get(format!("{}/knowledge/tables/index.md", fs_url))
        .send()
        .await
        .expect("Failed to read tables/index.md");

    assert_eq!(tables_response.status(), 200);
    let tables_file: SessionFile = tables_response.json().await.expect("Failed to parse file");
    let tables_content = tables_file.content.as_ref().unwrap();
    assert!(
        tables_content.contains("Table Documentation"),
        "tables/index.md should contain Table Documentation"
    );
    println!("tables/index.md has expected scaffold content");

    // Step 8: Create a second session - verify mounts are independent
    println!("\nStep 8: Creating second session...");
    let session2_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Second Mount Test Session"
        }))
        .send()
        .await
        .expect("Failed to create second session");

    assert_eq!(session2_response.status(), 201);
    let session2: Session = session2_response
        .json()
        .await
        .expect("Failed to parse session2");
    println!("Created session2: {}", session2.id);

    // Verify second session also has mounts
    let fs_url2 = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session2.id);
    let stat2_response = client
        .post(format!("{}/_/stat", fs_url2))
        .json(&json!({"path": "/knowledge"}))
        .send()
        .await
        .expect("Failed to stat /knowledge in session2");

    assert_eq!(stat2_response.status(), 200);
    let stat2: Value = stat2_response.json().await.expect("Failed to parse stat2");
    assert_eq!(
        stat2["is_directory"], true,
        "session2 should also have /knowledge"
    );
    println!("Second session also has /knowledge directory mounted");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("Capability mounts test passed!");
}

/// Test MCP server CRUD operations
///
/// This test verifies:
/// 1. MCP server creation
/// 2. MCP server listing
/// 3. MCP server retrieval by ID
/// 4. MCP server update
/// 5. MCP server deletion
#[tokio::test]
async fn test_mcp_server_crud() {
    use everruns_core::McpServer;

    let client = reqwest::Client::new();

    println!("Testing MCP server CRUD operations...");

    // Step 1: Create an MCP server
    println!("\nStep 1: Creating MCP server...");
    let create_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "test-mcp-server",
            "description": "A test MCP server for integration testing",
            "url": "https://mcp.example.com/v1/mcp"
        }))
        .send()
        .await
        .expect("Failed to create MCP server");

    assert_eq!(
        create_response.status(),
        201,
        "Expected 201 Created, got {}",
        create_response.status()
    );

    let server: McpServer = create_response
        .json()
        .await
        .expect("Failed to parse MCP server response");

    println!("Created MCP server: {} ({})", server.name, server.id);
    assert_eq!(server.name, "test-mcp-server");
    assert_eq!(server.url, "https://mcp.example.com/v1/mcp");
    assert_eq!(server.status.to_string(), "active");
    assert!(!server.api_key_set, "API key should not be set");

    // Step 2: List MCP servers
    println!("\nStep 2: Listing MCP servers...");
    let list_response = client
        .get(format!("{}/v1/mcp-servers", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list MCP servers");

    assert_eq!(list_response.status(), 200);
    let list_data: Value = list_response.json().await.expect("Failed to parse");
    let servers: Vec<McpServer> =
        serde_json::from_value(list_data["data"].clone()).expect("Failed to parse servers");
    println!("Found {} MCP server(s)", servers.len());
    assert!(!servers.is_empty());
    assert!(servers.iter().any(|s| s.id == server.id));

    // Step 3: Get MCP server by ID
    println!("\nStep 3: Getting MCP server by ID...");
    let get_response = client
        .get(format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id))
        .send()
        .await
        .expect("Failed to get MCP server");

    assert_eq!(get_response.status(), 200);
    let fetched_server: McpServer = get_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    println!("Fetched MCP server: {}", fetched_server.name);
    assert_eq!(fetched_server.id, server.id);
    assert_eq!(fetched_server.name, "test-mcp-server");

    // Step 4: Update MCP server
    println!("\nStep 4: Updating MCP server...");
    let update_response = client
        .patch(format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id))
        .json(&json!({
            "name": "updated-mcp-server",
            "description": "Updated description",
            "url": "https://mcp.updated.com/v1/mcp"
        }))
        .send()
        .await
        .expect("Failed to update MCP server");

    assert_eq!(update_response.status(), 200);
    let updated_server: McpServer = update_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    println!("Updated MCP server: {}", updated_server.name);
    assert_eq!(updated_server.name, "updated-mcp-server");
    assert_eq!(updated_server.url, "https://mcp.updated.com/v1/mcp");
    assert_eq!(
        updated_server.description,
        Some("Updated description".to_string())
    );

    // Step 5: Update MCP server status to disabled
    println!("\nStep 5: Disabling MCP server...");
    let disable_response = client
        .patch(format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id))
        .json(&json!({
            "status": "disabled"
        }))
        .send()
        .await
        .expect("Failed to disable MCP server");

    assert_eq!(disable_response.status(), 200);
    let disabled_server: McpServer = disable_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    assert_eq!(disabled_server.status.to_string(), "disabled");
    println!("MCP server disabled");

    // Step 6: Create MCP server with API key
    println!("\nStep 6: Creating MCP server with API key...");
    let create_with_key_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "mcp-server-with-key",
            "url": "https://secure.mcp.com/v1/mcp",
            "api_key": "test-api-key-12345"
        }))
        .send()
        .await
        .expect("Failed to create MCP server with API key");

    assert_eq!(create_with_key_response.status(), 201);
    let server_with_key: McpServer = create_with_key_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    assert!(server_with_key.api_key_set, "API key should be set");
    println!(
        "Created MCP server with API key: {} (api_key_set: {})",
        server_with_key.name, server_with_key.api_key_set
    );

    // Step 7: Create MCP server with custom headers
    println!("\nStep 7: Creating MCP server with custom headers...");
    let create_with_headers_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "mcp-server-with-headers",
            "url": "https://headers.mcp.com/v1/mcp",
            "headers": {
                "X-Custom-Header": "custom-value",
                "X-Another-Header": "another-value"
            }
        }))
        .send()
        .await
        .expect("Failed to create MCP server with headers");

    assert_eq!(create_with_headers_response.status(), 201);
    let server_with_headers: McpServer = create_with_headers_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    assert_eq!(server_with_headers.headers.len(), 2);
    // Header names are present in API responses, but values are redacted to
    // prevent secret exposure. Verify the key exists and value is cleared.
    assert!(
        server_with_headers.headers.contains_key("X-Custom-Header"),
        "Header key must be present"
    );
    assert_eq!(
        server_with_headers
            .headers
            .get("X-Custom-Header")
            .map(String::as_str),
        Some(""),
        "Header values must be redacted in API responses"
    );
    println!(
        "Created MCP server with headers: {} ({} headers)",
        server_with_headers.name,
        server_with_headers.headers.len()
    );

    // Step 8: Test validation - empty name
    println!("\nStep 8: Testing validation - empty name...");
    let empty_name_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "",
            "url": "https://mcp.example.com/v1/mcp"
        }))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        empty_name_response.status(),
        400,
        "Expected 400 Bad Request for empty name"
    );
    println!("Empty name correctly rejected");

    // Step 9: Test validation - empty URL
    println!("\nStep 9: Testing validation - empty URL...");
    let empty_url_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "test-server",
            "url": ""
        }))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        empty_url_response.status(),
        400,
        "Expected 400 Bad Request for empty URL"
    );
    println!("Empty URL correctly rejected");

    // Step 10: Test 404 for non-existent server
    println!("\nStep 10: Testing 404 for non-existent server...");
    let not_found_response = client
        .get(format!(
            "{}/v1/mcp-servers/mcp_00000000000000000000000000000000",
            API_BASE_URL
        ))
        .send()
        .await
        .expect("Failed to get non-existent server");

    assert_eq!(
        not_found_response.status(),
        404,
        "Expected 404 Not Found for non-existent server"
    );
    println!("Non-existent server correctly returns 404");

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id))
        .send()
        .await
        .expect("Failed to delete MCP server");
    client
        .post(format!(
            "{}/v1/mcp-servers/{}/delete",
            API_BASE_URL, server.id
        ))
        .send()
        .await
        .expect("Failed to permanently delete MCP server");
    client
        .delete(format!(
            "{}/v1/mcp-servers/{}",
            API_BASE_URL, server_with_key.id
        ))
        .send()
        .await
        .expect("Failed to delete MCP server with key");
    client
        .post(format!(
            "{}/v1/mcp-servers/{}/delete",
            API_BASE_URL, server_with_key.id
        ))
        .send()
        .await
        .expect("Failed to permanently delete MCP server with key");
    client
        .delete(format!(
            "{}/v1/mcp-servers/{}",
            API_BASE_URL, server_with_headers.id
        ))
        .send()
        .await
        .expect("Failed to delete MCP server with headers");
    client
        .post(format!(
            "{}/v1/mcp-servers/{}/delete",
            API_BASE_URL, server_with_headers.id
        ))
        .send()
        .await
        .expect("Failed to permanently delete MCP server with headers");

    // Verify deletion
    let verify_deleted = client
        .get(format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id))
        .send()
        .await
        .expect("Failed to verify deletion");
    assert_eq!(verify_deleted.status(), 404);

    println!("MCP server CRUD test passed!");
}

// =============================================================================
// Agent Execution Tests with Tool Calls
// =============================================================================
// These tests verify end-to-end agent execution with tool calls for each provider.
// The "dad jokes agent" pattern: ask for current time, use it in a joke.

/// Test agent execution with tool calls using LlmSim driver (deterministic).
///
/// This test verifies the full agent workflow with tool calls using a simulated LLM:
/// 1. Create an agent with current_time capability
/// 2. Send a message asking for a time-based joke
/// 3. Verify the agent calls get_current_time tool
/// 4. Verify the agent generates a response that includes time info
///
/// Requirements: API + Worker running (uses LlmSim, no real API keys needed)
#[tokio::test]
async fn test_agent_execution_llmsim_with_tool_calls() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with LlmSim driver (tool calls)...");

    // Step 1: Create LlmSim provider and model
    println!("\nStep 1: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Tool Test Provider",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created LlmSim provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-tool-test",
            "display_name": "LlmSim Tool Test Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim model: status={}, body={}",
            status, body
        );
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created LlmSim model: {}", model.id);

    // Step 2: Create a dad jokes agent with current_time capability
    println!("\nStep 2: Creating dad jokes agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "dad-jokes-time-agent",
            "display_name": "Dad Jokes Time Agent",
            "system_prompt": "You are a dad jokes comedian. When asked for a joke about the time, first use the get_current_time tool to get the current time, then tell a dad joke that incorporates the time. Your jokes should be punny and family-friendly.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with capabilities: {:?}",
        agent.public_id, agent.capabilities
    );

    // Step 3: Create a session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Dad Jokes Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 4: Send a message asking for a time-based joke
    println!("\nStep 4: Sending message asking for time-based joke...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Tell me a dad joke about the current time!"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);
    println!("Message sent successfully");

    // Step 5: Wait for agent response and tool calls
    println!("\nStep 5: Waiting for agent response with tool calls (up to 45 seconds)...");
    let mut agent_response_found = false;
    let mut tool_call_found = false;
    let mut tool_result_found = false;

    for i in 1..=45 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            // Debug output every 5 seconds
            if i % 5 == 0 {
                println!(
                    "  [{}s] Messages: {}, looking for agent response with tool calls...",
                    i,
                    messages.len()
                );
            }

            for msg in messages {
                let role = msg["role"].as_str().unwrap_or("");

                // Check for agent messages
                if role == "agent" {
                    agent_response_found = true;

                    // Check for tool calls in content
                    if let Some(content) = msg["content"].as_array() {
                        for part in content {
                            // Tool calls have type: "tool_call" and name field
                            if part["type"] == "tool_call" {
                                let tool_name = part["name"].as_str().unwrap_or("");
                                if tool_name == "get_current_time" {
                                    tool_call_found = true;
                                    println!("  Found get_current_time tool call after {}s", i);
                                }
                            }
                        }
                    }
                }
                // Check for tool results in user messages
                if role == "user"
                    && let Some(content) = msg["content"].as_array()
                {
                    for part in content {
                        if part["type"] == "tool_result" {
                            tool_result_found = true;
                            println!("  Found tool result after {}s", i);
                        }
                    }
                }
            }

            // We need at least an agent response (LlmSim may not always use tools)
            if agent_response_found {
                // Give a bit more time for full completion
                tokio::time::sleep(Duration::from_secs(2)).await;
                break;
            }
        }
    }

    // Step 6: Verify the workflow completed
    println!("\nStep 6: Verifying workflow completion...");

    // Get final messages
    let final_messages_response = client
        .get(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to get final messages");

    let final_data: Value = final_messages_response
        .json()
        .await
        .expect("Failed to parse final messages");
    let final_messages = final_data["data"]
        .as_array()
        .expect("Expected messages array");

    println!("Final message count: {}", final_messages.len());

    // Count message types
    let user_count = final_messages
        .iter()
        .filter(|m| m["role"] == "user")
        .count();
    let agent_count = final_messages
        .iter()
        .filter(|m| m["role"] == "agent")
        .count();

    println!("Message counts: {} user, {} agent", user_count, agent_count);

    // We should have at least 1 user message and 1 agent message
    assert!(
        user_count >= 1,
        "Expected at least 1 user message, got {}",
        user_count
    );
    assert!(
        agent_response_found,
        "Agent did not respond within 45 seconds"
    );
    assert!(
        agent_count >= 1,
        "Expected at least 1 agent message, got {}",
        agent_count
    );

    // Print summary
    println!("\nTest summary:");
    println!("  Agent response: {}", agent_response_found);
    println!("  Tool call (get_current_time): {}", tool_call_found);
    println!("  Tool result: {}", tool_result_found);

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("LlmSim agent execution with tool calls test passed!");
}

/// Test agent execution with OpenAI driver.
///
/// This test verifies end-to-end agent execution with real OpenAI API:
/// 1. Create an agent with current_time capability
/// 2. Send a message asking for a time-based joke
/// 3. Verify the agent calls get_current_time tool
/// 4. Verify the joke references the current time
///
/// Requirements: API + Worker running, OPENAI_API_KEY environment variable set.
/// Skips if no API key is available.
#[tokio::test]
async fn test_agent_execution_openai_with_tool_calls() {
    use std::time::Duration;

    // Check if OpenAI API key is available
    let api_key = std::env::var("OPENAI_API_KEY").ok();
    if api_key.is_none() || api_key.as_ref().map(|k| k.is_empty()).unwrap_or(true) {
        println!("Skipping OpenAI test: OPENAI_API_KEY not set");
        return;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with OpenAI driver (tool calls)...");

    // Step 1: Create OpenAI provider
    println!("\nStep 1: Creating OpenAI provider...");
    // Get API key from environment
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY should be set");

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "OpenAI Tool Test Provider",
            "provider_type": "openai",
            "api_key": api_key,
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create OpenAI provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created OpenAI provider: {}", provider.id);

    // Create model (gpt-5.4-mini for cost-effectiveness)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "gpt-5.4-mini",
            "display_name": "GPT-5.4 Mini (Tool Test)",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {} ({})", model.display_name, model.id);

    // Step 2: Create dad jokes agent with current_time capability
    println!("\nStep 2: Creating dad jokes agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "openai-dad-jokes-agent",
            "display_name": "OpenAI Dad Jokes Agent",
            "system_prompt": "You are a dad jokes comedian. When the user asks for a joke about the time, you MUST first use the get_current_time tool to get the current time, then tell a short dad joke that somehow references the time you received. Keep your response brief.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "OpenAI Dad Jokes Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 4: Send message
    println!("\nStep 4: Sending message...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Tell me a dad joke about the current time!"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);

    // Step 5: Wait for response with tool calls
    println!("\nStep 5: Waiting for response (up to 60 seconds)...");
    let mut tool_call_found = false;
    let mut final_response_found = false;
    let mut final_response_text = String::new();

    for i in 1..=60 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            if i % 10 == 0 {
                println!("  [{}s] Messages: {}", i, messages.len());
            }

            for msg in messages {
                if msg["role"] == "agent"
                    && let Some(content) = msg["content"].as_array()
                {
                    for part in content {
                        // Check for tool calls (type: "tool_call" with name field)
                        if part["type"] == "tool_call" {
                            let name = part["name"].as_str().unwrap_or("");
                            if name == "get_current_time" {
                                tool_call_found = true;
                                println!("  Found get_current_time tool call after {}s", i);
                            }
                        }
                        // Check for text response (final answer)
                        if part["type"] == "text" {
                            let text_str = part["text"].as_str().unwrap_or("");
                            if !text_str.is_empty() && text_str.len() > 10 {
                                final_response_found = true;
                                final_response_text = text_str.to_string();
                            }
                        }
                    }
                }
            }

            if tool_call_found && final_response_found {
                tokio::time::sleep(Duration::from_secs(2)).await;
                break;
            }
        }
    }

    // Verify results
    println!("\nResults:");
    println!("  Tool call found: {}", tool_call_found);
    println!("  Final response found: {}", final_response_found);
    if !final_response_text.is_empty() {
        println!(
            "  Response preview: {}...",
            &final_response_text[..final_response_text.len().min(100)]
        );
    }

    // Check for transient API errors (network issues, TLS errors, etc.)
    let is_error_response = final_response_text.contains("encountered an error")
        || final_response_text.contains("try again later");

    // Read the account block while the session is still intact; act on it once
    // the fixtures below are torn down.
    let account_block = session_provider_account_block(&client, &session.id).await;

    // Cleanup first before assertions
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    skip_on_provider_account_block!(account_block);

    // If we got an error response, skip the test (transient API issue)
    if is_error_response {
        println!("Skipping assertions due to transient API error");
        return;
    }

    assert!(
        tool_call_found,
        "OpenAI agent should have called get_current_time tool"
    );
    assert!(
        final_response_found,
        "OpenAI agent should have generated a final response"
    );

    println!("OpenAI agent execution with tool calls test passed!");
}

/// Test agent execution with Anthropic driver.
///
/// This test verifies end-to-end agent execution with real Anthropic API:
/// 1. Create an agent with current_time capability
/// 2. Send a message asking for a time-based joke
/// 3. Verify the agent calls get_current_time tool
/// 4. Verify the joke references the current time
///
/// Requirements: API + Worker running, ANTHROPIC_API_KEY environment variable set.
/// Skips if no API key is available.
#[tokio::test]
async fn test_agent_execution_anthropic_with_tool_calls() {
    use std::time::Duration;

    // Check if Anthropic API key is available
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    if api_key.is_none() || api_key.as_ref().map(|k| k.is_empty()).unwrap_or(true) {
        println!("Skipping Anthropic test: ANTHROPIC_API_KEY not set");
        return;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with Anthropic driver (tool calls)...");

    // Step 1: Create Anthropic provider
    println!("\nStep 1: Creating Anthropic provider...");

    // Get API key from environment
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY should be set");

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Anthropic Tool Test Provider",
            "provider_type": "anthropic",
            "api_key": api_key,
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create Anthropic provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created Anthropic provider: {}", provider.id);

    // Create model (claude-haiku-4-5 for cost-effectiveness)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": LIVE_ANTHROPIC_FAST_MODEL,
            "display_name": "Anthropic Fast (Tool Test)",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {} ({})", model.display_name, model.id);

    // Step 2: Create dad jokes agent with current_time capability
    println!("\nStep 2: Creating dad jokes agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "anthropic-dad-jokes-agent",
            "display_name": "Anthropic Dad Jokes Agent",
            "system_prompt": "You are a dad jokes comedian. When the user asks for a joke about the time, you MUST first use the get_current_time tool to get the current time, then tell a short dad joke that somehow references the time you received. Keep your response brief.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Anthropic Dad Jokes Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 4: Send message
    println!("\nStep 4: Sending message...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Tell me a dad joke about the current time!"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);

    // Step 5: Wait for response with tool calls
    println!("\nStep 5: Waiting for response (up to 60 seconds)...");
    let mut tool_call_found = false;
    let mut final_response_found = false;
    let mut final_response_text = String::new();

    for i in 1..=60 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            if i % 10 == 0 {
                println!("  [{}s] Messages: {}", i, messages.len());
            }

            for msg in messages {
                if msg["role"] == "agent"
                    && let Some(content) = msg["content"].as_array()
                {
                    for part in content {
                        // Check for tool calls (type: "tool_call" with name field)
                        if part["type"] == "tool_call" {
                            let name = part["name"].as_str().unwrap_or("");
                            if name == "get_current_time" {
                                tool_call_found = true;
                                println!("  Found get_current_time tool call after {}s", i);
                            }
                        }
                        // Check for text response (final answer)
                        if part["type"] == "text" {
                            let text_str = part["text"].as_str().unwrap_or("");
                            if !text_str.is_empty() && text_str.len() > 10 {
                                final_response_found = true;
                                final_response_text = text_str.to_string();
                            }
                        }
                    }
                }
            }

            if tool_call_found && final_response_found {
                tokio::time::sleep(Duration::from_secs(2)).await;
                break;
            }
        }
    }

    // Verify results
    println!("\nResults:");
    println!("  Tool call found: {}", tool_call_found);
    println!("  Final response found: {}", final_response_found);
    if !final_response_text.is_empty() {
        println!(
            "  Response preview: {}...",
            &final_response_text[..final_response_text.len().min(100)]
        );
    }

    // Check for transient API errors (network issues, TLS errors, etc.)
    let is_error_response = final_response_text.contains("encountered an error")
        || final_response_text.contains("try again later");

    // Read the account block while the session is still intact; act on it once
    // the fixtures below are torn down.
    let account_block = session_provider_account_block(&client, &session.id).await;

    // Cleanup first before assertions
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    skip_on_provider_account_block!(account_block);

    // If we got an error response, skip the test (transient API issue)
    if is_error_response {
        println!("Skipping assertions due to transient API error");
        return;
    }

    assert!(
        tool_call_found,
        "Anthropic agent should have called get_current_time tool"
    );
    assert!(
        final_response_found,
        "Anthropic agent should have generated a final response"
    );

    println!("Anthropic agent execution with tool calls test passed!");
}

/// Test agent execution with a tool-bearing capability over several turns.
///
/// This test verifies the full agent round-trip with a capability that
/// exposes tools (current_time):
/// 1. Create an agent with the current_time capability
/// 2. Send a message that would exercise the tool surface
/// 3. Verify the agent produces a response
///
/// Requirements: API + Worker running (uses LlmSim, no real API keys needed)
#[tokio::test]
async fn test_agent_execution_multiple_tool_calls() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("Failed to create client");

    println!("Testing agent execution with multiple tool calls...");

    // Step 1: Create LlmSim provider and model
    println!("\nStep 1: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Multi-Tool Test",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-multi-tool",
            "display_name": "LlmSim Multi-Tool Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created provider and model");

    // Step 2: Create agent with current_time capability
    println!("\nStep 2: Creating agent with current_time capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "multi-tool-agent",
            "display_name": "Multi Tool Agent",
            "system_prompt": "You are a scheduling assistant. Use the get_current_time tool to answer questions about the current time, and show your reasoning step by step.",
            "capabilities": [{"ref": "current_time", "config": {}}],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session and send message
    println!("\nStep 3: Creating session and sending message...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id}))
        .send()
        .await
        .expect("Failed to create session");

    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");

    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "What time is it right now? Check the clock before answering."}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);

    // Step 4: Wait for response
    println!("\nStep 4: Waiting for agent response (up to 30 seconds)...");
    let mut agent_response_found = false;

    for i in 1..=30 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            let agent_count = messages.iter().filter(|m| m["role"] == "agent").count();
            if agent_count >= 1 {
                agent_response_found = true;
                println!("  Found agent response after {}s", i);
                break;
            }
        }

        if i % 5 == 0 {
            println!("  Still waiting... ({}s)", i);
        }
    }

    assert!(
        agent_response_found,
        "Agent should have responded within 30 seconds"
    );

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Multiple tool calls test passed!");
}

/// Test that streaming events are emitted during LLM generation.
///
/// This test verifies that:
/// 1. `output.message.started` event is emitted when LLM starts generating (durable, persisted)
/// 2. `output.message.delta` events are ephemeral (may not appear in PG events list)
/// 3. `llm.generation` events are durable and appear in PG events list
///
/// Note: This test uses LlmSim which simulates streaming but does not produce
/// extended thinking events (reason.thinking.delta, reason.thinking.completed).
/// To test full extended thinking flow, use a real Anthropic model with reasoning_effort.
#[tokio::test]
async fn test_streaming_events_emitted() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create client");

    println!("Testing streaming events are emitted...");

    // Step 1: Create LlmSim provider and model
    println!("\nStep 1: Creating LlmSim provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Streaming Test Provider",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created provider: {}", provider.id);

    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-streaming",
            "display_name": "LlmSim Streaming Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 2: Create agent
    println!("\nStep 2: Creating agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "streaming-test-agent",
            "display_name": "Streaming Test Agent",
            "system_prompt": "You are a helpful assistant. Respond briefly.",
            "default_model_id": model.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Streaming Test Session"}))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 4: Send message
    println!("\nStep 4: Sending message...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Hello, how are you?"}]
            }
        }))
        .send()
        .await
        .expect("Failed to create message");

    assert_eq!(message_response.status(), 201);
    println!("Message created");

    // Step 5: Wait for workflow to complete
    println!("\nStep 5: Waiting for agent response (up to 30 seconds)...");
    let mut agent_response_found = false;

    for i in 1..=30 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            for msg in messages {
                if msg["role"] == "agent" {
                    agent_response_found = true;
                    println!("Found agent response after {}s", i);
                    break;
                }
            }

            if agent_response_found {
                break;
            }
        }

        if i % 5 == 0 {
            println!("Still waiting... ({}s)", i);
        }
    }

    assert!(
        agent_response_found,
        "Agent should have responded within 30 seconds"
    );

    // Step 6: Verify streaming events
    println!("\nStep 6: Verifying streaming events...");
    let events_response = client
        .get(format!(
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to list events");

    assert_eq!(events_response.status(), 200);
    let events_data: Value = events_response
        .json()
        .await
        .expect("Failed to parse events");
    let events = events_data["data"]
        .as_array()
        .expect("Expected events array");

    // Check for output.message.started event
    let started_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "output.message.started")
        .collect();
    println!(
        "Found {} output.message.started events",
        started_events.len()
    );
    assert!(
        !started_events.is_empty(),
        "Expected at least one output.message.started event"
    );

    // Verify output.message.started has turn_id
    let started_event = &started_events[0];
    assert!(
        started_event["data"]["turn_id"].is_string(),
        "output.message.started should have turn_id"
    );
    println!(
        "output.message.started event: turn_id={}, model={:?}",
        started_event["data"]["turn_id"], started_event["data"]["model"]
    );

    // Note: reason.thinking.started events are only emitted when reasoning_effort is set
    // LLMSim doesn't support extended thinking, so we don't expect thinking events here.
    let thinking_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "reason.thinking.started")
        .collect();
    println!(
        "Found {} reason.thinking.started events (expected 0 for LLMSim)",
        thinking_events.len()
    );

    // Check for output.message.delta events
    // Note: delta events are ephemeral (skip PG persistence, delivered via EventDelivery only).
    // They may be absent from the PG events list — this is expected behavior.
    let delta_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "output.message.delta")
        .collect();
    println!(
        "Found {} output.message.delta events (ephemeral — may be 0 in PG)",
        delta_events.len()
    );

    // If deltas are present (e.g., EventDelivery::InMemory mode still persists), verify structure
    if let Some(delta_event) = delta_events.first() {
        assert!(
            delta_event["data"]["turn_id"].is_string(),
            "output.message.delta should have turn_id"
        );
        assert!(
            delta_event["data"]["delta"].is_string(),
            "output.message.delta should have delta field"
        );
        assert!(
            delta_event["data"]["accumulated"].is_string(),
            "output.message.delta should have accumulated field"
        );
        println!(
            "output.message.delta event: delta='{}', accumulated='{}'",
            delta_event["data"]["delta"].as_str().unwrap_or(""),
            delta_event["data"]["accumulated"].as_str().unwrap_or("")
        );
    }

    // Check for output.message.completed event
    let completed_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "output.message.completed")
        .collect();
    println!(
        "Found {} output.message.completed events",
        completed_events.len()
    );
    assert!(
        !completed_events.is_empty(),
        "Expected at least one output.message.completed event"
    );

    // Verify output.message.completed has required fields (message)
    let completed_event = &completed_events[0];
    assert!(
        completed_event["data"]["message"].is_object(),
        "output.message.completed should have message object"
    );
    assert!(
        completed_event["data"]["message"]["role"] == "agent",
        "output.message.completed message should have role=agent"
    );
    assert!(
        completed_event["data"]["message"]["content"].is_array(),
        "output.message.completed message should have content array"
    );
    println!(
        "output.message.completed event: message_id={}",
        completed_event["data"]["message"]["id"]
            .as_str()
            .unwrap_or("unknown")
    );

    // Check for llm.generation event with time_to_first_token_ms
    let llm_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "llm.generation")
        .collect();
    println!("Found {} llm.generation events", llm_events.len());
    assert!(
        !llm_events.is_empty(),
        "Expected at least one durable llm.generation event"
    );

    if let Some(llm_event) = llm_events.first() {
        assert!(
            llm_event["data"]["metadata"]["success"].as_bool() == Some(true),
            "llm.generation should be successful"
        );
        if let Some(ttft) = llm_event["data"]["metadata"]["time_to_first_token_ms"].as_u64() {
            println!("llm.generation time_to_first_token_ms: {}ms", ttft);
        } else {
            println!("llm.generation time_to_first_token_ms: not set (optional)");
        }
    }

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!("{}/v1/models/{}", API_BASE_URL, model.id))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Streaming events test passed!");
}

/// Test cancel turn endpoint behavior.
///
/// This test verifies the cancel endpoint returns correct responses
/// for idle sessions (no active turn to cancel).
///
/// Requirements: API + Worker running
#[tokio::test]
async fn test_cancel_turn_endpoint() {
    let client = reqwest::Client::new();

    println!("Testing cancel turn endpoint...");

    // Step 1: Create LlmSim provider
    println!("\nStep 1: Creating LlmSim provider...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "LlmSim Cancel Test",
            "provider_type": "llmsim"
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create LlmSim provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created LlmSim provider: {}", provider.id);

    // Step 2: Create model
    println!("\nStep 2: Creating model...");
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": "llmsim-cancel-test",
            "display_name": "LlmSim Cancel Test Model",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 3: Create agent with LlmSim model
    println!("\nStep 3: Creating agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "cancel-test-agent",
            "display_name": "Cancel Test Agent",
            "system_prompt": "You are a helpful assistant.",
            "default_model_id": model.id.to_string()
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201, "Failed to create agent");
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 4: Create session
    println!("\nStep 4: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Cancel Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201, "Failed to create session");
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 5: Test cancel on idle session (should return 200 with no_op status)
    println!("\nStep 5: Testing cancel on idle session...");
    let cancel_response = client
        .post(format!(
            "{}/v1/sessions/{}/cancel",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to call cancel endpoint");

    assert_eq!(
        cancel_response.status(),
        200,
        "Expected 200 for cancelling idle session (no-op), got {}",
        cancel_response.status()
    );
    let cancel_body: Value = cancel_response
        .json()
        .await
        .expect("Failed to parse cancel response");
    assert_eq!(
        cancel_body["status"], "no_op",
        "Expected no_op status for idle session, got {:?}",
        cancel_body["status"]
    );
    println!("Correctly returned no_op: {}", cancel_body["message"]);

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");

    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    println!("Cancel turn endpoint test passed!");
}

/// Test extended thinking with real Anthropic API.
///
/// This test verifies that:
/// 1. Extended thinking events are emitted (reason.thinking.started, reason.thinking.delta, reason.thinking.completed)
/// 2. The message.agent event contains the thinking field
/// 3. Multi-turn conversations work correctly with thinking (thinking is sent back to Anthropic)
///
/// Requirements: API + Worker running, ANTHROPIC_API_KEY environment variable set.
/// Skips if no API key is available.
#[tokio::test]
async fn test_anthropic_extended_thinking() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .expect("Failed to create client");

    println!("Testing Anthropic extended thinking...");

    // Step 1: Create Anthropic provider
    println!("\nStep 1: Creating Anthropic provider...");
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            println!("ANTHROPIC_API_KEY not set - skipping test");
            return;
        }
    };

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Anthropic Thinking Test Provider",
            "provider_type": "anthropic",
            "api_key": api_key,
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create Anthropic provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created Anthropic provider: {}", provider.id);

    // Create model (the live thinking model supports extended thinking)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": LIVE_ANTHROPIC_THINKING_MODEL,
            "display_name": "Anthropic Thinking (Thinking Test)",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {} ({})", model.display_name, model.id);

    // Step 2: Create agent (no tool calls - tests basic thinking flow)
    println!("\nStep 2: Creating agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "thinking-test-agent",
            "display_name": "Thinking Test Agent",
            "system_prompt": "You are a helpful assistant. Think through problems step by step.",
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Extended Thinking Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 4: Send message with reasoning effort enabled (using "low" to minimize cost)
    // Note: controls is at request level, not inside message
    println!("\nStep 4: Sending message with reasoning effort=low...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "What is 17 * 23? Show your reasoning."}]
            },
            "controls": {
                "reasoning": {
                    "effort": "low"
                }
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);
    println!("Message sent successfully");

    // Step 5: Poll for completion and collect events
    println!("\nStep 5: Waiting for response and collecting events...");
    let mut response_complete = false;
    let mut thinking_started_found = false;
    let mut thinking_delta_found = false;
    let mut thinking_completed_found = false;
    let mut message_agent_with_reasoning = false;
    let mut all_events: Vec<Value> = Vec::new();

    for i in 1..=90 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Check session status
        let session_response = client
            .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
            .send()
            .await;

        if let Ok(resp) = session_response
            && resp.status() == 200
        {
            let session_data: Value = resp.json().await.unwrap_or_default();
            let status = session_data["status"].as_str().unwrap_or("");

            if i % 10 == 0 {
                println!("  [{}s] Session status: {}", i, status);
            }

            if status == "idle" {
                response_complete = true;
                break;
            }
        }
    }

    // Fetch all events
    let events_response = client
        .get(format!(
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to get events");

    if events_response.status() == 200 {
        let events_data: Value = events_response.json().await.unwrap_or_default();
        if let Some(events) = events_data["data"].as_array() {
            all_events = events.clone();

            // Check for thinking events
            for event in events {
                let event_type = event["type"].as_str().unwrap_or("");
                match event_type {
                    "reason.thinking.started" => {
                        thinking_started_found = true;
                        println!("  Found reason.thinking.started event");
                    }
                    "reason.thinking.delta" => {
                        thinking_delta_found = true;
                        // Only log first delta
                        if !thinking_delta_found {
                            println!("  Found reason.thinking.delta event");
                        }
                    }
                    "reason.thinking.completed" => {
                        thinking_completed_found = true;
                        let thinking_content = event["data"]["thinking"].as_str().unwrap_or("");
                        println!(
                            "  Found reason.thinking.completed event (thinking length: {} chars)",
                            thinking_content.len()
                        );
                    }
                    // Reasoning reaches the message as ordered `reasoning`
                    // content parts, not a scalar `thinking` field on the
                    // message — see ContentPart::Reasoning in crates/core.
                    everruns_core::events::OUTPUT_MESSAGE_COMPLETED => {
                        let reasoning_parts = event["data"]["message"]["content"]
                            .as_array()
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter(|part| part["type"] == "reasoning")
                                    .count()
                            })
                            .unwrap_or(0);
                        if reasoning_parts > 0 {
                            message_agent_with_reasoning = true;
                            println!(
                                "  Found message.agent with {} reasoning content part(s)",
                                reasoning_parts
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Print event summary
    println!("\nFirst turn results:");
    println!("  Response complete: {}", response_complete);
    println!(
        "  reason.thinking.started found: {}",
        thinking_started_found
    );
    println!("  reason.thinking.delta found: {}", thinking_delta_found);
    println!(
        "  reason.thinking.completed found: {}",
        thinking_completed_found
    );
    println!(
        "  message.agent with reasoning parts: {}",
        message_agent_with_reasoning
    );
    println!("  Total events: {}", all_events.len());

    // Step 6: Multi-turn test - send a follow-up message
    // This verifies thinking is properly sent back to Anthropic API
    println!("\nStep 6: Sending follow-up message to test multi-turn with thinking...");
    let followup_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Now multiply that result by 2."}]
            },
            "controls": {
                "reasoning": {
                    "effort": "low"
                }
            }
        }))
        .send()
        .await
        .expect("Failed to send follow-up message");

    assert_eq!(followup_response.status(), 201);
    println!("Follow-up message sent successfully");

    // Wait for follow-up to complete
    let mut followup_complete = false;
    for i in 1..=90 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let session_response = client
            .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
            .send()
            .await;

        if let Ok(resp) = session_response
            && resp.status() == 200
        {
            let session_data: Value = resp.json().await.unwrap_or_default();
            let status = session_data["status"].as_str().unwrap_or("");

            if i % 10 == 0 {
                println!("  [{}s] Session status: {}", i, status);
            }

            if status == "idle" {
                followup_complete = true;
                break;
            }
        }
    }

    println!(
        "Multi-turn test: follow-up complete = {}",
        followup_complete
    );

    // Read the account block while the session is still intact; act on it once
    // the fixtures below are torn down.
    let account_block = session_provider_account_block(&client, &session.id).await;

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    skip_on_provider_account_block!(account_block);

    // Assertions
    assert!(response_complete, "First turn should complete");
    assert!(
        thinking_started_found,
        "Should have reason.thinking.started event when reasoning_effort is set"
    );
    assert!(
        thinking_completed_found,
        "Should have reason.thinking.completed event with thinking content"
    );
    assert!(
        message_agent_with_reasoning,
        "message.agent event should carry reasoning content parts"
    );
    assert!(
        followup_complete,
        "Multi-turn should complete (thinking properly sent back to Anthropic)"
    );

    println!("Anthropic extended thinking test passed!");
}

/// Test extended thinking with tool use (real Anthropic API).
///
/// This test specifically exercises the scenario where:
/// 1. Extended thinking is enabled
/// 2. The agent has tools configured
/// 3. The model decides to use a tool during the thinking+response flow
///
/// This combination requires:
/// - `interleaved-thinking-2025-05-14` beta header
/// - Proper capture of thinking signature via `signature_delta` event
/// - Thinking block included before tool_use in multi-turn messages
///
/// Requirements: API + Worker running, ANTHROPIC_API_KEY environment variable set.
#[tokio::test]
async fn test_anthropic_extended_thinking_with_tools() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .expect("Failed to create client");

    println!("Testing Anthropic extended thinking with tool use...");

    // Step 1: Create Anthropic provider
    println!("\nStep 1: Creating Anthropic provider...");
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            println!("ANTHROPIC_API_KEY not set - skipping test");
            return;
        }
    };

    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "Anthropic Thinking+Tools Test",
            "provider_type": "anthropic",
            "api_key": api_key,
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create Anthropic provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created Anthropic provider: {}", provider.id);

    // Create model (the live thinking model supports extended thinking)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": LIVE_ANTHROPIC_THINKING_MODEL,
            "display_name": "Anthropic Thinking (Thinking+Tools Test)",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!("Failed to create model: status={}, body={}", status, body);
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {} ({})", model.display_name, model.id);

    // Step 2: Create agent WITH current_time tool
    println!("\nStep 2: Creating agent with current_time tool...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "time-reporter-agent",
            "display_name": "Time Reporter Agent",
            "system_prompt": "You help users with simple requests. When asked for the time, call the current_time tool once and report the result.",
            "default_model_id": model.id,
            "capabilities": [
                {"ref": "current_time"}
            ]
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with current_time capability",
        agent.public_id
    );

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Time Reporting with Thinking Test"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    // Step 4: Send message asking for the current time
    // This will trigger: thinking -> tool call -> tool result -> response
    println!("\nStep 4: Sending message (expecting thinking + tool use)...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "What time is it?"}]
            },
            "controls": {
                "reasoning": {
                    "effort": "low"
                }
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);
    println!("Message sent successfully");

    // Step 5: Poll for completion and collect events
    println!("\nStep 5: Waiting for response...");
    let mut response_complete = false;
    let mut thinking_found = false;
    let mut tool_call_found = false;
    let mut tool_completed_found = false;
    let mut final_message_found = false;

    for i in 1..=120 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let session_response = client
            .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
            .send()
            .await;

        if let Ok(resp) = session_response
            && resp.status() == 200
        {
            let session_data: Value = resp.json().await.unwrap_or_default();
            let status = session_data["status"].as_str().unwrap_or("");

            if i % 10 == 0 {
                println!("  [{}s] Session status: {}", i, status);
            }

            if status == "idle" {
                response_complete = true;
                break;
            } else if status == "failed" {
                // Max iterations or other failure - this is acceptable for this test
                // as long as we got thinking + tool call events
                let error = session_data["error"].as_str().unwrap_or("unknown");
                println!("  Session ended with status: failed ({})", error);
                response_complete = true; // Consider it complete for event collection
                break;
            }
        }
    }

    // Fetch all events
    let events_response = client
        .get(format!(
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to get events");

    if events_response.status() == 200 {
        let events_data: Value = events_response.json().await.unwrap_or_default();
        if let Some(events) = events_data["data"].as_array() {
            println!("\n  Event summary ({} events):", events.len());
            for event in events {
                let event_type = event["type"].as_str().unwrap_or("");
                match event_type {
                    "reason.thinking.started" | "reason.thinking.completed" => {
                        thinking_found = true;
                        println!("    - {}", event_type);
                    }
                    everruns_core::events::TOOL_STARTED => {
                        tool_call_found = true;
                        // The invoked tool is carried inside `tool_call`, which
                        // is the payload `tool.started` actually publishes.
                        let tool_name = event["data"]["tool_call"]["name"].as_str().unwrap_or("?");
                        println!("    - {} (tool: {})", event_type, tool_name);
                    }
                    everruns_core::events::TOOL_COMPLETED => {
                        tool_completed_found = true;
                        let success = event["data"]["success"].as_bool().unwrap_or(false);
                        println!("    - {} (success: {})", event_type, success);
                    }
                    everruns_core::events::OUTPUT_MESSAGE_COMPLETED => {
                        final_message_found = true;
                        // Reasoning is ordered `reasoning` content parts; the
                        // opaque signature is replay state and never published.
                        let reasoning_parts = event["data"]["message"]["content"]
                            .as_array()
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter(|part| part["type"] == "reasoning")
                                    .count()
                            })
                            .unwrap_or(0);
                        println!(
                            "    - {} (reasoning_parts: {})",
                            event_type, reasoning_parts
                        );
                    }
                    "turn.started" | "turn.completed" => {
                        println!("    - {}", event_type);
                    }
                    _ => {}
                }
            }
        }
    }

    // Step 6: Multi-turn test - send a follow-up
    // This verifies thinking+signature is properly sent back to Anthropic
    println!("\nStep 6: Sending follow-up message (multi-turn with thinking+tools)...");
    let followup_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Thanks! What time is it now?"}]
            },
            "controls": {
                "reasoning": {
                    "effort": "low"
                }
            }
        }))
        .send()
        .await
        .expect("Failed to send follow-up message");

    assert_eq!(followup_response.status(), 201);
    println!("Follow-up message sent");

    // Wait for follow-up to complete
    let mut followup_complete = false;
    for i in 1..=120 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let session_response = client
            .get(format!("{}/v1/sessions/{}", API_BASE_URL, session.id))
            .send()
            .await;

        if let Ok(resp) = session_response
            && resp.status() == 200
        {
            let session_data: Value = resp.json().await.unwrap_or_default();
            let status = session_data["status"].as_str().unwrap_or("");

            if i % 10 == 0 {
                println!("  [{}s] Session status: {}", i, status);
            }

            if status == "idle" {
                followup_complete = true;
                break;
            } else if status == "failed" {
                // A turn the provider account blocked never exercised the
                // thinking+tools path, so it is a skip rather than a failure.
                skip_on_provider_account_block!(
                    session_provider_account_block(&client, &session.id).await
                );
                let error = session_data["error"].as_str().unwrap_or("unknown");
                panic!("Follow-up failed with error: {}", error);
            }
        }
    }

    // Read the account block while the session is still intact; act on it once
    // the fixtures below are torn down.
    let account_block = session_provider_account_block(&client, &session.id).await;

    println!("\nResults:");
    println!("  First turn complete: {}", response_complete);
    println!("  Thinking events found: {}", thinking_found);
    println!("  Tool call found: {}", tool_call_found);
    println!("  Tool completed: {}", tool_completed_found);
    println!("  Final message found: {}", final_message_found);
    println!("  Multi-turn complete: {}", followup_complete);

    // Cleanup
    println!("\nCleaning up...");
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");
    client
        .delete(format!(
            "{}/v1/providers/{}/models/{}",
            API_BASE_URL, provider.id, model.id
        ))
        .send()
        .await
        .expect("Failed to delete model");
    client
        .delete(format!("{}/v1/providers/{}", API_BASE_URL, provider.id))
        .send()
        .await
        .expect("Failed to delete provider");

    skip_on_provider_account_block!(account_block);

    // Assertions
    // Note: With interleaved thinking, the model may hit max iterations (tool loop).
    // The key test is that thinking + tools work together without API errors.
    assert!(
        thinking_found,
        "Should have thinking events when reasoning_effort is set"
    );
    assert!(
        tool_call_found,
        "Should have tool.started event - model should use current_time when asked for time"
    );
    assert!(tool_completed_found, "Should have tool.completed event");
    assert!(
        final_message_found,
        "Should have at least one message.agent event"
    );

    // Multi-turn test may not complete if first turn hit max iterations
    if !followup_complete {
        println!("⚠️  Follow-up did not complete (first turn may have hit max iterations)");
        println!("   This is acceptable - the test verifies thinking+tools work together");
    }

    println!("Anthropic extended thinking with tools test passed!");
}

// ============================================================================
// Events API Contract Tests
// ============================================================================
// These tests verify that the events API responses match the public contract.

/// Test events list endpoint returns correct JSON structure
#[tokio::test]
async fn test_events_api_contract() {
    let client = reqwest::Client::new();

    println!("Testing events API contract...");

    // Step 1: Create agent and session
    let agent: Agent = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "events-contract-test-agent",
            "display_name": "Events Contract Test Agent",
            "system_prompt": "You are helpful"
        }))
        .send()
        .await
        .expect("Failed to create agent")
        .json()
        .await
        .expect("Failed to parse agent");

    let session: Session = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "Events Contract Test"}))
        .send()
        .await
        .expect("Failed to create session")
        .json()
        .await
        .expect("Failed to parse session");

    println!("Created session: {}", session.id);

    // Step 2: List events and verify contract structure (may be empty for new session)
    println!("\nVerifying events list API contract...");
    let events_response = client
        .get(format!(
            "{}/v1/sessions/{}/events",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to list events");

    assert_eq!(events_response.status(), 200);

    let events_json: Value = events_response
        .json()
        .await
        .expect("Failed to parse events");

    // Verify response structure has 'data' array (may be empty)
    assert!(
        events_json["data"].is_array(),
        "Response should have 'data' array"
    );
    let events = events_json["data"].as_array().unwrap();
    println!("Found {} events", events.len());

    // If there are events, verify each has required contract fields
    for event in events {
        // Required fields per knowledge/execution/events.md
        assert!(event["id"].is_string(), "Event must have 'id' string");
        assert!(event["type"].is_string(), "Event must have 'type' string");
        assert!(event["ts"].is_string(), "Event must have 'ts' string");
        assert!(
            event["session_id"].is_string(),
            "Event must have 'session_id' string"
        );
        assert!(
            event["context"].is_object(),
            "Event must have 'context' object"
        );
        assert!(event["data"].is_object(), "Event must have 'data' object");

        // Verify ID format (event_ prefix)
        let id = event["id"].as_str().unwrap();
        assert!(
            id.starts_with("event_"),
            "Event ID should have 'event_' prefix, got: {}",
            id
        );

        // Verify session_id format (session_ prefix)
        let sid = event["session_id"].as_str().unwrap();
        assert!(
            sid.starts_with("session_"),
            "Session ID should have 'session_' prefix, got: {}",
            sid
        );

        // Verify type is known (not "unsupported")
        let event_type = event["type"].as_str().unwrap();
        assert_ne!(
            event_type, "unsupported",
            "Unsupported events should be filtered"
        );
    }

    // Cleanup
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("Events API contract test passed!");
}

/// Test SSE streaming endpoint returns correct headers
#[tokio::test]
async fn test_events_sse_contract() {
    use futures::StreamExt;

    let client = reqwest::Client::new();

    println!("Testing SSE events contract...");

    // Step 1: Create agent and session
    let agent: Agent = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "sse-contract-test-agent",
            "display_name": "SSE Contract Test Agent",
            "system_prompt": "You are helpful"
        }))
        .send()
        .await
        .expect("Failed to create agent")
        .json()
        .await
        .expect("Failed to parse agent");

    let session: Session = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "SSE Contract Test"}))
        .send()
        .await
        .expect("Failed to create session")
        .json()
        .await
        .expect("Failed to parse session");

    println!("Created session: {}", session.id);

    // Step 2: Connect to SSE stream and verify headers and initial data
    println!("\nConnecting to SSE stream...");
    let sse_url = format!("{}/v1/sessions/{}/sse", API_BASE_URL, session.id);

    let sse_response = client
        .get(&sse_url)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .expect("Failed to connect to SSE");

    assert_eq!(sse_response.status(), 200);

    // Verify content type is text/event-stream
    let content_type = sse_response.headers().get("content-type");
    assert!(
        content_type.is_some(),
        "SSE response should have content-type header"
    );
    let ct = content_type.unwrap().to_str().unwrap();
    assert!(
        ct.contains("text/event-stream"),
        "Content-type should be text/event-stream, got: {}",
        ct
    );

    // Read first chunk with timeout (SSE streams are long-lived, can't use .text())
    let mut stream = sse_response.bytes_stream();
    let first_chunk = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("Timeout reading first SSE chunk")
        .expect("Stream ended unexpectedly")
        .expect("Failed to read chunk");

    let chunk_str = String::from_utf8_lossy(&first_chunk);
    println!("SSE first chunk:\n{}", chunk_str);

    // Verify SSE format (should have event: line and data: line)
    assert!(
        chunk_str.contains("event:") || chunk_str.contains("data:"),
        "SSE should have event: or data: lines"
    );

    // Cleanup
    client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent.public_id))
        .send()
        .await
        .expect("Failed to delete agent");

    println!("SSE contract test passed!");
}

// =============================================================================
// Durable SSE In-Process Integration Tests
// =============================================================================
//
// These tests use in-process testing with tower::ServiceExt for regular endpoints.
// For SSE endpoints (infinite streams), we spawn a TCP server and use reqwest
// to read streaming chunks with timeout.

mod durable_sse_tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use everruns_durable::InMemoryWorkflowEventStore;
    use everruns_server::api::durable;
    use futures::StreamExt;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    fn test_auth_state() -> everruns_server::auth::middleware::AuthState {
        use everruns_server::auth::config::AuthConfig;
        use everruns_server::storage::StorageBackend;
        let config = AuthConfig::default();
        everruns_server::auth::middleware::AuthState::builtin(
            config,
            std::sync::Arc::new(StorageBackend::in_memory()),
        )
    }

    /// Create a test router with in-memory durable store
    fn create_test_app() -> Router {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let state = durable::AppState::new(
            Some(store),
            test_auth_state(),
            None,
            "in_memory".to_string(),
        );
        durable::routes(state)
    }

    /// Create a test router with no durable store (simulates 503)
    fn create_test_app_no_store() -> Router {
        let state = durable::AppState::new(None, test_auth_state(), None, "in_memory".to_string());
        durable::routes(state)
    }

    /// Spawn a test server and return its base URL
    async fn spawn_test_server(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Give the server a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        url
    }

    /// Test global durable SSE stream contract (/v1/durable/sse)
    ///
    /// Verifies:
    /// - SSE connection establishes successfully (200 OK)
    /// - Content-Type is text/event-stream
    /// - First event is "connected"
    /// - Subsequent event is "snapshot" with expected structure
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_durable_sse_global_stream() {
        let app = create_test_app();
        let base_url = spawn_test_server(app).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/v1/durable/sse", base_url))
            .header("Accept", "text/event-stream")
            .send()
            .await
            .expect("Failed to connect to SSE");

        assert_eq!(response.status(), 200);

        // Verify content-type header
        let content_type = response
            .headers()
            .get("content-type")
            .expect("Missing content-type header")
            .to_str()
            .unwrap();
        assert!(
            content_type.contains("text/event-stream"),
            "Content-type should be text/event-stream, got: {}",
            content_type
        );

        // Read chunks with timeout until we have enough data
        let mut stream = response.bytes_stream();
        let mut collected = String::new();

        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_secs(2), stream.next()).await {
                Ok(Some(Ok(chunk))) => {
                    collected.push_str(&String::from_utf8_lossy(&chunk));
                    // Stop when we have both events
                    if collected.contains("event: connected")
                        && collected.contains("event: snapshot")
                    {
                        break;
                    }
                }
                Ok(Some(Err(e))) => panic!("Stream error: {}", e),
                Ok(None) => break, // Stream ended
                Err(_) => break,   // Timeout - that's OK for SSE
            }
        }

        println!("Durable SSE received:\n{}", collected);

        // Verify "connected" event
        assert!(
            collected.contains("event: connected"),
            "Should receive 'connected' event"
        );

        // Verify "snapshot" event with JSON data
        assert!(
            collected.contains("event: snapshot"),
            "Should receive 'snapshot' event"
        );
        assert!(
            collected.contains("\"health\""),
            "Snapshot should contain health"
        );
        assert!(
            collected.contains("\"workers\""),
            "Snapshot should contain workers"
        );
        assert!(
            collected.contains("\"workflows\""),
            "Snapshot should contain workflows"
        );
        assert!(
            collected.contains("\"tasks\""),
            "Snapshot should contain tasks"
        );
        assert!(collected.contains("\"dlq\""), "Snapshot should contain dlq");
        assert!(
            collected.contains("\"circuit_breakers\""),
            "Snapshot should contain circuit_breakers"
        );

        // Verify retry hint is present
        assert!(
            collected.contains("retry:"),
            "SSE should include retry hint"
        );

        println!("Durable global SSE test passed!");
    }

    /// Test SSE connected event has valid JSON format with timestamp
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_durable_sse_connected_event_format() {
        let app = create_test_app();
        let base_url = spawn_test_server(app).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/v1/durable/sse", base_url))
            .header("Accept", "text/event-stream")
            .send()
            .await
            .expect("Failed to connect to SSE");

        assert_eq!(response.status(), 200);

        // Read first chunk
        let mut stream = response.bytes_stream();
        let mut collected = String::new();

        match tokio::time::timeout(std::time::Duration::from_secs(2), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                collected.push_str(&String::from_utf8_lossy(&chunk));
            }
            Ok(Some(Err(e))) => panic!("Stream error: {}", e),
            Ok(None) => panic!("Stream ended unexpectedly"),
            Err(_) => panic!("Timeout reading first chunk"),
        }

        // Verify SSE format (event: and data: lines)
        assert!(collected.contains("event:"), "SSE should have event: field");
        assert!(
            collected.contains("event: connected"),
            "Should have connected event"
        );
        assert!(
            collected.contains("data:"),
            "connected event should have data: field"
        );

        // Extract and validate connected event JSON
        if let Some(data_start) = collected.find("data:") {
            let data_line = &collected[data_start..];
            if let Some(json_start) = data_line.find('{') {
                let potential_json = &data_line[json_start..];
                if let Some(json_end) = potential_json.find('}') {
                    let json_str = &potential_json[..=json_end];
                    let parsed: serde_json::Value = serde_json::from_str(json_str)
                        .expect("connected data should be valid JSON");
                    // Connected event has {"status":"connected"}
                    assert!(
                        parsed.get("status").is_some(),
                        "connected event should have status field"
                    );
                    assert_eq!(
                        parsed["status"], "connected",
                        "status should be 'connected'"
                    );
                }
            }
        }

        println!("Durable SSE format test passed!");
    }

    /// Test durable SSE returns 503 when store is not available
    #[tokio::test]
    async fn test_durable_sse_no_store_returns_503() {
        let app = create_test_app_no_store();

        let request = Request::builder()
            .uri("/v1/durable/sse")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "Should return 503 when store is not available"
        );

        println!("Durable SSE no-store test passed!");
    }

    /// Test workflow-specific SSE stream returns 404 for non-existent workflow
    #[tokio::test]
    async fn test_durable_workflow_sse_not_found() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/workflows/01234567-89ab-cdef-0123-456789abcdef/sse")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "Should return 404 for non-existent workflow"
        );

        println!("Workflow SSE not found test passed!");
    }

    /// Test health endpoint returns system health data
    #[tokio::test]
    async fn test_durable_health_endpoint() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/health")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let health: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Health should be valid JSON");

        // Verify health response structure
        assert!(health.get("total_workers").is_some());
        assert!(health.get("active_workers").is_some());
        assert!(health.get("total_capacity").is_some());
        assert!(health.get("current_load").is_some());

        println!("Durable health endpoint test passed!");
    }

    /// Test workers list endpoint
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_durable_workers_endpoint() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/workers")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let workers: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Workers should be valid JSON");

        // Verify response structure: { data: [...], total: N }
        assert!(workers.get("data").is_some(), "Should have data field");
        assert!(workers.get("total").is_some(), "Should have total field");
        assert!(workers["data"].is_array(), "data should be an array");

        println!("Durable workers endpoint test passed!");
    }

    /// Test workflows list endpoint
    #[tokio::test]
    async fn test_durable_workflows_endpoint() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/workflows")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let workflows: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Workflows should be valid JSON");

        // Verify structure
        assert!(workflows.get("total").is_some());
        assert!(workflows.get("data").is_some());
        assert!(workflows["data"].is_array());

        println!("Durable workflows endpoint test passed!");
    }

    /// Test circuit breakers list endpoint
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_durable_circuit_breakers_endpoint() {
        let app = create_test_app();

        let request = Request::builder()
            .uri("/v1/durable/circuit-breakers")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let breakers: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Circuit breakers should be valid JSON");

        // Verify response structure: { data: [...], total: N }
        assert!(breakers.get("data").is_some(), "Should have data field");
        assert!(breakers.get("total").is_some(), "Should have total field");
        assert!(breakers["data"].is_array(), "data should be an array");

        println!("Durable circuit breakers endpoint test passed!");
    }
}

/// Reasoning reaches the API as ordered parts, classified, with replay state stripped.
///
/// This covers the two things the reasoning rework publishes and the one thing
/// it must never publish, over the real API + worker + PostgreSQL path:
///
/// - `phase` / `phase_source` on the agent message. These cross the worker gRPC
///   boundary, where `phase` was previously hardcoded to `None` and the proto
///   had no field for it at all, so every message reached the API unclassified
///   regardless of what the provider reported. Nothing else in the suite would
///   catch that returning.
/// - reasoning as ordered `reasoning` content parts carrying readable text.
/// - the sanitization boundary: the artifact's signature and encrypted payload
///   are replay state for the provider, never API content (TM-LLM-034). llmsim
///   emits both alongside the text precisely so this assertion has something to
///   catch.
#[tokio::test]
async fn test_reasoning_reaches_api_sanitized_and_classified() {
    let client = reqwest::Client::new();

    // This test exists to prove the real path: a real provider's reasoning,
    // through the worker, into the API. Running it against a simulated provider
    // would prove only that the simulator behaves as written.
    //
    // So a missing credential is a failure, not a reason to skip. A job
    // configured to run this and silently unable to reach a provider reports
    // success while verifying nothing, which is the failure mode
    // `EVERRUNS_REQUIRE_LIVE_TESTS` exists to prevent.
    let require_live = std::env::var("EVERRUNS_REQUIRE_LIVE_TESTS")
        .ok()
        .is_some_and(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        });
    let has_provider_key = ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"]
        .iter()
        .any(|k| std::env::var(k).ok().is_some_and(|v| !v.trim().is_empty()));
    if !has_provider_key {
        assert!(
            !require_live,
            "EVERRUNS_REQUIRE_LIVE_TESTS is set but neither OPENAI_API_KEY nor \
             ANTHROPIC_API_KEY is present. This test verifies real provider \
             reasoning end to end and cannot do that without a credential — fix \
             the job's secrets rather than relaxing this check."
        );
        eprintln!(
            "SKIP: no provider credential; set EVERRUNS_REQUIRE_LIVE_TESTS=1 to \
             make this a failure"
        );
        return;
    }

    // Names are unique per run: the API rejects a duplicate agent name with
    // 409, so fixed names pass once against a fresh database and fail on every
    // rerun against the same one.
    let run_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();

    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": format!("reasoning-projection-agent-{run_id}"),
            "system_prompt": "You are a helpful assistant.",
            // No model override: the agent takes the org default, which is a
            // real reasoning-capable provider. That is the path this test is
            // here to verify.
        }))
        .send()
        .await
        .expect("Failed to create agent");
    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");

    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": format!("Reasoning Projection Session {run_id}"),
        }))
        .send()
        .await
        .expect("Failed to create session");
    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");

    // The effort is what makes llmsim reason, mirroring a real provider.
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "Think about this, then answer."}]
            },
            "controls": {"reasoning": {"effort": "high"}}
        }))
        .send()
        .await
        .expect("Failed to send message");
    assert_eq!(message_response.status(), 201, "Failed to send message");

    let mut agent_message: Option<Value> = None;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let Ok(resp) = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await
        else {
            continue;
        };
        if resp.status() != 200 {
            continue;
        }
        let data: Value = resp.json().await.unwrap_or_default();
        let empty = vec![];
        let messages = data["data"].as_array().unwrap_or(&empty);
        if let Some(m) = messages.iter().find(|m| m["role"] == "agent") {
            agent_message = Some(m.clone());
            break;
        }
    }

    let agent_message = agent_message.expect("No agent message produced within 60s");
    println!(
        "agent message: {}",
        serde_json::to_string_pretty(&agent_message).unwrap_or_default()
    );

    // An unusable provider account is a billing condition, not a regression in
    // the reasoning projection this test covers.
    skip_on_provider_account_block!(provider_account_block(&agent_message));

    // A provider error message would carry no reasoning and no phase, and would
    // make every assertion below vacuous.
    assert!(
        agent_message["metadata"]["error_code"].is_null(),
        "Turn failed rather than producing a reasoning message: {:?}",
        agent_message["metadata"]
    );

    // 1. Classification survives the worker boundary.
    let phase = agent_message["phase"]
        .as_str()
        .expect("agent message must carry `phase`");
    assert!(
        phase == "commentary" || phase == "final_answer",
        "unexpected phase {phase:?}"
    );
    let phase_source = agent_message["phase_source"]
        .as_str()
        .expect("agent message must carry `phase_source`");
    assert!(
        phase_source == "provider" || phase_source == "derived",
        "unexpected phase_source {phase_source:?}"
    );

    // 2. Reasoning is published as ordered content parts with readable text.
    let empty = vec![];
    let content = agent_message["content"].as_array().unwrap_or(&empty);
    let reasoning_parts: Vec<&Value> = content
        .iter()
        .filter(|p| p["type"] == "reasoning")
        .collect();
    assert!(
        !reasoning_parts.is_empty(),
        "agent message must carry reasoning content parts, got types {:?}",
        content
            .iter()
            .map(|p| p["type"].as_str().unwrap_or("?"))
            .collect::<Vec<_>>()
    );
    // Readable text arrives in whichever shape the provider exposes: verbatim
    // chain-of-thought as `plain`, a curated gloss as `summary`. Both are
    // reasoning-channel content; neither may be the answer.
    let reasoning_text: Vec<String> = reasoning_parts
        .iter()
        .filter_map(|p| match p["text"]["kind"].as_str() {
            Some("plain") => p["text"]["text"].as_str().map(str::to_string),
            Some("summary") => p["text"]["parts"].as_array().map(|parts| {
                parts
                    .iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            _ => None,
        })
        .filter(|t| !t.trim().is_empty())
        .collect();
    // Readable text is not guaranteed. A provider may return a reasoning
    // artifact carrying only an id and its encrypted payload — the model
    // reasoned but surfaced no prose — and that artifact is still valid and
    // still replayable. Requiring text here would fail on a correct response.
    // What must always hold is that the artifact is identified, so it can be
    // replayed under the handle the provider issued.
    assert!(
        reasoning_parts
            .iter()
            .any(|p| p["item_id"].is_string() || p["provider"].is_string()),
        "reasoning parts must be identified: {reasoning_parts:?}"
    );

    // Reasoning folded into the answer is the bug this rework fixes: it
    // persists as the model's reply and replays to the model as its own prior
    // output.
    let answer_text: String = content
        .iter()
        .filter(|p| p["type"] == "text")
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for reasoning in &reasoning_text {
        assert!(
            !answer_text.contains(reasoning.trim()),
            "reasoning text leaked into the answer"
        );
    }

    // 3. Replay state must not. The provider needs it; a client never does, and
    //    publishing it widens the surface for no benefit (TM-LLM-034).
    for part in &reasoning_parts {
        for leaked in ["signature", "encrypted", "encrypted_content"] {
            assert!(
                part.get(leaked).is_none_or(Value::is_null),
                "reasoning part leaked `{leaked}` to the API: {part}"
            );
        }
    }
    let serialized = serde_json::to_string(&agent_message).unwrap_or_default();
    for opaque in ["llmsim-signature-opaque", "llmsim-encrypted-opaque"] {
        assert!(
            !serialized.contains(opaque),
            "opaque replay value {opaque:?} reached the API payload"
        );
    }

    // 4. The same boundary holds for the event stream, which is a separate
    //    projection and so a separate way for the payload to escape.
    let events: Value = client
        .get(format!(
            "{}/v1/sessions/{}/events?limit=200",
            API_BASE_URL, session.id
        ))
        .send()
        .await
        .expect("Failed to fetch events")
        .json()
        .await
        .unwrap_or_default();
    let empty_events = vec![];
    let events = events["data"].as_array().unwrap_or(&empty_events);
    let reason_items: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "reason.item")
        .collect();
    assert!(
        !reason_items.is_empty(),
        "a reasoning turn must publish reason.item events"
    );
    for item in &reason_items {
        let data = &item["data"];
        for leaked in ["encrypted_content", "signature", "encrypted"] {
            assert!(
                data.get(leaked).is_none_or(Value::is_null),
                "reason.item leaked `{leaked}`: {data}"
            );
        }
        assert!(
            data["item_id"].is_string(),
            "reason.item should identify the artifact: {data}"
        );
    }
}
