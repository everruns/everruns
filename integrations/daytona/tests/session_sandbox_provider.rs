//! Integration tests for the Daytona session_sandbox provider.

use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::session_sandbox::{SessionSandboxConfig, create_session_sandbox_provider};
use everruns_core::traits::{KeyInfo, SecretInfo, ToolContext, UserConnectionResolver};
use everruns_core::{SessionStorageStore, typed_id::SessionId};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
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

#[test]
fn daytona_session_sandbox_provider_is_registered() {
    let provider = create_session_sandbox_provider("daytona");
    assert!(provider.is_some());
}

#[tokio::test]
async fn daytona_provider_manages_managed_sandbox_flow() {
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
    Mock::given(method("GET"))
        .and(path("/sandbox/sb_managed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sb_managed",
            "name": "Managed Sandbox",
            "state": "started"
        })))
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
            &everruns_core::SessionSandboxExecRequest {
                command: "echo ready".to_string(),
                cwd: Some("/home/daytona".to_string()),
                timeout_ms: Some(10_000),
                output_mode: "concise".to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(exec.exit_code, 0);
    assert!(exec.output.contains("ready"));

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
            &everruns_core::SessionSandboxState {
                provider: "daytona".to_string(),
                status: everruns_core::SessionSandboxStatus::Running,
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
