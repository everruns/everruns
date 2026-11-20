use super::*;
use serde_json::json;

#[test]
fn filesystem_typed_errors_win_over_conflicting_messages_and_wrappers() {
    for (error, expected) in [
        (
            FileSystemError::NotFound("readonly".into()),
            FileSystemErrorClass::NotFound,
        ),
        (
            FileSystemError::ReadOnly("not found".into()),
            FileSystemErrorClass::ReadOnly,
        ),
        (
            FileSystemError::IsADirectory("not empty".into()),
            FileSystemErrorClass::IsADirectory,
        ),
        (
            FileSystemError::NotADirectory("is a directory".into()),
            FileSystemErrorClass::NotADirectory,
        ),
        (
            FileSystemError::NotEmpty("not found".into()),
            FileSystemErrorClass::NotEmpty,
        ),
    ] {
        assert_eq!(classify_fs_error(&error), expected);
        let wrapped =
            AgentLoopError::Internal(anyhow::Error::new(error).context("readonly outer failure"));
        assert_eq!(classify_fs_error(&wrapped), expected);
    }
}

#[test]
fn filesystem_legacy_messages_preserve_routing_and_case_boundaries() {
    for (message, expected) in [
        (
            "Cannot modify readonly file: /a",
            FileSystemErrorClass::ReadOnly,
        ),
        (
            "Cannot delete readonly file: /a",
            FileSystemErrorClass::ReadOnly,
        ),
        (
            "write target is a directory: /a",
            FileSystemErrorClass::IsADirectory,
        ),
        (
            "Path is not a directory: /a",
            FileSystemErrorClass::NotADirectory,
        ),
        (
            "workspace root is not a directory: /a",
            FileSystemErrorClass::NotADirectory,
        ),
        ("Directory not found: /a", FileSystemErrorClass::NotFound),
        (
            "Directory is not empty. Use recursive=true to delete",
            FileSystemErrorClass::NotEmpty,
        ),
        (
            "Cannot delete root directory without recursive flag",
            FileSystemErrorClass::NotEmpty,
        ),
        (
            "recursive delete failed for /a: io",
            FileSystemErrorClass::NotEmpty,
        ),
        ("readonly file not found", FileSystemErrorClass::ReadOnly),
        ("file is read-only: /a", FileSystemErrorClass::Other),
        ("NOT FOUND", FileSystemErrorClass::Other),
        ("disk full", FileSystemErrorClass::Other),
    ] {
        assert_eq!(
            classify_fs_error(&AgentLoopError::store(message)),
            expected,
            "{message}"
        );
    }
}

#[test]
fn typed_request_and_model_errors_preserve_identity_and_safe_user_payload() {
    let context = || {
        UserFacingErrorContext::default()
            .with_provider("provider")
            .with_model_id("context-model")
            .with_retry_after(9)
    };
    let request = AgentLoopError::request_too_large("private payload");
    assert_eq!(request.to_string(), "Request too large: private payload");
    assert!(request.is_request_too_large());
    assert!(!request.is_model_not_available());
    assert_eq!(request.model_not_available_id(), None);
    assert_eq!(
        serde_json::to_value(request.user_facing_error(context())).unwrap(),
        json!({"code":"request_too_large","fields":{"provider":"provider","model_id":"context-model"}})
    );
    assert_eq!(
        request.user_facing_message(),
        "The conversation has become too long for the model to process. Please start a new session or reduce the context size."
    );
    let model = AgentLoopError::model_not_available("gpt-99")
        .with_provider("custom")
        .with_provider("custom");
    assert!(!model.is_request_too_large());
    assert!(model.is_model_not_available());
    assert_eq!(model.model_not_available_id(), Some("gpt-99"));
    assert_eq!(model.to_string(), "Model not available: gpt-99");
    assert_eq!(
        model.user_facing_message(),
        "The model `gpt-99` is not available. It may have been removed, renamed, or your API key may not have access to it. Please select a different model."
    );
    assert_eq!(
        serde_json::to_value(model.user_facing_error(context())).unwrap(),
        json!({"code":"model_unavailable","fields":{"provider":"provider","model_id":"gpt-99"}})
    );
    for other in [
        AgentLoopError::llm("Request too large: Model not available: gpt-99"),
        AgentLoopError::tool("failed"),
        AgentLoopError::Cancelled,
    ] {
        assert!(!other.is_request_too_large());
        assert!(!other.is_model_not_available());
        assert_eq!(other.model_not_available_id(), None);
    }
}

#[test]
fn semantic_kinds_override_conflicting_text_for_predicates_and_payloads() {
    for (kind, message, predicates, code) in [
        (
            LlmErrorKind::Authentication,
            "(429) rate limit (503)",
            (false, true, false, false),
            "provider_misconfigured",
        ),
        (
            LlmErrorKind::QuotaExhausted,
            "(401) (429) rate limit (503)",
            (false, false, false, false),
            "provider_quota_exhausted",
        ),
        (
            LlmErrorKind::RateLimited,
            "(401) (503) insufficient_quota",
            (true, false, false, true),
            "provider_rate_limited",
        ),
        (
            LlmErrorKind::Unavailable,
            "(401) (429) insufficient_quota",
            (false, false, true, true),
            "provider_unavailable",
        ),
        (
            LlmErrorKind::InvalidRequest,
            "opaque private failure",
            (false, false, false, false),
            "processing_error",
        ),
    ] {
        let error = AgentLoopError::llm_kind(kind, message);
        assert_eq!(error.llm_error_kind(), Some(kind));
        assert_eq!(
            (
                error.is_rate_limited(),
                error.is_auth_error(),
                error.is_server_error(),
                error.is_transient_llm_error()
            ),
            predicates,
            "{kind:?}"
        );
        let mut fields = json!({"provider":"provider","model_id":"model"});
        if kind == LlmErrorKind::RateLimited {
            fields["retry_after"] = json!(12);
        }
        assert_eq!(
            serde_json::to_value(
                error.user_facing_error(
                    UserFacingErrorContext::default()
                        .with_provider("provider")
                        .with_model_id("model")
                        .with_retry_after(12)
                )
            )
            .unwrap(),
            json!({"code":code,"fields":fields})
        );
    }
}

#[test]
fn legacy_predicates_and_user_copy_use_independent_literal_cases() {
    for (message, expected, copy) in [
        (
            "Anthropic API error (429): rate limit exceeded",
            (true, false, false),
            "Rate limited by the AI provider. Please wait a moment.",
        ),
        (
            "Rate limit exceeded (after 2 retries)",
            (true, false, false),
            "Rate limited by the AI provider. Please wait a moment.",
        ),
        (
            "too many requests",
            (true, false, false),
            "Rate limited by the AI provider. Please wait a moment.",
        ),
        (
            "Anthropic API error (401): invalid api key",
            (false, true, false),
            "The AI provider is not set up correctly. Ask an administrator to check the provider in Models.",
        ),
        (
            "OpenAI API error (403): forbidden",
            (false, true, false),
            "The AI provider is not set up correctly. Ask an administrator to check the provider in Models.",
        ),
        (
            "Anthropic API error (500): internal server error",
            (false, false, true),
            "The AI provider is experiencing issues. Please try again shortly.",
        ),
        (
            "OpenAI API error (503): service unavailable",
            (false, false, true),
            "The AI provider is experiencing issues. Please try again shortly.",
        ),
        (
            "Failed to send request: connection refused",
            (false, false, false),
            "I encountered an error while processing your request. Please try again later.",
        ),
    ] {
        let error = AgentLoopError::llm(message);
        assert_eq!(
            (
                error.is_rate_limited(),
                error.is_auth_error(),
                error.is_server_error()
            ),
            expected,
            "{message}"
        );
        assert_eq!(error.user_facing_message(), copy, "{message}");
    }
    for status in [502, 504, 529] {
        assert!(AgentLoopError::llm(format!("error ({status})")).is_server_error());
    }
    let non_llm = AgentLoopError::tool("(401) (429) (503) rate limit");
    assert_eq!(
        (
            non_llm.is_rate_limited(),
            non_llm.is_auth_error(),
            non_llm.is_server_error(),
            non_llm.is_transient_llm_error()
        ),
        (false, false, false, false)
    );
}

#[test]
fn provider_status_classification_covers_boundaries_and_quota_precedence() {
    for (status, expected) in [
        (200, LlmErrorKind::Other),
        (399, LlmErrorKind::Other),
        (400, LlmErrorKind::InvalidRequest),
        (401, LlmErrorKind::Authentication),
        (403, LlmErrorKind::Authentication),
        (404, LlmErrorKind::InvalidRequest),
        (408, LlmErrorKind::Unavailable),
        (409, LlmErrorKind::Unavailable),
        (429, LlmErrorKind::RateLimited),
        (499, LlmErrorKind::InvalidRequest),
        (500, LlmErrorKind::Unavailable),
        (501, LlmErrorKind::Other),
        (502, LlmErrorKind::Unavailable),
        (503, LlmErrorKind::Unavailable),
        (529, LlmErrorKind::Unavailable),
        (599, LlmErrorKind::Unavailable),
        (600, LlmErrorKind::Other),
    ] {
        assert_eq!(
            LlmErrorKind::from_provider_status(status, "opaque"),
            expected,
            "{status}"
        );
    }
    for message in [
        r#"{"error":{"type":"insufficient_quota"}}"#,
        r#"{"error":{"code":"credit_balance_exhausted"}}"#,
        r#"{"error":{"type":"usage_limit_reached"}}"#,
        "Your credit balance is too low to access the Anthropic API.",
    ] {
        for status in [400, 401, 429, 503] {
            assert_eq!(
                LlmErrorKind::from_provider_status(status, message),
                LlmErrorKind::QuotaExhausted,
                "{status}: {message}"
            );
        }
    }
}

/// The canonical OpenRouter refusal from EVE-952, verbatim off the wire.
const ATTESTATION_BODY: &str = r#"{"error":{"message":"This model requires you to complete the following before use: 18+ age confirmation. Confirm at https://openrouter.ai/settings/preferences.","code":403,"metadata":{"missing_attestation_types":["age_18plus"],"routing_funnel":[{"step":"Initial Endpoints","endpoint_count":1}],"failed_routing_step":"Gate Endpoints with Attestations"}}}"#;

#[test]
fn attestation_gate_is_classified_apart_from_other_403s() {
    assert_eq!(
        LlmErrorKind::from_provider_status(403, ATTESTATION_BODY),
        LlmErrorKind::AttestationRequired
    );
    // Status is not the signal: the same body under another status still
    // names the gate, and a 403 without one stays an auth failure.
    assert_eq!(
        LlmErrorKind::from_provider_status(429, ATTESTATION_BODY),
        LlmErrorKind::AttestationRequired
    );
    for body in [
        r#"{"error":{"message":"Invalid credentials","code":403}}"#,
        r#"{"error":{"message":"Insufficient credits","code":403,"metadata":{"routing_funnel":[]}}}"#,
        "opaque",
    ] {
        assert_ne!(
            LlmErrorKind::from_provider_status(403, body),
            LlmErrorKind::AttestationRequired,
            "{body}"
        );
    }
    // Exhausted billing keeps precedence over the gate check.
    assert_eq!(
        LlmErrorKind::from_provider_status(
            403,
            r#"{"error":{"message":"insufficient_quota; requires you to complete the following before use"}}"#
        ),
        LlmErrorKind::QuotaExhausted
    );
}

#[test]
fn attestation_gate_reaches_the_reader_with_the_types_and_the_confirm_url() {
    let error = AgentLoopError::llm_kind(
        LlmErrorKind::AttestationRequired,
        format!("OpenAI Responses API error (403): {ATTESTATION_BODY}"),
    )
    .with_provider("openrouter");
    // Not a credential problem and never worth retrying.
    assert!(!error.is_auth_error());
    assert!(!error.is_transient_llm_error());
    assert_eq!(
        serde_json::to_value(
            error.user_facing_error(
                UserFacingErrorContext::default()
                    .with_provider("openrouter")
                    .with_model_id("meta/muse-spark-1.3-contributor")
            )
        )
        .unwrap(),
        json!({
            "code": "provider_attestation_required",
            "fields": {
                "provider": "openrouter",
                "model_id": "meta/muse-spark-1.3-contributor",
                "missing_types": ["age_18plus"],
                "confirm_url": "https://openrouter.ai/settings/preferences",
            }
        })
    );
    assert_eq!(
        error.user_facing_message(),
        "The AI provider account has not completed a confirmation this model requires (age_18plus). Complete it at https://openrouter.ai/settings/preferences, then try again."
    );
}

#[test]
fn untyped_attestation_bodies_still_route_off_the_403_misconfiguration_copy() {
    // Legacy/untyped errors reach the string classifier instead; it must
    // reach the same code rather than "contact support".
    let error = AgentLoopError::llm(format!(
        "provider 'openrouter': OpenAI Responses API error (403): {ATTESTATION_BODY}"
    ));
    assert_eq!(
        error
            .user_facing_error(UserFacingErrorContext::default())
            .code,
        "provider_attestation_required"
    );
}

#[test]
fn attestation_parsing_covers_multiple_types_escaped_bodies_and_a_missing_url() {
    let requirement = |body: &str| {
        parse_attestation_requirement(body).unwrap_or_else(|| panic!("no gate in {body}"))
    };

    // Multiple gates, in payload order.
    let multiple = requirement(
        r#"{"error":{"message":"This model requires you to complete the following before use: 18+ age confirmation and identity verification. Confirm at https://openrouter.ai/settings/preferences.","metadata":{"missing_attestation_types":["age_18plus","identity_verified"]}}}"#,
    );
    assert_eq!(multiple.missing_types, ["age_18plus", "identity_verified"]);
    assert_eq!(
        multiple.confirm_url,
        "https://openrouter.ai/settings/preferences"
    );

    // JSON-escaped body (a provider error nested in another envelope).
    let escaped = requirement(
        r#"{"detail":"{\"error\":{\"message\":\"This model requires you to complete the following before use: 18+ age confirmation. Confirm at https:\/\/openrouter.ai\/settings\/gates.\",\"metadata\":{\"missing_attestation_types\":[\"age_18plus\"]}}}"}"#,
    );
    assert_eq!(escaped.missing_types, ["age_18plus"]);
    assert_eq!(escaped.confirm_url, "https://openrouter.ai/settings/gates");

    // No URL in the message: fall back rather than leave the reader with
    // nowhere to go.
    let no_url = requirement(
        r#"{"error":{"message":"This model requires you to complete the following before use: 18+ age confirmation.","metadata":{"missing_attestation_types":["age_18plus"]}}}"#,
    );
    assert_eq!(
        no_url.confirm_url,
        "https://openrouter.ai/settings/preferences"
    );

    // The gate sentence alone is enough; the metadata block is optional.
    let sentence_only = requirement(
        "This model requires you to complete the following before use: 18+ age confirmation. Confirm at https://openrouter.ai/settings/preferences",
    );
    assert!(sentence_only.missing_types.is_empty());
    assert_eq!(
        sentence_only.confirm_url,
        "https://openrouter.ai/settings/preferences"
    );
    // With nothing parsed, the message drops the list rather than
    // rendering an empty one.
    assert_eq!(
        AgentLoopError::llm_kind(
            LlmErrorKind::AttestationRequired,
            "This model requires you to complete the following before use: a confirmation."
        )
        .user_facing_message(),
        "The AI provider account has not completed a confirmation this model requires. Complete it at https://openrouter.ai/settings/preferences, then try again."
    );

    // A URL in the driver's own prefix is not mistaken for the gate page.
    assert_eq!(
        requirement(&format!(
            "POST https://openrouter.ai/api/v1/responses failed: {ATTESTATION_BODY}"
        ))
        .confirm_url,
        "https://openrouter.ai/settings/preferences"
    );

    for body in [
        r#"{"error":{"message":"Invalid credentials"}}"#,
        r#"{"error":{"metadata":{"missing_attestation_types":[]}}}"#,
        "",
    ] {
        assert!(parse_attestation_requirement(body).is_none(), "{body}");
    }
}

#[test]
fn a_hostile_attestation_payload_cannot_choose_how_much_reaches_the_viewer() {
    let types = (0..40)
        .map(|index| format!(r#""gate_{index}""#))
        .collect::<Vec<_>>()
        .join(",");
    let long_type = "x".repeat(65);
    let long_url = format!("https://evil.example/{}", "a".repeat(400));
    let requirement = parse_attestation_requirement(&format!(
        r#"{{"error":{{"message":"This model requires you to complete the following before use: gates. Confirm at {long_url}","metadata":{{"missing_attestation_types":["{long_type}",{types}]}}}}}}"#
    ))
    .expect("gate recognized");

    // Over-long entries are dropped, not truncated, and the list is capped.
    assert_eq!(requirement.missing_types.len(), 8);
    assert_eq!(requirement.missing_types[0], "gate_0");
    // An over-long URL falls back rather than shipping a 400-char link.
    assert_eq!(
        requirement.confirm_url,
        "https://openrouter.ai/settings/preferences"
    );

    // Non-http(s) schemes never become the confirmation link.
    for scheme in [
        "javascript:alert(1)",
        "data:text/html,<script>",
        "file:///etc/passwd",
    ] {
        assert_eq!(
            parse_attestation_requirement(&format!(
                "This model requires you to complete the following before use: a gate. Confirm at {scheme}"
            ))
            .expect("gate recognized")
            .confirm_url,
            "https://openrouter.ai/settings/preferences",
            "{scheme}"
        );
    }
}

#[test]
fn provider_text_classification_uses_independent_keywords_and_precedence() {
    for (message, expected) in [
        ("ThrottlingException", LlmErrorKind::RateLimited),
        ("TooManyRequestsException", LlmErrorKind::RateLimited),
        ("RATE LIMIT", LlmErrorKind::RateLimited),
        ("too many requests", LlmErrorKind::RateLimited),
        ("AccessDeniedException", LlmErrorKind::Authentication),
        ("UnrecognizedClientException", LlmErrorKind::Authentication),
        ("ExpiredTokenException", LlmErrorKind::Authentication),
        ("InvalidSignatureException", LlmErrorKind::Authentication),
        ("unauthorized", LlmErrorKind::Authentication),
        ("ServiceUnavailableException", LlmErrorKind::Unavailable),
        ("service unavailable", LlmErrorKind::Unavailable),
        ("InternalServerException", LlmErrorKind::Unavailable),
        ("ModelNotReadyException", LlmErrorKind::Unavailable),
        (
            "usage_limit_reached; resets_at=1783767823; throttlingexception",
            LlmErrorKind::QuotaExhausted,
        ),
        ("something else entirely", LlmErrorKind::Other),
    ] {
        assert_eq!(
            LlmErrorKind::from_error_text(message),
            expected,
            "{message}"
        );
    }
}

#[test]
fn provider_prefix_preserves_kind_and_retry_metadata_without_duplication() {
    let metadata = crate::llm_retry::RetryMetadata {
        attempts: 2,
        total_retry_wait: std::time::Duration::from_millis(1234),
        ..Default::default()
    };
    let error = AgentLoopError::llm_kind(LlmErrorKind::Unavailable, "network failure")
        .with_retry_metadata(&metadata)
        .with_provider("custom")
        .with_provider("custom");
    assert_eq!(error.llm_retry_attempts(), 2);
    assert!(error.llm_retry_handled());
    let AgentLoopError::Llm(error) = error else {
        panic!("lost LLM variant")
    };
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        json!({"kind":"unavailable","message":"provider 'custom': network failure","retry_attempts":2,"retry_wait_ms":1234,"retry_handled":true})
    );
    let legacy: LlmError =
        serde_json::from_value(json!({"kind":"other","message":"legacy"})).unwrap();
    assert_eq!(
        serde_json::to_value(legacy).unwrap(),
        json!({"kind":"other","message":"legacy","retry_attempts":0,"retry_wait_ms":0,"retry_handled":false})
    );
    let non_llm = AgentLoopError::Cancelled
        .with_retry_metadata(&metadata)
        .with_provider("custom");
    assert!(matches!(non_llm, AgentLoopError::Cancelled));
    assert_eq!(non_llm.llm_retry_attempts(), 0);
    assert!(!non_llm.llm_retry_handled());
}

#[test]
fn billing_pressure_preserves_typed_payload_and_safe_user_fields() {
    let kind = LlmErrorKind::BillingPressure {
        reason: BillingPressureReason::InFlightBudgetExhausted,
        retry_after_secs: Some(120),
    };
    let error = AgentLoopError::llm_kind(kind, "private provider body");
    assert_eq!(error.llm_error_kind(), Some(kind));
    assert!(!error.is_transient_llm_error());
    assert_eq!(
        serde_json::to_value(
            error.user_facing_error(
                UserFacingErrorContext::default()
                    .with_provider("openrouter")
                    .with_model_id("vendor/model")
            )
        )
        .unwrap(),
        json!({
            "code": "provider_rate_limited",
            "fields": {
                "provider": "openrouter",
                "model_id": "vendor/model",
                "reason": "in_flight_budget_exhausted",
                "retry_after": 120,
            }
        })
    );
    let AgentLoopError::Llm(error) = error else {
        panic!("lost LLM variant")
    };
    assert_eq!(
        serde_json::to_value(error.kind).unwrap(),
        json!({
            "billing_pressure": {
                "reason": "in_flight_budget_exhausted",
                "retry_after_secs": 120,
            }
        })
    );

    let exhausted = AgentLoopError::llm_kind(
        LlmErrorKind::BillingPressure {
            reason: BillingPressureReason::InsufficientCredits,
            retry_after_secs: None,
        },
        "private provider body",
    );
    assert_eq!(
        exhausted
            .user_facing_error(UserFacingErrorContext::default())
            .code,
        "provider_quota_exhausted"
    );
}

#[test]
fn missing_model_and_iteration_limits_have_complete_safe_payloads() {
    let missing = AgentLoopError::model_not_configured();
    assert!(missing.is_non_retryable());
    assert_eq!(
        missing.user_facing_message(),
        "No model is configured for this chat. Choose a model or configure a default model, then try again."
    );
    assert_eq!(
        serde_json::to_value(missing.user_facing_error(UserFacingErrorContext::default())).unwrap(),
        json!({"code":"model_not_configured"})
    );
    assert_eq!(
        serde_json::to_value(
            AgentLoopError::MaxIterationsReached(7)
                .user_facing_error(UserFacingErrorContext::default())
        )
        .unwrap(),
        json!({"code":"max_iterations","fields":{"max_iterations":7}})
    );
}

#[test]
fn store_adapter_preserves_success_and_exact_error_variant_and_message() {
    let success: std::result::Result<Vec<String>, String> =
        Ok(vec!["first".into(), "second".into()]);
    assert_eq!(success.store_err().unwrap(), ["first", "second"]);
    let failure: std::result::Result<(), std::io::Error> =
        Err(std::io::Error::other("db unavailable"));
    let error = failure.store_err().unwrap_err();
    assert_eq!(error.to_string(), "Message store error: db unavailable");
    assert!(matches!(error,AgentLoopError::MessageStore(message) if message=="db unavailable"));
}

#[test]
fn json_helpers_preserve_structures_and_apply_documented_error_defaults() {
    assert_eq!(json_val(&vec![1, 2, 3]), json!([1, 2, 3]));
    assert_eq!(from_json::<Vec<String>>(json!(["a", "b"])), ["a", "b"]);
    assert_eq!(from_json::<i32>(json!("not a number")), 0);
    struct Fails;
    impl Serialize for Fails {
        fn serialize<S: serde::Serializer>(&self, _: S) -> std::result::Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("synthetic serialization failure"))
        }
    }
    assert_eq!(json_val(&Fails), serde_json::Value::Null);
}

#[test]
fn llm_http_records_the_status_and_provider_code() {
    let body =
        r#"{"error":{"message":"no","code":"model_not_found","type":"invalid_request_error"}}"#;
    let error = AgentLoopError::llm_http(404, body, "OpenAI API error (404)");
    assert_eq!(error.http_status(), Some(404));
    assert_eq!(error.provider_error_code(), Some("model_not_found"));
}

#[test]
fn llm_http_falls_back_to_the_anthropic_error_type() {
    let body = r#"{"error":{"type":"overloaded_error","message":"busy"}}"#;
    let error = AgentLoopError::llm_http(529, body, "Anthropic API error (529)");
    assert_eq!(error.provider_error_code(), Some("overloaded_error"));
    assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::Unavailable));
}

#[test]
fn a_provider_sized_error_body_is_not_parsed_and_a_long_code_is_not_kept() {
    // TM-DOS-038: the body is provider-controlled and nothing upstream
    // bounds it, so neither the parse nor the retained string may be
    // sized by the provider.
    let padding = "x".repeat(70 * 1024);
    let huge = format!(r#"{{"error":{{"code":"rate_limit","pad":"{padding}"}}}}"#);
    let error = AgentLoopError::llm_http(429, &huge, "provider refused");
    assert_eq!(error.provider_error_code(), None);
    // The status still classifies it, so nothing is lost but the code.
    assert!(error.is_rate_limited());

    let long_code = "c".repeat(200);
    let body = format!(r#"{{"error":{{"code":"{long_code}"}}}}"#);
    assert_eq!(
        AgentLoopError::llm_http(400, &body, "boom").provider_error_code(),
        None,
        "a code longer than any real one is not a code"
    );
}

#[test]
fn a_non_json_body_yields_no_provider_code() {
    let error = AgentLoopError::llm_http(503, "upstream is down", "boom");
    assert_eq!(error.http_status(), Some(503));
    assert_eq!(error.provider_error_code(), None);
}

#[test]
fn semantic_variants_report_the_status_they_stand_for() {
    assert_eq!(
        AgentLoopError::model_not_available("gpt-5").http_status(),
        Some(404)
    );
    assert_eq!(
        AgentLoopError::request_too_large("too big").http_status(),
        Some(413)
    );
    assert_eq!(AgentLoopError::Cancelled.http_status(), None);
}

#[test]
fn a_recorded_status_classifies_an_untyped_failure() {
    // The message carries no "(429)" marker, so only the recorded status
    // can answer this — the case that forced embedders to scrape strings.
    let error = AgentLoopError::llm_http(429, "slow down", "provider refused the request");
    assert!(error.is_rate_limited());
    assert_eq!(error.retry_after_secs(), None);
    assert!(!AgentLoopError::llm("provider refused the request").is_rate_limited());
}

#[test]
fn retry_after_prefers_the_recorded_delay_over_the_billing_hint() {
    let error = AgentLoopError::llm_kind(
        LlmErrorKind::BillingPressure {
            reason: BillingPressureReason::InFlightBudgetExhausted,
            retry_after_secs: Some(30),
        },
        "budget",
    );
    assert_eq!(error.retry_after_secs(), Some(30));
    assert_eq!(error.with_retry_after_secs(5).retry_after_secs(), Some(5));
}

#[test]
fn transport_fields_survive_a_serde_round_trip_and_default_when_absent() {
    let error = LlmError::new(LlmErrorKind::RateLimited, "slow down")
        .with_status(429)
        .with_code("rate_limit_exceeded")
        .with_retry_after_secs(12);
    let json = serde_json::to_value(&error).unwrap();
    let back: LlmError = serde_json::from_value(json).unwrap();
    assert_eq!(back.status, Some(429));
    assert_eq!(back.code.as_deref(), Some("rate_limit_exceeded"));
    assert_eq!(back.retry_after_secs, Some(12));

    let legacy: LlmError = serde_json::from_str(r#"{"kind":"other","message":"legacy"}"#).unwrap();
    assert_eq!(legacy.status, None);
    assert_eq!(legacy.code, None);
}

#[test]
fn a_malformed_response_is_classified_and_never_retried() {
    let error = AgentLoopError::llm_kind(
        LlmErrorKind::MalformedResponse,
        "stream ended before its terminal event",
    );
    assert!(!error.is_transient_llm_error());
    assert!(!error.is_rate_limited());
}
