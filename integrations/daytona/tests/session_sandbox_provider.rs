//! Integration tests for the Daytona session_sandbox provider.

use async_trait::async_trait;
use everruns_core::{
    connection_services::UserConnectionResolver, session_services::KeyInfo,
    session_services::SecretInfo, session_services::SessionStorageStore, tool_context::ToolContext,
};
use everruns_platform::session_sandbox::{
    SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxInstance,
    create_session_sandbox_provider,
};
use everruns_provider::error::Result;
use everruns_provider::typed_id::SessionId;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_daytona as _;

struct MockStorageStore {
    secrets: Mutex<HashMap<String, String>>,
}

impl MockStorageStore {
    fn new() -> Self {
        Self {
            secrets: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl SessionStorageStore for MockStorageStore {
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
            .insert(format!("{session_id}:{name}"), value.to_string());
        Ok(())
    }
    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        Ok(self
            .secrets
            .lock()
            .await
            .get(&format!("{session_id}:{name}"))
            .cloned())
    }
    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        Ok(self
            .secrets
            .lock()
            .await
            .remove(&format!("{session_id}:{name}"))
            .is_some())
    }
    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        let prefix = format!("{session_id}:");
        Ok(self
            .secrets
            .lock()
            .await
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .map(|key| SecretInfo {
                name: key.strip_prefix(&prefix).unwrap_or(key).to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .collect())
    }
}

struct MockConnectionResolver;

#[async_trait]
impl UserConnectionResolver for MockConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(Some("test_api_key".to_string()))
    }
}

fn test_context() -> ToolContext {
    let session_id = SessionId::new();
    let mut context =
        ToolContext::with_storage_store(session_id, Arc::new(MockStorageStore::new()));
    context.connection_resolver = Some(Arc::new(MockConnectionResolver));
    context
}

fn test_config(mock_server: &MockServer) -> SessionSandboxConfig {
    SessionSandboxConfig {
        provider: "daytona".to_string(),
        auto_start: true,
        idle_pause_after_seconds: 180,
        provider_config: json!({
            "api_base": mock_server.uri(),
            "toolbox_base": mock_server.uri(),
            "workspace_path": "/home/daytona",
            "snapshot": "daytona-small",
        }),
        init: Default::default(),
    }
}

async fn setup_exec_mocks(
    mock_server: &MockServer,
    sandbox_id: &str,
    exit_code: i64,
    output: &str,
) {
    Mock::given(method("POST"))
        .and(path(format!("/{sandbox_id}/process/session")))
        .respond_with(ResponseTemplate::new(201))
        .mount(mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path(format!(
            "/{sandbox_id}/process/session/everruns-exec/exec"
        )))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "cmdId": "cmd_001"
        })))
        .mount(mock_server)
        .await;

    let mut log_bytes = vec![0x01, 0x01, 0x01];
    log_bytes.extend_from_slice(output.as_bytes());
    Mock::given(method("GET"))
        .and(path(format!(
            "/{sandbox_id}/process/session/everruns-exec/command/cmd_001/logs"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(log_bytes))
        .mount(mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/{sandbox_id}/process/session/everruns-exec/command/cmd_001"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "exitCode": exit_code
        })))
        .mount(mock_server)
        .await;
}

fn managed_instance(external_id: &str) -> SessionSandboxInstance {
    SessionSandboxInstance {
        external_id: external_id.to_string(),
        display_name: Some("Managed Sandbox".to_string()),
        workspace_path: Some("/home/daytona".to_string()),
        provider_state: json!({}),
        metadata: json!({}),
    }
}

fn recovery_instance(external_id: &str, session_id: SessionId) -> SessionSandboxInstance {
    SessionSandboxInstance {
        external_id: external_id.to_string(),
        display_name: Some("Managed Sandbox".to_string()),
        workspace_path: Some("/workspace".to_string()),
        provider_state: json!({
            "recovery": {
                "volume_id": "vol_recovery",
                "volume_name": "everruns-recovery",
                "mount_path": "/mnt/everruns-recovery",
                "subpath": format!("sessions/{session_id}"),
                "retained_revisions": 10,
                "head_revision": "rev-1"
            }
        }),
        metadata: json!({}),
    }
}

#[test]
fn daytona_session_sandbox_provider_is_registered() {
    let provider = create_session_sandbox_provider("daytona");
    assert!(provider.is_some());
}

#[tokio::test]
async fn daytona_provider_mounts_recovery_volume_for_logical_session() {
    let mock_server = MockServer::start().await;
    let context = test_context();
    let mut config = test_config(&mock_server);
    config.provider_config["workspace_path"] = json!("/workspace");
    config.provider_config["recovery"] = json!({
        "enabled": true,
        "volume_name": "everruns-recovery"
    });
    let provider = create_session_sandbox_provider("daytona").unwrap();

    Mock::given(method("GET"))
        .and(path("/volumes/by-name/everruns-recovery"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "vol_recovery",
            "name": "everruns-recovery",
            "state": "ready"
        })))
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_recovery",
            "name": "Recovery Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_recovery"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_recovery",
            "name": "Recovery Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    setup_exec_mocks(&mock_server, "sb_recovery", 0, "").await;

    let instance = provider.create(&context, &config).await.unwrap();

    assert_eq!(
        instance.provider_state["recovery"]["volume_id"],
        "vol_recovery"
    );
    assert_eq!(
        instance.provider_state["recovery"]["subpath"],
        format!("sessions/{}", context.session_id)
    );
    let requests = mock_server.received_requests().await.unwrap();
    let create = requests
        .iter()
        .find(|request| request.url.path() == "/sandbox" && request.method.as_str() == "POST")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&create.body).unwrap();
    assert_eq!(body["volumes"][0]["volumeId"], "vol_recovery");
    assert_eq!(body["volumes"][0]["mountPath"], "/mnt/everruns-recovery");
    assert_eq!(
        body["volumes"][0]["subpath"],
        format!("sessions/{}", context.session_id)
    );
}

#[tokio::test]
async fn daytona_provider_replaces_lost_instance_and_restores_workspace() {
    let mock_server = MockServer::start().await;
    let context = test_context();
    let mut config = test_config(&mock_server);
    config.provider_config["workspace_path"] = json!("/workspace");
    let provider = create_session_sandbox_provider("daytona").unwrap();
    let instance = recovery_instance("sb_lost", context.session_id);

    Mock::given(method("GET"))
        .and(path("/sandbox/sb_lost"))
        .respond_with(ResponseTemplate::new(404).set_body_string("sandbox not found"))
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_replacement",
            "name": "Replacement Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_replacement"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_replacement",
            "name": "Replacement Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    setup_exec_mocks(&mock_server, "sb_replacement", 0, "").await;

    let lost_status = provider
        .status(
            &context,
            &config,
            &everruns_platform::session_sandbox::SessionSandboxState {
                provider: "daytona".to_string(),
                status: everruns_platform::session_sandbox::SessionSandboxStatus::Running,
                instance: instance.clone(),
                init_completed_at: Some(chrono::Utc::now().to_rfc3339()),
                last_init_error: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        lost_status.session_status,
        everruns_platform::session_sandbox::SessionSandboxStatus::Lost
    );

    let replacement = provider.resume(&context, &config, &instance).await.unwrap();

    assert_eq!(replacement.external_id, "sb_replacement");
    assert_eq!(replacement.metadata["recovered"], true);
    assert_eq!(
        replacement.provider_state["recovery"],
        instance.provider_state["recovery"]
    );
    let requests = mock_server.received_requests().await.unwrap();
    let create = requests
        .iter()
        .find(|request| request.url.path() == "/sandbox" && request.method.as_str() == "POST")
        .unwrap();
    let create_body: serde_json::Value = serde_json::from_slice(&create.body).unwrap();
    assert_eq!(create_body["volumes"][0]["volumeId"], "vol_recovery");
    assert!(requests.iter().any(|request| {
        request.url.path() == "/sb_replacement/process/session/everruns-exec/exec"
            && String::from_utf8_lossy(&request.body).contains("workspace.tar.gz.sha256")
    }));
}

#[tokio::test]
async fn daytona_provider_checkpoints_after_completed_exec() {
    let mock_server = MockServer::start().await;
    let context = test_context();
    let config = test_config(&mock_server);
    let provider = create_session_sandbox_provider("daytona").unwrap();
    let instance = recovery_instance("sb_checkpoint", context.session_id);
    setup_exec_mocks(&mock_server, "sb_checkpoint", 0, "done\n").await;

    provider
        .exec(
            &context,
            &config,
            &instance,
            &SessionSandboxExecRequest {
                command: "printf done".to_string(),
                cwd: Some("/workspace".to_string()),
                timeout_ms: Some(5_000),
                output_mode: "normal".to_string(),
            },
        )
        .await
        .unwrap();
    let checkpointed = provider
        .checkpoint(&context, &config, &instance)
        .await
        .unwrap();

    let requests = mock_server.received_requests().await.unwrap();
    assert!(requests.iter().any(|request| {
        request.url.path() == "/sb_checkpoint/process/session/everruns-exec/exec"
            && String::from_utf8_lossy(&request.body).contains("workspace.tar.gz")
            && String::from_utf8_lossy(&request.body).contains("$mount/HEAD")
            && !String::from_utf8_lossy(&request.body).contains("mv -f")
            && String::from_utf8_lossy(&request.body).contains("protected_revision='rev-1'")
    }));
    assert_ne!(
        checkpointed.provider_state["recovery"]["head_revision"],
        "rev-1"
    );
}

#[tokio::test]
async fn daytona_provider_clears_recovery_storage_when_deleting_lost_instance() {
    let mock_server = MockServer::start().await;
    let context = test_context();
    let mut config = test_config(&mock_server);
    config.provider_config["workspace_path"] = json!("/workspace");
    let provider = create_session_sandbox_provider("daytona").unwrap();
    let instance = recovery_instance("sb_lost_delete", context.session_id);

    Mock::given(method("GET"))
        .and(path("/sandbox/sb_lost_delete"))
        .respond_with(ResponseTemplate::new(404).set_body_string("sandbox not found"))
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_cleanup",
            "name": "Cleanup Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_cleanup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_cleanup",
            "name": "Cleanup Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/sandbox/sb_cleanup"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;
    setup_exec_mocks(&mock_server, "sb_cleanup", 0, "").await;

    provider.delete(&context, &config, &instance).await.unwrap();

    let requests = mock_server.received_requests().await.unwrap();
    assert!(requests.iter().any(|request| {
        request.url.path() == "/sb_cleanup/process/session/everruns-exec/exec"
            && String::from_utf8_lossy(&request.body).contains("-mindepth 1 -maxdepth 1")
    }));
    assert!(requests.iter().any(|request| {
        request.url.path() == "/sandbox/sb_cleanup" && request.method.as_str() == "DELETE"
    }));
}

#[tokio::test]
async fn daytona_provider_starts_paused_instance_before_clearing_recovery_storage() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mock_server = MockServer::start().await;
    let context = test_context();
    let mut config = test_config(&mock_server);
    config.provider_config["workspace_path"] = json!("/workspace");
    let provider = create_session_sandbox_provider("daytona").unwrap();
    let instance = recovery_instance("sb_paused_delete", context.session_id);

    let get_count = Arc::new(AtomicUsize::new(0));
    let get_count_clone = get_count.clone();
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_paused_delete"))
        .respond_with(move |_: &wiremock::Request| {
            let state = if get_count_clone.fetch_add(1, Ordering::SeqCst) < 2 {
                "stopped"
            } else {
                "started"
            };
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_paused_delete",
                "name": "Paused Sandbox",
                "state": state
            }))
        })
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sandbox/sb_paused_delete/start"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&mock_server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/sandbox/sb_paused_delete"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;
    setup_exec_mocks(&mock_server, "sb_paused_delete", 0, "").await;

    provider.delete(&context, &config, &instance).await.unwrap();

    let requests = mock_server.received_requests().await.unwrap();
    assert!(requests.iter().any(|request| {
        request.url.path() == "/sb_paused_delete/process/session/everruns-exec/exec"
            && String::from_utf8_lossy(&request.body).contains("-mindepth 1 -maxdepth 1")
    }));
}

#[tokio::test]
async fn daytona_provider_manages_managed_sandbox_flow() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mock_server = MockServer::start().await;
    let context = test_context();
    let config = test_config(&mock_server);
    let provider = create_session_sandbox_provider("daytona").unwrap();

    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_managed",
            "name": "Managed Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    let get_count = Arc::new(AtomicUsize::new(0));
    let get_count_clone = get_count.clone();
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_managed"))
        .respond_with(move |_: &wiremock::Request| {
            let n = get_count_clone.fetch_add(1, Ordering::SeqCst);
            let state = match n {
                0 => "started",
                1 => "stopping",
                2 | 3 => "stopped",
                _ => "started",
            };
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_managed",
                "name": "Managed Sandbox",
                "state": state
            }))
        })
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sandbox/sb_managed/stop"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sandbox/sb_managed/start"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/sandbox/sb_managed"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sb_managed/files/upload"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sb_managed/files/download"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fn main() {}\n".to_vec()))
        .mount(&mock_server)
        .await;
    setup_exec_mocks(&mock_server, "sb_managed", 0, "ready\n").await;

    let instance = provider.create(&context, &config).await.unwrap();
    assert_eq!(instance.external_id, "sb_managed");

    let exec = provider
        .exec(
            &context,
            &config,
            &instance,
            &everruns_platform::session_sandbox::SessionSandboxExecRequest {
                command: "echo ready".to_string(),
                cwd: Some("/home/daytona".to_string()),
                timeout_ms: Some(10_000),
                output_mode: "concise".to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(exec.exit_code, 0);
    assert!(exec.stdout.contains("ready"));

    let read = provider
        .read_file(&context, &config, &instance, "/home/daytona/main.rs")
        .await
        .unwrap();
    assert_eq!(read.encoding, "text");

    let write = provider
        .write_file(
            &context,
            &config,
            &instance,
            "/home/daytona/main.rs",
            "fn main() {}\n",
        )
        .await
        .unwrap();
    assert_eq!(write.bytes_written, 13);

    let paused = provider.pause(&context, &config, &instance).await.unwrap();
    assert_eq!(paused.external_id, "sb_managed");

    let resumed = provider.resume(&context, &config, &paused).await.unwrap();
    assert_eq!(resumed.external_id, "sb_managed");

    let status = provider
        .status(
            &context,
            &config,
            &everruns_platform::session_sandbox::SessionSandboxState {
                provider: "daytona".to_string(),
                status: everruns_platform::session_sandbox::SessionSandboxStatus::Running,
                instance: resumed.clone(),
                init_completed_at: None,
                last_init_error: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .await
        .unwrap();
    assert_eq!(status.external_id, "sb_managed");

    provider.delete(&context, &config, &resumed).await.unwrap();
}

#[tokio::test]
async fn daytona_provider_resume_tolerates_transition_conflicts() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mock_server = MockServer::start().await;
    let context = test_context();
    let config = test_config(&mock_server);
    let provider = create_session_sandbox_provider("daytona").unwrap();
    let instance = managed_instance("sb_transition");

    Mock::given(method("POST"))
        .and(path("/sandbox/sb_transition/stop"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let get_count = Arc::new(AtomicUsize::new(0));
    let get_count_clone = get_count.clone();
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_transition"))
        .respond_with(move |_: &wiremock::Request| {
            let n = get_count_clone.fetch_add(1, Ordering::SeqCst);
            let state = match n {
                0 => "stopping",
                1 | 2 => "stopped",
                3 => "starting",
                _ => "started",
            };
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_transition",
                "name": "Managed Sandbox",
                "state": state
            }))
        })
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/sandbox/sb_transition/start"))
        .respond_with(ResponseTemplate::new(409).set_body_string(
            "{\"statusCode\":409,\"message\":\"Sandbox state change in progress\"}",
        ))
        .expect(1)
        .mount(&mock_server)
        .await;

    let paused = provider.pause(&context, &config, &instance).await.unwrap();
    assert_eq!(paused.metadata["remote_state"], "stopped");

    let resumed = provider.resume(&context, &config, &paused).await.unwrap();
    assert_eq!(resumed.external_id, "sb_transition");
    assert_eq!(resumed.metadata["remote_state"], "started");
}

#[tokio::test]
async fn daytona_provider_resume_stays_within_single_poll_budget_on_transition_timeout() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mock_server = MockServer::start().await;
    let context = test_context();
    let config = test_config(&mock_server);
    let provider = create_session_sandbox_provider("daytona").unwrap();
    let instance = managed_instance("sb_transition_timeout");

    let get_count = Arc::new(AtomicUsize::new(0));
    let get_count_clone = get_count.clone();
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_transition_timeout"))
        .respond_with(move |_: &wiremock::Request| {
            get_count_clone.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_transition_timeout",
                "name": "Managed Sandbox",
                "state": "starting"
            }))
        })
        .mount(&mock_server)
        .await;

    let err = provider
        .resume(&context, &config, &instance)
        .await
        .unwrap_err();

    assert_eq!(get_count.load(Ordering::SeqCst), 20);
    let everruns_core::tools::ToolExecutionResult::ToolError(message) = err else {
        panic!("expected tool error");
    };
    assert!(
        message.contains("Daytona sandbox 'sb_transition_timeout' did not reach 'started' state")
    );
    assert!(message.contains("last state: starting"));
}

#[tokio::test]
async fn daytona_provider_escapes_workspace_path_when_creating_directory() {
    let mock_server = MockServer::start().await;
    let context = test_context();
    let mut config = test_config(&mock_server);
    config.provider_config["workspace_path"] = json!("/tmp/it's workspace; rm -rf /");
    let provider = create_session_sandbox_provider("daytona").unwrap();

    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_escape",
            "name": "Escaped Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_escape"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_escape",
            "name": "Escaped Sandbox",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sb_escape/process/session"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sb_escape/process/session/everruns-exec/exec"))
        .and(body_string_contains(
            "mkdir -p -- '/tmp/it'\\\\''s workspace; rm -rf /'",
        ))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "cmdId": "cmd_escape"
        })))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/sb_escape/process/session/everruns-exec/command/cmd_escape/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x01, 0x01, 0x01]))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/sb_escape/process/session/everruns-exec/command/cmd_escape",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "exitCode": 0
        })))
        .mount(&mock_server)
        .await;

    let instance = provider.create(&context, &config).await.unwrap();
    assert_eq!(instance.external_id, "sb_escape");
    assert_eq!(
        instance.workspace_path.as_deref(),
        Some("/tmp/it's workspace; rm -rf /")
    );
}

#[tokio::test]
async fn daytona_provider_retries_conflict_and_uses_canonical_display_name() {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mock_server = MockServer::start().await;
    let context = test_context();
    let mut config = test_config(&mock_server);
    config.provider_config["title"] = json!("Managed Sandbox");
    let provider = create_session_sandbox_provider("daytona").unwrap();

    let seen_names = Arc::new(StdMutex::new(Vec::<String>::new()));
    let call_count = Arc::new(AtomicUsize::new(0));
    let seen_names_clone = seen_names.clone();
    let call_count_clone = call_count.clone();
    Mock::given(method("POST"))
        .and(path("/sandbox"))
        .respond_with(move |request: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let name = body["name"].as_str().unwrap().to_string();
            seen_names_clone.lock().unwrap().push(name.clone());

            if call_count_clone.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(409).set_body_string("sandbox already exists")
            } else {
                ResponseTemplate::new(200).set_body_json(json!({
                    "id": "sb_retry",
                    "name": name,
                    "state": "started"
                }))
            }
        })
        .expect(2)
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_retry"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_retry",
            "name": "ready-name",
            "state": "started"
        })))
        .mount(&mock_server)
        .await;
    setup_exec_mocks(&mock_server, "sb_retry", 0, "ready\n").await;

    let instance = provider.create(&context, &config).await.unwrap();

    assert_eq!(instance.external_id, "sb_retry");
    let display_name = instance.display_name.as_deref().unwrap();
    assert!(display_name.starts_with("Managed Sandbox-"));

    let seen_names = seen_names.lock().unwrap();
    assert_eq!(seen_names.len(), 2);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
    assert!(
        seen_names
            .iter()
            .all(|name| name.starts_with("Managed Sandbox-"))
    );
    assert_eq!(display_name, seen_names[1]);
}

#[tokio::test]
async fn daytona_provider_exec_timeout_resets_session_and_next_command_succeeds() {
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicU32, Ordering};

    let mock_server = MockServer::start().await;
    let context = test_context();
    let config = test_config(&mock_server);
    let provider = create_session_sandbox_provider("daytona").unwrap();
    let instance = managed_instance("sb_timeout");

    Mock::given(method("POST"))
        .and(path("/sb_timeout/process/session"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&mock_server)
        .await;

    let exec_call_count = StdArc::new(AtomicU32::new(0));
    let counter = exec_call_count.clone();
    Mock::given(method("POST"))
        .and(path("/sb_timeout/process/session/everruns-exec/exec"))
        .respond_with(move |_: &wiremock::Request| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            let cmd_id = if n == 0 { "cmd_timeout" } else { "cmd_ok" };
            ResponseTemplate::new(202).set_body_json(json!({ "cmdId": cmd_id }))
        })
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path(
            "/sb_timeout/process/session/everruns-exec/command/cmd_timeout/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![]))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/sb_timeout/process/session/everruns-exec/command/cmd_timeout",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmd_timeout",
            "command": "sleep 999"
        })))
        .mount(&mock_server)
        .await;

    let mut ok_log_bytes = vec![0x01, 0x01, 0x01];
    ok_log_bytes.extend_from_slice(b"still works\n");
    Mock::given(method("GET"))
        .and(path(
            "/sb_timeout/process/session/everruns-exec/command/cmd_ok/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(ok_log_bytes))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/sb_timeout/process/session/everruns-exec/command/cmd_ok",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmd_ok",
            "command": "echo still works",
            "exitCode": 0
        })))
        .mount(&mock_server)
        .await;

    Mock::given(method("DELETE"))
        .and(path("/sb_timeout/process/session/everruns-exec"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&mock_server)
        .await;

    let timeout_err = provider
        .exec(
            &context,
            &config,
            &instance,
            &SessionSandboxExecRequest {
                command: "sleep 999".to_string(),
                cwd: None,
                timeout_ms: Some(1_200),
                output_mode: "normal".to_string(),
            },
        )
        .await
        .unwrap_err();
    let everruns_core::tools::ToolExecutionResult::ToolError(message) = timeout_err else {
        panic!("Expected ToolError timeout, got {timeout_err:?}");
    };
    assert!(message.contains("Command timed out after"));
    assert!(message.contains("automatically reset"));

    let result = provider
        .exec(
            &context,
            &config,
            &instance,
            &SessionSandboxExecRequest {
                command: "echo still works".to_string(),
                cwd: None,
                timeout_ms: Some(5_000),
                output_mode: "normal".to_string(),
            },
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "still works\n");
    assert_eq!(result.stderr, "");
    assert_eq!(result.raw_output.as_deref(), Some("still works\n"));
    assert_eq!(exec_call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn daytona_provider_exec_maps_signal_exit_codes_to_hints() {
    let mock_server = MockServer::start().await;
    let context = test_context();
    let config = test_config(&mock_server);
    let provider = create_session_sandbox_provider("daytona").unwrap();
    let instance = managed_instance("sb_signal");

    setup_exec_mocks(&mock_server, "sb_signal", 141, "partial output\n").await;

    let result = provider
        .exec(
            &context,
            &config,
            &instance,
            &SessionSandboxExecRequest {
                command: "yes | head -n 1".to_string(),
                cwd: None,
                timeout_ms: Some(5_000),
                output_mode: "normal".to_string(),
            },
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 141);
    assert!(!result.success);
    assert_eq!(result.stdout, "partial output\n");
    assert_eq!(result.stderr, "");
    assert_eq!(result.raw_output.as_deref(), Some("partial output\n"));
    let hint = result.hint.as_deref().expect("expected signal hint");
    assert!(hint.contains("SIGPIPE"), "unexpected hint: {hint}");
}
