//! Same public suite used by built-in hosts, against actual workerd storage.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use everruns_durable_object::{AuthorityClient, ObjectHosting};
fn profile() -> ObjectHosting {
    ObjectHosting::new(
        AuthorityClient::new(
            "http://127.0.0.1:8789",
            "test",
            "authority-test-token-at-least-32-bytes",
        )
        .unwrap(),
    )
}
#[tokio::test]
#[ignore = "requires the local workerd authority on port 8789"]
async fn public_authority_contract() {
    everruns::hosting::conformance::authority(&profile()).await;
}
#[tokio::test]
#[ignore = "requires the local workerd authority on port 8789"]
async fn public_engine_wait_recovery() {
    everruns::hosting::conformance::engine_wait_recovery(profile()).await;
}

#[tokio::test]
#[ignore = "requires the local workerd authority on port 8789"]
async fn public_expired_owner() {
    everruns::hosting::conformance::lease_expiry(&profile()).await;
}

#[tokio::test]
#[ignore = "requires the local workerd authority on port 8789"]
async fn public_retained_deployment() {
    everruns::hosting::conformance::retained_build_recovery(profile()).await;
}
