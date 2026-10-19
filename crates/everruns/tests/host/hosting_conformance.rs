//! Keep built-in authorities honest against the downstream hosting contract.
use everruns::hosting::{MemoryHosting, conformance};
#[tokio::test]
async fn memory_authority_contract() {
    conformance::authority(&MemoryHosting::default()).await;
}
#[tokio::test]
async fn memory_engine_wait_recovery() {
    conformance::engine_wait_recovery(MemoryHosting::default()).await;
}
#[cfg(feature = "local")]
#[tokio::test]
async fn sqlite_authority_contract() {
    let dir = tempfile::tempdir().unwrap();
    let profile = everruns::hosting::SqliteHosting::open(dir.path().join("authority.db")).unwrap();
    conformance::authority(&profile).await;
}
#[cfg(feature = "local")]
#[tokio::test]
async fn sqlite_engine_wait_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let profile = everruns::hosting::SqliteHosting::open(dir.path().join("authority.db")).unwrap();
    conformance::engine_wait_recovery(profile).await;
}

#[tokio::test]
async fn memory_expired_owner() {
    conformance::lease_expiry(&MemoryHosting::default()).await;
}
#[cfg(feature = "local")]
#[tokio::test]
async fn sqlite_expired_owner() {
    let dir = tempfile::tempdir().unwrap();
    let profile = everruns::hosting::SqliteHosting::open(dir.path().join("authority.db")).unwrap();
    conformance::lease_expiry(&profile).await;
}

#[tokio::test]
async fn memory_retained_deployment() {
    conformance::retained_build_recovery(MemoryHosting::default()).await;
}
#[cfg(feature = "local")]
#[tokio::test]
async fn sqlite_retained_deployment() {
    let dir = tempfile::tempdir().unwrap();
    conformance::retained_build_recovery(
        everruns::hosting::SqliteHosting::open(dir.path().join("authority.db")).unwrap(),
    )
    .await;
}
