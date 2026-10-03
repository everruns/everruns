#![allow(deprecated)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Compile and runtime checks for individual deprecated host feature forwards.

#[cfg(feature = "host-shell")]
#[test]
fn host_shell_keeps_its_filesystem_and_shell_catalog() {
    let registry = everruns_host::runtime_capability_registry();
    assert!(registry.has("host_shell"));
    assert!(registry.has("session_file_system"));
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn mcp_keeps_the_direct_egress_api_and_policy_enforcement() {
    use everruns_core::network_access::NetworkAccessList;
    use everruns_core::{EgressError, EgressRequest, EgressRequestKind};
    // The old feature implies direct-egress, including its public type export.
    let _transport = everruns_host::DirectEgressService::new();
    let request = EgressRequest::new("GET", "https://denied.invalid", EgressRequestKind::Mcp)
        .network_access(Some(NetworkAccessList::allow_only(["allowed.invalid"])));
    let error = everruns_host::runtime_egress_service()
        .send(request)
        .await
        .unwrap_err();
    assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
}
