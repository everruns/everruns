use super::*;
use crate::egress::{EgressError, EgressRequestKind, EgressSigning};
use crate::egress_transport::tests::MockEgress;
use crate::network_access::NetworkAccessList;

fn http_context(egress: Option<Arc<MockEgress>>) -> ToolContext {
    let (mut context, _) = create_context_with_mock_store();
    if let Some(egress) = egress {
        context.egress_service = Some(egress);
    }
    context
}

#[tokio::test]
async fn http_disabled_by_default_even_with_egress_available() {
    let egress = Arc::new(MockEgress::with_responses(vec![]));
    let context = http_context(Some(egress.clone()));
    let tool = BashTool::default();

    let result = tool
        .execute_with_context(
            json!({"commands": "curl -s http://93.184.216.34/ 2>&1; echo rc=$?"}),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(output) = result else {
        panic!("expected success result");
    };
    let combined = format!("{}{}", output["stdout"], output["stderr"]);
    assert!(
        !combined.contains("rc=0"),
        "curl must fail without enable_http, got: {combined}"
    );
    assert!(
        egress.requests.lock().unwrap().is_empty(),
        "no request may reach egress when HTTP is disabled"
    );
}

#[tokio::test]
async fn http_enable_without_egress_service_stays_offline() {
    let context = http_context(None);
    let tool = BashTool { enable_http: true };

    let result = tool
        .execute_with_context(
            json!({"commands": "curl -s http://93.184.216.34/ 2>&1; echo rc=$?"}),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(output) = result else {
        panic!("expected success result");
    };
    let combined = format!("{}{}", output["stdout"], output["stderr"]);
    assert!(
        !combined.contains("rc=0"),
        "curl must fail without an egress service, got: {combined}"
    );
}

#[tokio::test]
async fn curl_routes_through_egress_and_forwards_policy_metadata() {
    let egress = Arc::new(MockEgress::with_responses(vec![MockEgress::ok(
        200,
        &[("content-type", "text/plain")],
        "egress-ok",
    )]));
    let acl = NetworkAccessList::allow_only(["93.184.216.34"]);
    let mut context = http_context(Some(egress.clone()));
    context.network_access = Some(acl.clone());
    let tool = BashTool { enable_http: true };

    let result = tool
        .execute_with_context(
            json!({"commands": "curl -s http://93.184.216.34/data"}),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(output) = result else {
        panic!("expected success result");
    };
    assert_eq!(output["exit_code"], 0, "stderr: {}", output["stderr"]);
    assert!(
        output["stdout"].as_str().unwrap().contains("egress-ok"),
        "stdout: {}",
        output["stdout"]
    );

    assert_eq!(*egress.send_calls.lock().unwrap(), 0);
    assert_eq!(*egress.stream_calls.lock().unwrap(), 1);
    let requests = egress.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert_eq!(request.url, "http://93.184.216.34/data");
    assert_eq!(request.kind, EgressRequestKind::Capability);
    assert_eq!(request.signing, EgressSigning::PlatformDefault);
    assert_eq!(request.network_access, Some(acl));
    assert!(request.timeout_ms.is_some(), "deadline must be forwarded");
    // IP-literal host: bashkit's SSRF precheck pins the validated
    // address so the egress boundary can enforce resolve-then-check.
    let (host, addrs) = request.pinned_addrs.as_ref().expect("pinned addrs");
    assert_eq!(host, "93.184.216.34");
    assert_eq!(addrs[0].ip().to_string(), "93.184.216.34");
    assert_eq!(addrs[0].port(), 80);
    assert!(
        !request.dns_pinning_required,
        "pins already present; egress DNS re-resolve is unnecessary"
    );
}

/// EVE-1154: bashkit's DNS precheck fails open with empty pins when
/// lookup fails (`.invalid` never resolves). The egress transport must
/// require egress-side DNS pinning so a later private answer cannot
/// connect unpinned.
#[tokio::test]
async fn dns_precheck_failure_requires_egress_dns_pinning() {
    let url = "http://eve-1154-precheck-fail.invalid/";
    let egress = Arc::new(MockEgress::with_responses(vec![Err(
        EgressError::NetworkAccessDenied {
            url: url.to_string(),
        },
    )]));
    let context = http_context(Some(egress.clone()));
    let tool = BashTool { enable_http: true };

    let result = tool
        .execute_with_context(json!({"commands": format!("curl -s {url}")}), &context)
        .await;

    let ToolExecutionResult::Success(output) = result else {
        panic!("expected success result wrapper");
    };
    assert_eq!(
        *egress.stream_calls.lock().unwrap(),
        1,
        "transport must still be invoked after bashkit's fail-open precheck; stderr={}",
        output["stderr"]
    );
    let requests = egress.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, url);
    assert!(
        requests[0].pinned_addrs.is_none(),
        "DNS precheck failure must leave pins empty"
    );
    assert!(
        requests[0].dns_pinning_required,
        "empty bashkit pins must fail closed via egress DNS pinning"
    );
    assert_eq!(output["exit_code"], 7, "stderr: {}", output["stderr"]);
}

#[tokio::test]
async fn egress_denial_surfaces_as_curl_access_denied_exit_7() {
    let egress = Arc::new(MockEgress::with_responses(vec![Err(
        EgressError::NetworkAccessDenied {
            url: "http://93.184.216.34/blocked".to_string(),
        },
    )]));
    let context = http_context(Some(egress));
    let tool = BashTool { enable_http: true };

    let result = tool
        .execute_with_context(
            json!({"commands": "curl -s http://93.184.216.34/blocked"}),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(output) = result else {
        panic!("expected success result");
    };
    assert_eq!(output["exit_code"], 7, "stderr: {}", output["stderr"]);
    assert!(
        output["stderr"].as_str().unwrap().contains("access denied"),
        "stderr: {}",
        output["stderr"]
    );
    assert!(
        output["stderr"]
            .as_str()
            .unwrap()
            .contains("blocked by network policy"),
        "stderr: {}",
        output["stderr"]
    );
}

#[tokio::test]
async fn oversized_egress_response_surfaces_as_curl_exit_63() {
    // 11 MB body exceeds bashkit's 10 MB default cap; the transport
    // maps it to TooLarge before the interpreter sees the body.
    let big = "x".repeat(11 * 1024 * 1024);
    let egress = Arc::new(MockEgress::with_responses(vec![MockEgress::ok(
        200,
        &[("content-type", "text/plain")],
        &big,
    )]));
    let context = http_context(Some(egress.clone()));
    let tool = BashTool { enable_http: true };

    let result = tool
        .execute_with_context(
            json!({"commands": "curl -s http://93.184.216.34/huge"}),
            &context,
        )
        .await;

    let ToolExecutionResult::Success(output) = result else {
        panic!("expected success result");
    };
    assert_eq!(*egress.send_calls.lock().unwrap(), 0);
    assert_eq!(*egress.stream_calls.lock().unwrap(), 1);
    assert_eq!(output["exit_code"], 63, "stderr: {}", output["stderr"]);
    assert!(
        output["stderr"]
            .as_str()
            .unwrap()
            .contains("response too large"),
        "stderr: {}",
        output["stderr"]
    );
}

#[test]
fn validate_config_accepts_bool_and_rejects_other_types() {
    let cap = BashkitShellCapability;
    assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
    assert!(cap.validate_config(&json!({})).is_ok());
    assert!(cap.validate_config(&json!({"enable_http": true})).is_ok());
    assert!(cap.validate_config(&json!({"enable_http": "yes"})).is_err());
    assert!(cap.validate_config(&json!("nope")).is_err());
}

#[test]
fn config_schema_exposes_enable_http() {
    let schema = BashkitShellCapability.config_schema().unwrap();
    assert!(schema["properties"]["enable_http"]["type"] == "boolean");
}
