//! Live Docker Engine API integration tests for the container-sandbox capability.
//!
//! Gated behind:
//! - Feature flag: `container-sandbox-live-tests`
//! - Daemon availability on `CONTAINER_SANDBOX_DOCKER_HOST` (which the workflows
//!   set explicitly to `http://localhost:2375`). The secure default host is the
//!   local unix socket, which this client cannot speak yet, so these live tests
//!   require the env var to point at an http(s):// daemon.
//!   The workflow `.github/workflows/container-sandbox-integration.yml` runs a
//!   `docker:dind` service with TLS disabled to provide this endpoint.
//!
//! Run locally (requires an unauthenticated Docker daemon on localhost:2375):
//!     docker run -d --privileged -p 2375:2375 -e DOCKER_TLS_CERTDIR='' docker:dind
//!     cargo test -p everruns-platform \
//!         --features container-sandbox-live-tests --test container_sandbox_live_api_test -- --test-threads=1
//!
//! The test is intentionally narrow: it proves the `DockerClient` speaks the
//! Docker Engine API end-to-end (TCP, HTTP, JSON, filter encoding) without
//! pulling images or depending on container runtime features that the repo's
//! unit tests already cover via wiremock.

#![cfg(feature = "container-sandbox-live-tests")]

use everruns_platform::container_sandbox::client::DockerClient;

/// Round-trip a scoped `list_containers` call against a real daemon.
///
/// Uses a label filter that cannot match anything on a fresh dind daemon, so
/// the assertion stays stable while still exercising the full HTTP + JSON
/// path through the client.
#[tokio::test(flavor = "multi_thread")]
async fn list_containers_round_trips_against_live_daemon() {
    let client = DockerClient::new(None);
    let probe = ("everruns-live-test", "does-not-exist");
    let containers = client
        .list_containers(&[probe])
        .await
        .expect("list_containers must round-trip against the live daemon");
    assert!(
        containers.is_empty(),
        "unexpected containers matched probe label {probe:?}: {containers:?}"
    );
}

/// Create and remove an isolated bridge network. Proves write-path plumbing
/// (POST /networks/create + DELETE /networks/{id}) without needing any image.
#[tokio::test(flavor = "multi_thread")]
async fn network_lifecycle_against_live_daemon() {
    let client = DockerClient::new(None);
    let name = format!(
        "everruns-live-net-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos()
    );
    let mut labels = std::collections::HashMap::new();
    labels.insert("everruns.live-test".to_string(), "1".to_string());

    let id = client
        .create_network(&name, labels)
        .await
        .expect("create_network must succeed against the live daemon");
    assert!(!id.is_empty(), "network id must not be empty");

    client
        .remove_network(&id)
        .await
        .expect("remove_network must succeed against the live daemon");
}
