//! Shared fixtures for the workflow tests: base URLs, teardown helpers, and
//! the provider-account skip reporting that keeps these tests green when a
//! live provider is out of quota.

use serde_json::{Value, json};

pub(crate) const SERVER_BASE_URL: &str = "http://localhost:9000";
pub(crate) const API_BASE_URL: &str = "http://localhost:9000/api";
// Note: With AUTH_MODE=none, org is derived from the anonymous user's default org.
// No cookie or header needed for integration tests.

/// Built-in harness name — resolved per-org at runtime (UUIDs are DB-assigned
/// and no longer stable). Passed via `harness_name` on session creation.
pub(crate) const SEED_HARNESS_NAME: &str = "base";

// === Teardown =============================================================
//
// `client.delete(..).send().await.expect("...")` only asserts the request was
// *sent*. A refused delete answers 4xx/5xx and is still `Ok`, so teardown
// written that way reports success while leaving rows behind in the shared
// test database. Models, providers, agents and sessions accumulated that way
// for weeks before anyone noticed, because nothing ever failed (EVE-955).
//
// These helpers assert the server actually did the work, and release the
// references that would otherwise make a delete impossible to satisfy.

/// Send a teardown `DELETE` and assert the server accepted it.
///
/// `404` passes: teardown is idempotent and the row may already be gone.
pub(crate) async fn cleanup_delete(client: &reqwest::Client, url: String, what: &str) {
    let response = client
        .delete(&url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("cleanup: DELETE {url} ({what}) could not be sent: {e}"));
    let status = response.status();
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return;
    }
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<unreadable body>".to_string());
    panic!("cleanup: DELETE {url} ({what}) returned {status}: {body}");
}

/// Remove an agent for real.
///
/// `DELETE /v1/agents/{id}` only *archives* it: the row survives and keeps its
/// `default_model_id`, so a later model delete is refused on
/// `agents_default_model_id_fkey`. `POST /v1/agents/{id}/delete` is what
/// actually removes the row, and it requires the archive first.
pub(crate) async fn cleanup_agent(
    client: &reqwest::Client,
    agent_public_id: impl std::fmt::Display,
) {
    cleanup_delete(
        client,
        format!("{}/v1/agents/{}", API_BASE_URL, agent_public_id),
        "agent",
    )
    .await;

    let url = format!("{}/v1/agents/{}/delete", API_BASE_URL, agent_public_id);
    let response =
        client.post(&url).send().await.unwrap_or_else(|e| {
            panic!("cleanup: POST {url} (destroy agent) could not be sent: {e}")
        });
    let status = response.status();
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return;
    }
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<unreadable body>".to_string());
    panic!("cleanup: POST {url} (destroy agent) returned {status}: {body}");
}

/// Live Anthropic model ids used by the tests in this file that talk to the
/// real provider.
///
/// Pinned to explicit dated ids rather than `-latest` aliases so a failure
/// names the model it actually asked for. Anthropic retires models, and a
/// retired id comes back as `model_unavailable` rather than a test-logic
/// failure — when that happens, re-point these two constants at current ids
/// (`GET /v1/models`) instead of editing each test.
pub(crate) const LIVE_ANTHROPIC_FAST_MODEL: &str = "claude-haiku-4-5-20251001";
/// Extended thinking requires a model that supports it; Haiku does not.
pub(crate) const LIVE_ANTHROPIC_THINKING_MODEL: &str = "claude-sonnet-4-5-20250929";

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
pub(crate) fn provider_account_block(message: &Value) -> Option<&str> {
    use everruns_provider::user_facing_error::codes;

    let code = message["metadata"]["error_code"].as_str()?;
    (code == codes::PROVIDER_QUOTA_EXHAUSTED || code == codes::PROVIDER_USAGE_LIMIT_REACHED)
        .then_some(code)
}

#[test]
pub(crate) fn provider_account_block_matches_only_billing_codes() {
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
pub(crate) fn messages_provider_account_block(body: &Value) -> Option<&str> {
    body["data"]
        .as_array()?
        .iter()
        .find_map(provider_account_block)
}

#[test]
pub(crate) fn messages_provider_account_block_reads_a_real_quota_response() {
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
pub(crate) async fn session_provider_account_block(
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
pub(crate) fn provider_account_skip_line(test: &str, code: &str) -> String {
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
pub(crate) fn report_provider_account_skip(code: &str) {
    let test = std::thread::current()
        .name()
        .unwrap_or("unknown test")
        .to_owned();
    println!("SKIP: {test}: live provider account unavailable ({code})");

    // Writes one file; the `Report skipped live tests` step in ci.yml renders it
    // into both the job log and the step summary. Deliberately not written
    // straight to GITHUB_STEP_SUMMARY: only the log is readable back over the
    // Actions API, which is the channel a scheduled maintenance session has —
    // the same reasoning security-alerts.yml states for teeing its report. A
    // skip only a human can see is how OpenAI coverage stayed dark for days
    // (EVE-943). Routing through the workflow also keeps the rendering in one
    // place instead of appending to the summary from two directions.
    if let Ok(path) = std::env::var("EVERRUNS_LIVE_SKIP_REPORT") {
        append_skip_to_report(std::path::Path::new(&path), &test, code);
    }
}

/// Appends one skip line to the report file, swallowing every I/O error.
///
/// Split out so the append is covered by a test rather than only ever running
/// inside Actions.
pub(crate) fn append_skip_to_report(path: &std::path::Path, test: &str, code: &str) {
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
pub(crate) fn append_skip_to_report_accumulates_and_never_panics() {
    let dir = std::env::temp_dir().join(format!("everruns-skip-report-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("live-provider-skips.md");

    // Appends rather than truncating: several tests can skip in one run and the
    // report must list every one of them.
    append_skip_to_report(&path, "test_one", "provider_quota_exhausted");
    append_skip_to_report(&path, "test_two", "provider_usage_limit_reached");
    let written = std::fs::read_to_string(&path).expect("report written");
    assert_eq!(written.lines().count(), 2, "{written}");
    assert!(written.contains("test_one"), "{written}");
    assert!(written.contains("test_two"), "{written}");

    // An unwritable path is silently ignored — a skip must never become a
    // failure because the report path changed or was never created.
    append_skip_to_report(
        &dir.join("no-such-directory").join("live-provider-skips.md"),
        "test_three",
        "provider_quota_exhausted",
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Returns the same preview the live tests print, so the truncation is covered
/// without needing a provider.
pub(crate) fn response_preview(text: &str) -> String {
    text.chars().take(100).collect::<String>()
}

#[test]
pub(crate) fn response_preview_truncates_on_char_boundaries() {
    // A byte slice at 100 panics here: the model's curly apostrophe occupies
    // bytes 98..101, so index 100 lands inside it. Live OpenAI output containing
    // one took main red once the account had credits again and these tests
    // resumed running.
    let text = format!("{}\u{2019}s trailing text", "a".repeat(98));
    assert!(!text.is_char_boundary(100));

    let preview = response_preview(&text);
    assert_eq!(preview.chars().count(), 100);
    assert!(preview.starts_with(&"a".repeat(98)));

    // Shorter-than-limit and empty input must pass through unchanged.
    assert_eq!(response_preview("short"), "short");
    assert_eq!(response_preview(""), "");
}

#[test]
pub(crate) fn provider_account_skip_line_names_the_test_and_code() {
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
