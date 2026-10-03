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

#![cfg(feature = "container-sandbox-live-tests")]

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use everruns_core::capabilities::Capability;
use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_platform::container_sandbox::ContainerSandboxCapability;
use everruns_platform::container_sandbox::client::{
    ContainerCreateConfig, DockerClient, HostConfig,
};
use everruns_platform::container_sandbox::state::{
    CONTAINER_SANDBOX_SECRET_PREFIX, SandboxState, container_name, sandbox_labels,
};
use serde_json::json;
use tokio::sync::Mutex;

#[derive(Default)]
struct MemoryStorage {
    secrets: Mutex<HashMap<String, String>>,
}

impl MemoryStorage {
    fn key(session_id: SessionId, name: &str) -> String {
        format!("{session_id}:{name}")
    }
}

#[async_trait]
impl SessionStorageStore for MemoryStorage {
    async fn set_value(&self, _session_id: SessionId, _key: &str, _value: &str) -> Result<()> {
        Ok(())
    }
    async fn get_value(&self, _session_id: SessionId, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn delete_value(&self, _session_id: SessionId, _key: &str) -> Result<bool> {
        Ok(false)
    }
    async fn list_keys(&self, _session_id: SessionId) -> Result<Vec<KeyInfo>> {
        Ok(vec![])
    }
    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        self.secrets
            .lock()
            .await
            .insert(Self::key(session_id, name), value.to_string());
        Ok(())
    }
    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        Ok(self
            .secrets
            .lock()
            .await
            .get(&Self::key(session_id, name))
            .cloned())
    }
    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        Ok(self
            .secrets
            .lock()
            .await
            .remove(&Self::key(session_id, name))
            .is_some())
    }
    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        let prefix = format!("{session_id}:");
        let now = chrono::Utc::now();
        Ok(self
            .secrets
            .lock()
            .await
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .map(|name| SecretInfo {
                name: name.to_string(),
                created_at: now,
                updated_at: now,
            })
            .collect())
    }
}

fn tool_named(cap: &ContainerSandboxCapability, name: &str) -> Box<dyn Tool> {
    cap.tools()
        .into_iter()
        .find(|tool| tool.name() == name)
        .unwrap_or_else(|| panic!("missing tool {name}"))
}

fn assert_tool_error_contains(result: ToolExecutionResult, needle: &str) {
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains(needle),
                "expected tool error containing {needle:?}, got {msg:?}"
            );
        }
        other => panic!("expected ToolError containing {needle:?}, got {other:?}"),
    }
}

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

/// EVE-1151: forging `container_sandbox:` state to another container's ID must
/// not let a session read, write, exec, stop, or remove that foreign object.
/// The session's own sandbox remains usable after the forged state is restored.
#[tokio::test(flavor = "multi_thread")]
async fn forged_sandbox_state_cannot_redirect_docker_ops() {
    let client = DockerClient::new(None);
    let image = "alpine:3.20";
    client
        .pull_image(image)
        .await
        .expect("pull alpine for live forge test");

    let session_id = SessionId::new();
    let storage = Arc::new(MemoryStorage::default());
    let context = ToolContext::with_storage_store(session_id, storage.clone());
    let cap = ContainerSandboxCapability;

    let create = tool_named(&cap, "sandbox_create");
    let create_result = create
        .execute_with_context(json!({ "image": image }), &context)
        .await;
    assert!(
        matches!(create_result, ToolExecutionResult::Success(_)),
        "sandbox_create must succeed: {create_result:?}"
    );

    let secret_key = format!(
        "{CONTAINER_SANDBOX_SECRET_PREFIX}{}",
        container_name(&session_id.to_string())
    );
    let original_raw = storage
        .get_secret(session_id, &secret_key)
        .await
        .expect("read state")
        .expect("own sandbox state must exist");
    let original: SandboxState =
        serde_json::from_str(&original_raw).expect("own sandbox state JSON");

    // Victim: Everruns-labeled sandbox for a different session on the shared daemon.
    let victim_session = "session_victim_live_forge";
    let victim_name = format!(
        "evr-live-victim-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos()
    );
    let victim = client
        .create_container(
            &victim_name,
            &ContainerCreateConfig {
                image: image.to_string(),
                cmd: vec!["sleep".to_string(), "3600".to_string()],
                working_dir: "/".to_string(),
                labels: sandbox_labels(victim_session),
                host_config: HostConfig {
                    runtime: String::new(),
                    network_mode: "bridge".to_string(),
                    memory: 64 * 1024 * 1024,
                    nano_cpus: 100_000_000,
                    pids_limit: 64,
                    init: true,
                },
            },
        )
        .await
        .expect("create victim container");
    client
        .start_container(&victim.id)
        .await
        .expect("start victim container");

    // Marker file only the victim has — if forge succeeds, read would see it.
    client
        .put_archive(&victim.id, "/", "victim-secret.txt", b"should-not-leak")
        .await
        .expect("seed victim marker file");

    let forged = SandboxState {
        container_id: victim.id.clone(),
        network_id: "bridge".to_string(),
        container_name: original.container_name.clone(),
        network_name: original.network_name.clone(),
        image: image.to_string(),
        working_dir: "/".to_string(),
        started_at: original.started_at.clone(),
    };
    storage
        .set_secret(
            session_id,
            &secret_key,
            &serde_json::to_string(&forged).expect("serialize forged state"),
        )
        .await
        .expect("forge state");

    let exec = tool_named(&cap, "sandbox_exec");
    assert_tool_error_contains(
        exec.execute_with_context(json!({ "command": "cat /victim-secret.txt" }), &context)
            .await,
        "not an Everruns-managed sandbox",
    );

    let read = tool_named(&cap, "sandbox_read_file");
    assert_tool_error_contains(
        read.execute_with_context(json!({ "path": "/victim-secret.txt" }), &context)
            .await,
        "not an Everruns-managed sandbox",
    );

    let write = tool_named(&cap, "sandbox_write_file");
    assert_tool_error_contains(
        write
            .execute_with_context(
                json!({ "path": "/pwned.txt", "content": "attacker" }),
                &context,
            )
            .await,
        "not an Everruns-managed sandbox",
    );

    let manage = tool_named(&cap, "sandbox_manage");
    assert_tool_error_contains(
        manage
            .execute_with_context(json!({ "action": "stop" }), &context)
            .await,
        "not an Everruns-managed sandbox",
    );
    assert_tool_error_contains(
        manage
            .execute_with_context(json!({ "action": "remove" }), &context)
            .await,
        "not an Everruns-managed sandbox",
    );

    // Victim must still be running and untouched.
    let victim_inspect = client
        .inspect_container(&victim.id)
        .await
        .expect("victim must still exist");
    assert!(
        victim_inspect.state.running,
        "forged stop/remove must not affect the victim"
    );
    let marker = client
        .get_archive(&victim.id, "/victim-secret.txt")
        .await
        .expect("victim marker must remain readable via direct Docker API");
    assert!(
        String::from_utf8_lossy(&marker).contains("should-not-leak"),
        "forged write/read must not alter victim filesystem"
    );

    // Restore legitimate state; own sandbox must still work.
    storage
        .set_secret(session_id, &secret_key, &original_raw)
        .await
        .expect("restore own state");
    let own_exec = exec
        .execute_with_context(json!({ "command": "echo owned-ok" }), &context)
        .await;
    match own_exec {
        ToolExecutionResult::Success(value) => {
            let text = value
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    value
                        .get("_raw_output")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| value.to_string());
            assert!(
                text.contains("owned-ok"),
                "own sandbox exec should succeed after restore: {value}"
            );
        }
        other => panic!("own sandbox must remain usable: {other:?}"),
    }

    // Cleanup via the Docker client so a manage-path flake cannot leave orphans.
    let _ = client.stop_container(&original.container_id, 2).await;
    let _ = client.remove_container(&original.container_id).await;
    let _ = client.remove_network(&original.network_id).await;
    let _ = client.stop_container(&victim.id, 2).await;
    let _ = client.remove_container(&victim.id).await;
    let _ = storage.delete_secret(session_id, &secret_key).await;
}
