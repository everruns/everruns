use super::credentials::*;
use crate::credential_test_support::*;
use serde_json::{Value, json};
use std::sync::{Arc, atomic::Ordering};
fn arguments() -> Value {
    json!({"url":"https://auth.example/token","method":"POST","form":{"grant_type":"device_code"},"capture_secrets":[{"name":"ACCESS_TOKEN","from":"/access_token","scope":"session","binding":{"url":"https://auth.example/api","auth":{"type":"bearer"}}},{"name":"REFRESH_TOKEN","from":"/refresh_token","scope":"session","binding":{"url":"https://auth.example/token","auth":{"type":"form_field","name":"refresh_token"}}}]})
}
#[tokio::test]
async fn all_captures_and_encoded_echoes_are_sanitized_before_commit_or_result() {
    let svc = Arc::new(Credentials::default());
    let mut http = Http::json(
        r#"{"access_token":"access +/sentinel","refresh_token":"refresh-sentinel","echo":"access+%2B%2Fsentinel"}"#,
    );
    http.headers
        .insert("x-echo".into(), "refresh-sentinel".into());
    let http = Arc::new(http);
    let ctx = context(svc.clone(), http.clone());
    let result = execute(&arguments(), Some(&ctx), None).await;
    let visible = format!("{result:?}");
    for secret in [
        "access +/sentinel",
        "refresh-sentinel",
        "access+%2B%2Fsentinel",
    ] {
        assert!(!visible.contains(secret), "{visible}");
    }
    let commits = svc.commits.lock().unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].0.len(), 2);
    assert_eq!(commits[0].0[0].value, "access +/sentinel");
    assert!(!commits[0].1.to_string().contains("refresh-sentinel"));
    assert_eq!(http.seen.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn consent_sends_nothing_and_has_no_persistent_value_side_effect() {
    let svc = Arc::new(Credentials::default());
    svc.consent.store(true, Ordering::SeqCst);
    let http = Arc::new(Http::json("{}"));
    let ctx = context(svc.clone(), http.clone());
    let result = execute(&arguments(), Some(&ctx), None).await;
    let text = format!("{result:?}");
    assert!(text.contains("tool_approval_required"));
    assert!(text.contains("credential_consent"));
    assert!(http.seen.lock().unwrap().is_empty());
    assert!(svc.commits.lock().unwrap().is_empty());
}
#[tokio::test]
async fn invalid_or_incomplete_responses_never_partially_capture_or_expose_payload() {
    for body in [
        r#"{"access_token":"withheld-sentinel"}"#,
        r#"{"access_token":"withheld-sentinel","access_token":"other","refresh_token":"r"}"#,
        r#"{"access_token":42,"refresh_token":"withheld-sentinel"}"#,
        "<html>withheld-sentinel</html>",
    ] {
        let svc = Arc::new(Credentials::default());
        let http = Arc::new(Http::json(body));
        let ctx = context(svc.clone(), http);
        let result = execute(&arguments(), Some(&ctx), None).await;
        assert!(matches!(
            result,
            everruns_contracts::runtime::ToolExecutionResult::ToolError(_)
        ));
        assert!(!format!("{result:?}").contains("withheld-sentinel"));
        assert!(svc.commits.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn interrupted_oversize_and_failed_persistence_withhold_the_entire_response() {
    for case in 0..3 {
        let svc = Arc::new(Credentials::default());
        let mut http = Http::json(
            r#"{"access_token":"withheld-sentinel","refresh_token":"refresh-sentinel"}"#,
        );
        if case == 0 {
            http.interrupted = true;
        }
        if case == 1 {
            http.chunks.push(vec![b'x'; 256 * 1024]);
        }
        if case == 2 {
            svc.fail_commit.store(true, Ordering::SeqCst);
        }
        let result = execute(
            &arguments(),
            Some(&context(svc.clone(), Arc::new(http))),
            None,
        )
        .await;
        assert!(matches!(
            result,
            everruns_contracts::runtime::ToolExecutionResult::ToolError(_)
        ));
        assert!(!format!("{result:?}").contains("withheld-sentinel"));
        assert!(svc.commits.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn unsafe_placements_and_processing_modes_fail_before_network() {
    for patch in [
        json!({"save_to_file":"/tmp/leak"}),
        json!({"url":"https://auth.example/$TOKEN"}),
        json!({"json":{"token":"$TOKEN"}}),
        json!({"body":"$TOKEN"}),
    ] {
        let svc = Arc::new(Credentials::default());
        let http = Arc::new(Http::json("{}"));
        let ctx = context(svc, http.clone());
        let mut args = arguments();
        args.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let result = execute(&args, Some(&ctx), None).await;
        assert!(matches!(
            result,
            everruns_contracts::runtime::ToolExecutionResult::ToolError(_)
        ));
        assert!(http.seen.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn json_echoes_decode_unicode_escapes_before_redaction_without_capture() {
    use everruns_contracts::runtime::{
        SecretAuth, SecretBinding, credential_services::*, resource_access::ResourceScope,
    };
    let svc = Arc::new(Credentials::default());
    svc.values.lock().unwrap().push(ResolvedCredential {
        info: CredentialInfo {
            id: uuid::Uuid::now_v7(),
            revision: 1,
            name: "TOKEN".into(),
            binding: SecretBinding {
                url: "https://api.example/v1".into(),
                auth: SecretAuth::Bearer,
            },
            scope: ResourceScope::Agent,
            expires_at: None,
        },
        value: "secret-sentinel".into(),
    });
    let http = Arc::new(Http::json(r#"{"echo":"\u0073ecret-sentinel"}"#));
    let ctx = context(svc, http);
    let result = execute(
        &json!({"url":"https://api.example/v1","headers":{"Authorization":"Bearer $TOKEN"}}),
        Some(&ctx),
        None,
    )
    .await;
    let everruns_contracts::runtime::ToolExecutionResult::Success(value) = result else {
        panic!("{result:?}")
    };
    let body: Value = serde_json::from_str(value["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["echo"], "$TOKEN");
    assert!(!value.to_string().contains("sentinel"));
}
