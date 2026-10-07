//! Audit logs are served by the generic `#[command(http = ..)]` handler. These
//! pin the shape the hand-written handler had (`{"data": [...]}`) and that
//! query parameters still arrive typed.

use crate::test_harness::TestServer;
use axum::http::StatusCode;
use serde_json::Value;

const AUDIT_LOGS: &str = "/v1/orgs/org_00000000000000000000000000000001/audit-logs";

#[tokio::test]
async fn audit_log_route_keeps_its_response_shape() {
    let server = TestServer::new().await;

    let listed: Value = server
        .get(&format!("{AUDIT_LOGS}?limit=5&domain=management"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(listed["data"].is_array(), "{listed}");

    server
        .get(&format!("{AUDIT_LOGS}?limit=many"))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}
