use super::*;
use crate::driver_registry::LlmCompletionMetadata;

fn response(reason: Option<&str>, text: &str) -> LlmResponse {
    let mut metadata = LlmCompletionMetadata::default();
    metadata.finish_reason = reason.map(str::to_owned);
    LlmResponse {
        text: text.into(),
        reasoning: vec![],
        tool_calls: None,
        metadata,
    }
}

#[test]
fn all_output_limit_spellings_reject_installation_and_allow_only_a_bounded_retry() {
    let mut config = LlmCallConfig::new("extension-model");
    config.max_tokens = Some(512);
    for reason in [
        "length",
        "max_tokens",
        "max_output_tokens",
        "MAX_TOKENS",
        "MAX_OUTPUT_TOKENS",
    ] {
        let response = response(Some(reason), "Partial continuity");
        assert!(ValidatedSummary::new(&response).is_none(), "{reason}");
        let retry = summary_retry_config(&config, &response, 4096, 1024, 2048).expect("retry fits");
        assert!(retry.max_tokens.unwrap() > 512);
        assert!(retry.max_tokens.unwrap() <= 3072);
        assert!(
            summary_retry_config(&retry, &response, 4096, 1024, 2048).is_none(),
            "no unbounded retry"
        );
    }
}

#[test]
fn unknown_refused_and_missing_endings_never_install_a_summary() {
    let config = LlmCallConfig::new("extension-model");
    for reason in [
        None,
        Some("future-provider-reason"),
        Some("safety"),
        Some("cancelled"),
        Some("error"),
    ] {
        let response = response(reason, "Unverified notes");
        assert!(ValidatedSummary::new(&response).is_none());
        assert!(summary_retry_config(&config, &response, 4096, 100, 2048).is_none());
    }
    for reason in ["stop", "end_turn", "END_TURN", "completed"] {
        let response = response(Some(reason), "Verified complete notes");
        assert_eq!(
            ValidatedSummary::new(&response).unwrap().text(),
            "Verified complete notes"
        );
    }
}

#[test]
fn refusal_with_hidden_reasoning_does_not_trigger_a_paid_retry() {
    let mut response = response(Some("safety"), "");
    response.metadata.reasoning_tokens = Some(512);
    assert!(!summary_ran_out_of_output(&response));
}

#[test]
fn raw_unknown_or_paused_endings_cannot_install_or_trigger_a_paid_retry() {
    for raw in ["pause_turn", "future-provider-ending", "requires_action"] {
        let mut response = response(Some("stop"), "Unverified notes");
        response.metadata.provider_finish_reason = Some(raw.into());
        assert!(ValidatedSummary::new(&response).is_none(), "{raw}");
        response.text.clear();
        response.metadata.reasoning_tokens = Some(512);
        assert!(!summary_ran_out_of_output(&response), "{raw}");
    }
}

#[test]
fn an_inferred_stop_cannot_hide_raw_truncation_or_unaccounted_tool_calls() {
    let mut response = response(Some("stop"), "Partial notes");
    response.metadata.provider_finish_reason = Some("MAX_OUTPUT_TOKENS".into());
    assert!(ValidatedSummary::new(&response).is_none());
    assert!(summary_ran_out_of_output(&response));
    response.metadata.provider_finish_reason = None;
    response.metadata.tool_calls_truncated_executed = 1;
    assert!(ValidatedSummary::new(&response).is_none());
}
