//! A managed sandbox provider that runs on this machine, for fleet tests
//! and the sandbox bench (data plane v2, step 11.3).
//!
//! Provider id `local`: each sandbox is a directory under the system temp
//! directory whose `workspace` the sandbox tools see, and `bash` runs there.
//! It labels its sandboxes like a real provider (so an orphan scan finds
//! them), can be told to lose one (as a provider that deleted it), and
//! can add a fixed delay to every call (`provider_config.latency_ms`), to
//! compare sync against a provider's own round trip.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::session_sandbox::{
    OwnedSandbox, SandboxAdminError, SessionSandboxConfig, SessionSandboxContext,
    SessionSandboxExecRequest, SessionSandboxExecResponse, SessionSandboxInstance,
    SessionSandboxProvider, SessionSandboxProviderPlugin, SessionSandboxReadFileResponse,
    SessionSandboxState, SessionSandboxStatus, SessionSandboxStatusResponse,
    SessionSandboxWriteFileResponse,
};
use everruns_contracts::tools::ToolExecutionResult;
use parking_lot::Mutex;
use serde_json::{Value, json};

pub const PROVIDER: &str = "local";

/// Live sandboxes by id: their labels.
static SANDBOXES: LazyLock<Mutex<HashMap<String, BTreeMap<String, String>>>> =
    LazyLock::new(Mutex::default);
/// Calls by session and kind, for tests.
static CALLS: LazyLock<Mutex<BTreeMap<(String, String), u64>>> = LazyLock::new(Mutex::default);

fn count(session: &str, kind: &str) {
    *CALLS
        .lock()
        .entry((session.to_owned(), kind.to_owned()))
        .or_default() += 1;
}

/// Calls the provider took for `session`, by kind (`create`, `exec`,
/// `read_file`, ...).
pub fn calls_of(session: &str) -> BTreeMap<String, u64> {
    CALLS
        .lock()
        .iter()
        .filter(|((s, _), _)| s == session)
        .map(|((_, kind), n)| (kind.clone(), *n))
        .collect()
}

/// Live sandboxes labeled with `session`.
pub fn sandboxes_of(session: &str) -> Vec<String> {
    SANDBOXES
        .lock()
        .iter()
        .filter(|(_, labels)| {
            labels.get("everruns.session_id").map(String::as_str) == Some(session)
        })
        .map(|(id, _)| id.clone())
        .collect()
}

/// Where sandbox `id` lives.
pub fn dir(id: &str) -> PathBuf {
    std::env::temp_dir()
        .join("everruns-local-sandboxes")
        .join(id)
}

/// The workspace of sandbox `id`.
pub fn workspace(id: &str) -> PathBuf {
    dir(id).join("workspace")
}

/// Lose sandbox `id`, as a provider that deleted it would.
pub fn lose(id: &str) {
    let _ = std::fs::remove_dir_all(dir(id));
    SANDBOXES.lock().remove(id);
}

/// A sandbox labeled with `session` that no session recorded (its
/// executor died between the provider's create and the record); its id.
pub fn orphan(session: &str) -> String {
    let id = format!("sb_{}", uuid::Uuid::new_v4().simple());
    let _ = std::fs::create_dir_all(workspace(&id));
    SANDBOXES.lock().insert(
        id.clone(),
        BTreeMap::from([
            ("everruns".to_owned(), "true".to_owned()),
            ("everruns.session_id".to_owned(), session.to_owned()),
        ]),
    );
    id
}

/// The `session_sandbox` capability entry for this provider: every call
/// takes `latency_ms` more, and a create `create_ms` more again.
pub fn capability(latency_ms: u64, create_ms: u64) -> Value {
    json!({
        "ref": "session_sandbox",
        "config": {
            "provider": PROVIDER,
            "credential": { "source": "platform" },
            "auto_start": true,
            "idle_pause_after_seconds": 300,
            "provider_config": { "latency_ms": latency_ms, "create_ms": create_ms },
        }
    })
}

pub struct LocalSandboxProvider;

inventory::submit! {
    SessionSandboxProviderPlugin {
        factory: || Box::new(LocalSandboxProvider),
    }
}

async fn delay(config: &SessionSandboxConfig) {
    delay_of(config, "latency_ms").await;
}

async fn delay_of(config: &SessionSandboxConfig, field: &str) {
    if let Some(ms) = config.provider_config[field].as_u64()
        && ms > 0
    {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

fn instance(id: &str) -> SessionSandboxInstance {
    SessionSandboxInstance {
        external_id: id.to_owned(),
        display_name: None,
        workspace_path: Some(workspace(id).display().to_string()),
        provider_state: Value::Null,
        metadata: json!({}),
    }
}

fn failed(error: impl std::fmt::Display) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(format!("local sandbox: {error}"))
}

async fn create(
    context: &dyn SessionSandboxContext,
) -> Result<SessionSandboxInstance, ToolExecutionResult> {
    let id = format!("sb_{}", uuid::Uuid::new_v4().simple());
    std::fs::create_dir_all(workspace(&id)).map_err(failed)?;
    let labels = context
        .resource_labels()
        .await
        .into_iter()
        .map(|(k, v)| (k, v.as_str().map_or_else(|| v.to_string(), str::to_owned)))
        .collect();
    SANDBOXES.lock().insert(id.clone(), labels);
    Ok(instance(&id))
}

#[async_trait]
impl SessionSandboxProvider for LocalSandboxProvider {
    fn id(&self) -> &str {
        PROVIDER
    }

    async fn list_owned(
        &self,
        _token: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<Vec<OwnedSandbox>, SandboxAdminError> {
        count(
            labels.get("everruns.session_id").map_or("", String::as_str),
            "list_owned",
        );
        Ok(SANDBOXES
            .lock()
            .iter()
            .filter(|(_, have)| labels.iter().all(|(k, v)| have.get(k) == Some(v)))
            .map(|(id, have)| OwnedSandbox {
                external_id: id.clone(),
                labels: have.clone(),
                state: "started".into(),
                active: true,
                created_at: None,
            })
            .collect())
    }

    async fn delete_by_id(&self, _token: &str, external_id: &str) -> Result<(), SandboxAdminError> {
        let session = SANDBOXES
            .lock()
            .get(external_id)
            .and_then(|labels| labels.get("everruns.session_id").cloned())
            .unwrap_or_default();
        count(&session, "delete_by_id");
        lose(external_id);
        Ok(())
    }

    async fn create(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        count(&context.session_id().to_string(), "create");
        delay(config).await;
        delay_of(config, "create_ms").await;
        create(context).await
    }

    async fn resume(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        count(&context.session_id().to_string(), "resume");
        delay(config).await;
        if workspace(&instance.external_id).is_dir() {
            return Ok(instance.clone());
        }
        // Gone: an empty replacement, as a provider gives.
        create(context).await
    }

    async fn pause(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        Ok(instance.clone())
    }

    async fn delete(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult> {
        lose(&instance.external_id);
        Ok(())
    }

    async fn exec(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        request: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        count(&context.session_id().to_string(), "exec");
        delay(config).await;
        let root = workspace(&instance.external_id);
        let cwd = request.cwd.clone().map_or(root, PathBuf::from);
        let run = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(&request.command)
            .current_dir(cwd)
            .kill_on_drop(true)
            .output();
        let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(60_000));
        let output = tokio::time::timeout(timeout, run)
            .await
            .map_err(|_| failed("the command timed out"))?
            .map_err(failed)?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        Ok(SessionSandboxExecResponse {
            exit_code: output.status.code().unwrap_or(-1),
            total_lines: stdout.lines().count(),
            stdout,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            success: output.status.success(),
            truncated: false,
            raw_output: None,
            hint: None,
        })
    }

    async fn read_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        _instance: &SessionSandboxInstance,
        path: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
        count(&context.session_id().to_string(), "read_file");
        delay(config).await;
        let bytes = std::fs::read(path).map_err(failed)?;
        let (content, encoding) = everruns_contracts::runtime::SessionFile::encode_content(&bytes);
        Ok(SessionSandboxReadFileResponse {
            path: path.to_owned(),
            content,
            encoding,
        })
    }

    async fn write_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        _instance: &SessionSandboxInstance,
        path: &str,
        content: &[u8],
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
        count(&context.session_id().to_string(), "write_file");
        delay(config).await;
        let path_buf = PathBuf::from(path);
        if let Some(parent) = path_buf.parent() {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
        std::fs::write(&path_buf, content).map_err(failed)?;
        Ok(SessionSandboxWriteFileResponse {
            path: path.to_owned(),
            bytes_written: content.len(),
        })
    }

    async fn status(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
        count(&context.session_id().to_string(), "status");
        delay(config).await;
        let id = &state.instance.external_id;
        Ok(SessionSandboxStatusResponse {
            provider: PROVIDER.into(),
            session_status: if workspace(id).is_dir() {
                SessionSandboxStatus::Running
            } else {
                SessionSandboxStatus::Lost
            },
            external_id: id.clone(),
            display_name: None,
            workspace_path: state.instance.workspace_path.clone(),
            metadata: json!({}),
        })
    }
}
