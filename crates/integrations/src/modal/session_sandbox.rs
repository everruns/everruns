//! Modal as a managed Sandboxes provider (`session_sandbox`, provider `modal`).
//!
//! Decisions:
//! - Modal has no stop/start. Pause snapshots the filesystem into an image
//!   (`SandboxSnapshotFs`) and terminates the sandbox; resume boots a new
//!   sandbox from that image. The external id therefore changes on resume,
//!   which the host already supports (it is how Daytona replaces a lost box).
//! - Terminate is asynchronous on Modal's side: a sandbox can still report
//!   running just after pause terminated it. Pause therefore records
//!   `paused` in the provider state, and resume and status trust that record
//!   over Modal's report.
//! - A sandbox that finished without being paused (Modal lifetime or idle
//!   timeout) reports `Paused` when an earlier pause left a snapshot to boot
//!   from, otherwise `Lost`; resume then boots the base image again.
//! - Files go over exec: reads through `base64` so binary content survives,
//!   writes through stdin with the path as an argv value, never shell text.
//! - Egress rules (`network`, written by the server from the template's
//!   containment, and `inject_connections`) apply on every boot; the egress
//!   Secret is recreated per boot and deleted on pause and delete.
//! - No recovery checkpoints: durability is `provider_snapshot` only, enforced
//!   by Sandbox Template validation.

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine as _;
use everruns_contracts::runtime::exec_tool_result::ExecToolResultPayload;
use everruns_contracts::session_sandbox::{
    SessionSandboxConfig, SessionSandboxContext, SessionSandboxExecRequest,
    SessionSandboxExecResponse, SessionSandboxInstance, SessionSandboxLease,
    SessionSandboxProvider, SessionSandboxReadFileResponse, SessionSandboxState,
    SessionSandboxStatus, SessionSandboxStatusResponse, SessionSandboxWriteFileResponse,
};
use everruns_contracts::tools::ToolExecutionResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::warn;

use super::client::{CreateSandboxParams, ModalClient, ModalCredentials, SandboxStatus};
use super::egress::{EgressSpec, delete_egress_secret};
use super::state::MODAL_SANDBOX_LEASE_DURATION_SECONDS;
use super::tools::validate_image_tag;
use super::{
    MODAL_APP_NAME, MODAL_DEFAULT_IMAGE, MODAL_DEFAULT_RUNTIME, MODAL_DEFAULT_SETUP_COMMANDS,
    MODAL_MAX_EXEC_TIMEOUT_SECS, MODAL_MAX_TIMEOUT_SECS, MODAL_PROVIDER, MODAL_WORKSPACE_PATH,
};

/// Default exec timeout when the host does not pass one.
const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(120);
/// How often a long exec refreshes the cleanup lease.
const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Largest file `read_file` returns.
const MAX_READ_FILE_BYTES: usize = 5 * 1024 * 1024;

/// The `modal` managed Sandboxes provider.
pub struct ModalSessionSandboxProvider;

/// Non-secret state needed to reach and resume the sandbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct ProviderState {
    task_id: String,
    app_id: String,
    runtime: String,
    image: String,
    /// Filesystem snapshot from the last pause; resume boots from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot_image_id: Option<String>,
    /// Set by pause, which terminated the sandbox.
    #[serde(default)]
    paused: bool,
    /// Modal Secret holding injected connection tokens for this boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    egress_secret_id: Option<String>,
}

impl ProviderState {
    fn from_instance(instance: &SessionSandboxInstance) -> Result<Self, ToolExecutionResult> {
        serde_json::from_value(instance.provider_state.clone()).map_err(|e| {
            ToolExecutionResult::internal_error_msg(format!("Corrupt Modal sandbox state: {e}"))
        })
    }

    fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
}

/// Template options (`target.options`), validated again here because the
/// provider must not trust its config.
#[derive(Debug, Clone, PartialEq)]
struct Options {
    image: Option<String>,
    runtime: String,
    milli_cpu: Option<u32>,
    memory_mb: Option<u32>,
    workspace_path: String,
    egress: EgressSpec,
}

fn options(config: &SessionSandboxConfig) -> Result<Options, ToolExecutionResult> {
    let cfg = &config.provider_config;
    let image = match cfg.get("image") {
        None | Some(Value::Null) => None,
        Some(Value::String(tag)) => {
            validate_image_tag(tag)?;
            Some(tag.clone())
        }
        Some(_) => {
            return Err(ToolExecutionResult::tool_error(
                "Modal option 'image' must be a string",
            ));
        }
    };
    let runtime = match cfg.get("runtime").and_then(Value::as_str) {
        None => MODAL_DEFAULT_RUNTIME.to_string(),
        Some(runtime @ ("vm" | "gvisor")) => runtime.to_string(),
        Some(other) => {
            return Err(ToolExecutionResult::tool_error(format!(
                "Modal option 'runtime' must be vm or gvisor, not '{other}'"
            )));
        }
    };
    let milli_cpu = match cfg.get("cpu") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_f64().filter(|c| (0.125..=64.0).contains(c)) {
            Some(cpu) => Some((cpu * 1000.0).round() as u32),
            None => {
                return Err(ToolExecutionResult::tool_error(
                    "Modal option 'cpu' must be between 0.125 and 64",
                ));
            }
        },
    };
    let memory_mb = match cfg.get("memory_mb") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64().filter(|m| (128..=262_144).contains(m)) {
            Some(mb) => Some(mb as u32),
            None => {
                return Err(ToolExecutionResult::tool_error(
                    "Modal option 'memory_mb' must be between 128 and 262144",
                ));
            }
        },
    };
    let workspace_path = match cfg.get("workspace_path").and_then(Value::as_str) {
        None => MODAL_WORKSPACE_PATH.to_string(),
        Some(path) if is_normalized_absolute_path(path) => path.to_string(),
        Some(_) => {
            return Err(ToolExecutionResult::tool_error(
                "Modal option 'workspace_path' must be a normalized absolute non-root path",
            ));
        }
    };
    let egress = EgressSpec::from_json(cfg)
        .map_err(|e| ToolExecutionResult::tool_error(format!("Invalid Modal egress: {e}")))?;
    Ok(Options {
        image,
        runtime,
        milli_cpu,
        memory_mb,
        workspace_path,
        egress,
    })
}

fn is_normalized_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && !path.ends_with('/')
        && !path.contains("//")
        && !path.split('/').any(|part| matches!(part, "." | ".."))
}

async fn client(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
) -> Result<ModalClient, ToolExecutionResult> {
    let raw = match context
        .sandbox_connection_token(MODAL_PROVIDER, &config.credential)
        .await?
    {
        Some(raw) if !raw.trim().is_empty() => raw,
        // THREAT[TM-AGENT-016]: never ask for credentials in chat.
        _ => return Err(ToolExecutionResult::connection_required(MODAL_PROVIDER)),
    };
    let credentials = ModalCredentials::parse(&raw).map_err(|e| {
        ToolExecutionResult::tool_error(format!(
            "The saved Modal connection is not a valid token pair: {e} Reconnect Modal in settings."
        ))
    })?;
    // Tests point the client at an in-process mock. Templates cannot set this
    // key (validation allowlists options) and shipped builds never read it.
    #[cfg(feature = "modal-test-util")]
    if let Some(url) = config
        .provider_config
        .get("_test_server_url")
        .and_then(Value::as_str)
    {
        return ModalClient::with_server_url(credentials, url)
            .map_err(ToolExecutionResult::tool_error);
    }
    #[cfg(not(feature = "modal-test-util"))]
    let _ = config;
    ModalClient::new(credentials).map_err(ToolExecutionResult::tool_error)
}

fn is_not_found(err: &str) -> bool {
    err.contains("not found")
}

async fn refresh_lease(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
    instance: &SessionSandboxInstance,
    state: &ProviderState,
) -> Result<(), ToolExecutionResult> {
    context
        .refresh_lease(SessionSandboxLease {
            provider: MODAL_PROVIDER.to_string(),
            external_id: instance.external_id.clone(),
            display_name: instance.display_name.clone(),
            duration_seconds: MODAL_SANDBOX_LEASE_DURATION_SECONDS,
            credential: config.credential.clone(),
            // THREAT[TM-API-015]: lease metadata is API-visible; non-secret only.
            metadata: json!({
                "runtime": state.runtime,
                "image": state.image,
                "workspace_path": instance.workspace_path,
                "managed": true,
                // An ID, not the secret: cleanup deletes the Modal Secret by it.
                "egress_secret_id": state.egress_secret_id,
            }),
        })
        .await
}

/// Boot a sandbox from `image_id` (or build the configured image) and record it.
async fn boot(
    client: &ModalClient,
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
    display_name: Option<String>,
    snapshot_image_id: Option<&str>,
) -> Result<SessionSandboxInstance, ToolExecutionResult> {
    let options = options(config)?;
    let app_id = client
        .get_or_create_app(MODAL_APP_NAME)
        .await
        .map_err(ToolExecutionResult::tool_error)?;
    let image_label = options
        .image
        .clone()
        .unwrap_or_else(|| MODAL_DEFAULT_IMAGE.to_string());
    let image_id = match snapshot_image_id {
        Some(id) => id.to_string(),
        None => {
            // The default image gets git and curl; a custom one is used as is.
            let commands: Vec<String> = if options.image.is_none() {
                MODAL_DEFAULT_SETUP_COMMANDS
                    .iter()
                    .map(|c| (*c).to_string())
                    .collect()
            } else {
                Vec::new()
            };
            client
                .build_image(&app_id, &image_label, &commands)
                .await
                .map_err(ToolExecutionResult::tool_error)?
        }
    };

    let (egress_secret_id, header_replacements) =
        options.egress.prepare(client, &app_id, context).await?;
    let mut tags: Vec<(String, String)> = context
        .resource_labels()
        .await
        .into_iter()
        .filter_map(|(key, value)| value.as_str().map(|v| (key, v.to_string())))
        .collect();
    tags.push(("everruns.managed".to_string(), "true".to_string()));
    let params = CreateSandboxParams {
        image_id,
        runtime: Some(options.runtime.clone()),
        timeout_secs: MODAL_MAX_TIMEOUT_SECS,
        idle_timeout_secs: None,
        workdir: None,
        milli_cpu: options.milli_cpu,
        memory_mb: options.memory_mb,
        encrypted_ports: Vec::new(),
        tags,
        network_access: options.egress.network.clone(),
        header_replacements,
    };
    let (sandbox_id, task_id) = match client.create_sandbox(&app_id, &params).await {
        Ok(ids) => ids,
        Err(err) => {
            delete_egress_secret(client, egress_secret_id.as_deref()).await;
            return Err(ToolExecutionResult::tool_error(err));
        }
    };

    let state = ProviderState {
        task_id,
        app_id,
        runtime: options.runtime,
        image: image_label,
        snapshot_image_id: snapshot_image_id.map(str::to_string),
        paused: false,
        egress_secret_id,
    };
    let instance = SessionSandboxInstance {
        external_id: sandbox_id.clone(),
        display_name,
        workspace_path: Some(options.workspace_path.clone()),
        provider_state: state.to_value(),
        metadata: json!({
            "remote_state": "running",
            "runtime": state.runtime,
            "restored_from_snapshot": snapshot_image_id.is_some(),
        }),
    };
    // A sandbox nothing can clean up must not outlive this call.
    if let Err(err) = refresh_lease(context, config, &instance, &state).await {
        let _ = client.terminate(&sandbox_id).await;
        delete_egress_secret(client, state.egress_secret_id.as_deref()).await;
        return Err(err);
    }
    let mkdir = vec![
        "mkdir".to_string(),
        "-p".to_string(),
        options.workspace_path,
    ];
    if let Err(err) = client
        .exec(
            &state.task_id,
            &mkdir,
            None,
            &HashMap::new(),
            Duration::from_secs(60),
            None,
        )
        .await
    {
        warn!(sandbox_id = %sandbox_id, "Failed to create Modal workspace directory: {err}");
    }
    Ok(instance)
}

fn resolve_path(instance: &SessionSandboxInstance, path: &str) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    let base = instance
        .workspace_path
        .as_deref()
        .unwrap_or(MODAL_WORKSPACE_PATH);
    format!("{}/{}", base.trim_end_matches('/'), path)
}

fn exit_code_hint(exit_code: i32) -> Option<&'static str> {
    match exit_code {
        0 => None,
        124 => Some("Command timed out."),
        126 => Some("Command found but not executable. Check file permissions."),
        127 => Some("Command not found. Check that the tool is installed and in PATH."),
        137 => {
            Some("Process was killed (SIGKILL). This often means the process ran out of memory.")
        }
        _ if exit_code > 128 && exit_code <= 192 => Some("Process was killed by a signal."),
        _ => None,
    }
}

#[async_trait::async_trait]
impl SessionSandboxProvider for ModalSessionSandboxProvider {
    fn id(&self) -> &str {
        MODAL_PROVIDER
    }

    async fn create(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let client = client(context, config).await?;
        let display_name = config
            .provider_config
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .map(str::to_string)
            .or_else(|| Some(format!("Session Sandbox {}", context.session_id())));
        boot(&client, context, config, display_name, None).await
    }

    async fn resume(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let client = client(context, config).await?;
        let state = ProviderState::from_instance(instance)?;
        match client.sandbox_status(&instance.external_id).await {
            // A paused sandbox may still report running while it shuts down.
            _ if state.paused => {}
            Ok(SandboxStatus::Running) => {
                refresh_lease(context, config, instance, &state).await?;
                return Ok(instance.clone());
            }
            Ok(SandboxStatus::Finished { .. }) => {}
            Err(err) if is_not_found(&err) => {}
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        }
        let replacement = boot(
            &client,
            context,
            config,
            instance.display_name.clone(),
            state.snapshot_image_id.as_deref(),
        )
        .await?;
        delete_egress_secret(&client, state.egress_secret_id.as_deref()).await;
        if let Err(err) = context
            .release_lease(MODAL_PROVIDER, &instance.external_id)
            .await
        {
            warn!(sandbox_id = %instance.external_id, error = ?err, "Failed to release lease for replaced Modal sandbox");
        }
        Ok(replacement)
    }

    async fn pause(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let client = client(context, config).await?;
        let mut state = ProviderState::from_instance(instance)?;
        if state.paused {
            return Ok(instance.clone());
        }
        let remote_state = match client.snapshot_filesystem(&instance.external_id).await {
            Ok(image_id) => {
                state.snapshot_image_id = Some(image_id);
                "paused"
            }
            // Already gone: keep the previous snapshot, if any, to resume from.
            Err(err) => match client.sandbox_status(&instance.external_id).await {
                Ok(SandboxStatus::Finished { .. }) => "lost",
                Err(status_err) if is_not_found(&status_err) => "lost",
                _ => return Err(ToolExecutionResult::tool_error(err)),
            },
        };
        match client.terminate(&instance.external_id).await {
            Ok(()) => {}
            Err(err) if is_not_found(&err) => {}
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        }
        context
            .release_lease(MODAL_PROVIDER, &instance.external_id)
            .await?;
        delete_egress_secret(&client, state.egress_secret_id.take().as_deref()).await;
        state.paused = true;
        Ok(SessionSandboxInstance {
            provider_state: state.to_value(),
            metadata: json!({
                "remote_state": remote_state,
                "runtime": state.runtime,
                "snapshot_image_id": state.snapshot_image_id,
            }),
            ..instance.clone()
        })
    }

    async fn delete(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult> {
        let client = client(context, config).await?;
        match client.terminate(&instance.external_id).await {
            Ok(()) => {}
            Err(err) if is_not_found(&err) => {}
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        }
        if let Ok(state) = ProviderState::from_instance(instance) {
            delete_egress_secret(&client, state.egress_secret_id.as_deref()).await;
        }
        context
            .release_lease(MODAL_PROVIDER, &instance.external_id)
            .await
    }

    async fn exec(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        request: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        let client = client(context, config).await?;
        let state = ProviderState::from_instance(instance)?;
        let cwd = resolve_path(instance, request.cwd.as_deref().unwrap_or("."));
        let cwd = cwd.trim_end_matches("/.").to_string();
        let timeout = request
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_EXEC_TIMEOUT)
            .min(Duration::from_secs(u64::from(MODAL_MAX_EXEC_TIMEOUT_SECS)));

        // Long commands keep the lease alive so cleanup does not reap a busy box.
        let heartbeat_context = context.clone_context();
        let heartbeat_config = config.clone();
        let heartbeat_instance = instance.clone();
        let heartbeat_state = state.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::time::sleep(LEASE_HEARTBEAT_INTERVAL).await;
                if let Err(err) = refresh_lease(
                    heartbeat_context.as_ref(),
                    &heartbeat_config,
                    &heartbeat_instance,
                    &heartbeat_state,
                )
                .await
                {
                    warn!(error = ?err, "Modal session sandbox heartbeat failed");
                }
            }
        });
        let argv = vec!["sh".to_string(), "-c".to_string(), request.command.clone()];
        let result = client
            .exec(
                &state.task_id,
                &argv,
                Some(&cwd),
                &HashMap::new(),
                timeout,
                None,
            )
            .await;
        heartbeat.abort();
        let output = result.map_err(ToolExecutionResult::tool_error)?;
        refresh_lease(context, config, instance, &state).await?;

        let payload = ExecToolResultPayload::new(
            &output.stdout,
            &output.stderr,
            output.exit_code,
            &request.output_mode,
        );
        Ok(SessionSandboxExecResponse {
            exit_code: payload.exit_code,
            stdout: payload.stdout,
            stderr: payload.stderr,
            success: payload.success,
            truncated: payload.truncated || output.truncated,
            total_lines: payload.total_lines,
            raw_output: Some(payload.raw_output),
            hint: exit_code_hint(payload.exit_code).map(str::to_string),
        })
    }

    async fn read_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
        let client = client(context, config).await?;
        let state = ProviderState::from_instance(instance)?;
        let full_path = resolve_path(instance, path);
        // base64 keeps binary content intact through the UTF-8 exec streams.
        let script = format!(
            "s=$(wc -c < \"$1\") || exit 1; [ \"$s\" -le {MAX_READ_FILE_BYTES} ] || {{ echo \"file is $s bytes; the limit is {MAX_READ_FILE_BYTES}\" >&2; exit 3; }}; base64 < \"$1\" | tr -d '\\n'"
        );
        let argv = vec![
            "sh".to_string(),
            "-c".to_string(),
            script,
            "sh".to_string(),
            full_path.clone(),
        ];
        let output = client
            .exec(
                &state.task_id,
                &argv,
                None,
                &HashMap::new(),
                Duration::from_secs(120),
                None,
            )
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        if output.exit_code != 0 {
            return Err(ToolExecutionResult::tool_error(format!(
                "Failed to read {full_path}: {}",
                output.stderr.trim()
            )));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(output.stdout.trim())
            .map_err(|e| {
                ToolExecutionResult::tool_error(format!("Failed to decode {full_path}: {e}"))
            })?;
        refresh_lease(context, config, instance, &state).await?;
        let (content, encoding) = everruns_contracts::runtime::SessionFile::encode_content(&bytes);
        Ok(SessionSandboxReadFileResponse {
            path: full_path,
            content,
            encoding: encoding.to_string(),
        })
    }

    async fn write_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
        content: &[u8],
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
        let client = client(context, config).await?;
        let state = ProviderState::from_instance(instance)?;
        let full_path = resolve_path(instance, path);
        // THREAT[TM-MODAL-004]: the path is an argv value, never shell text.
        let argv = vec![
            "sh".to_string(),
            "-c".to_string(),
            "mkdir -p -- \"$(dirname -- \"$1\")\" && cat > \"$1\"".to_string(),
            "sh".to_string(),
            full_path.clone(),
        ];
        let output = client
            .exec(
                &state.task_id,
                &argv,
                None,
                &HashMap::new(),
                Duration::from_secs(120),
                Some(content),
            )
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        if output.exit_code != 0 {
            return Err(ToolExecutionResult::tool_error(format!(
                "Failed to write {full_path}: {}",
                output.stderr.trim()
            )));
        }
        refresh_lease(context, config, instance, &state).await?;
        Ok(SessionSandboxWriteFileResponse {
            path: full_path,
            bytes_written: content.len(),
        })
    }

    async fn status(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
        let client = client(context, config).await?;
        let provider_state = ProviderState::from_instance(&state.instance)?;
        let (session_status, remote_state) = if provider_state.paused {
            (finished_status(&provider_state), "paused".to_string())
        } else {
            match client.sandbox_status(&state.instance.external_id).await {
                Ok(SandboxStatus::Running) => {
                    (SessionSandboxStatus::Running, "running".to_string())
                }
                Ok(SandboxStatus::Finished { status, .. }) => (
                    finished_status(&provider_state),
                    format!("finished: {status}"),
                ),
                Err(err) if is_not_found(&err) => {
                    (finished_status(&provider_state), "not found".to_string())
                }
                Err(err) => return Err(ToolExecutionResult::tool_error(err)),
            }
        };
        Ok(SessionSandboxStatusResponse {
            provider: state.provider.clone(),
            session_status,
            external_id: state.instance.external_id.clone(),
            display_name: state.instance.display_name.clone(),
            workspace_path: state.instance.workspace_path.clone(),
            metadata: json!({
                "remote_state": remote_state,
                "runtime": provider_state.runtime,
                "snapshot_image_id": provider_state.snapshot_image_id,
            }),
        })
    }
}

/// A finished sandbox is resumable from its last pause snapshot, else lost.
fn finished_status(state: &ProviderState) -> SessionSandboxStatus {
    if state.snapshot_image_id.is_some() {
        SessionSandboxStatus::Paused
    } else {
        SessionSandboxStatus::Lost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(provider_config: Value) -> SessionSandboxConfig {
        SessionSandboxConfig {
            provider: MODAL_PROVIDER.to_string(),
            provider_config,
            ..Default::default()
        }
    }

    #[test]
    fn options_default_to_a_vm_in_workspace() {
        let options = options(&config(json!({}))).unwrap();
        assert_eq!(options.runtime, "vm");
        assert_eq!(options.workspace_path, "/workspace");
        assert_eq!(options.image, None);
    }

    #[test]
    fn options_are_validated() {
        let parsed = options(&config(json!({
            "image": "node:22", "runtime": "gvisor", "cpu": 0.5, "memory_mb": 1024,
            "workspace_path": "/srv/app"
        })))
        .unwrap();
        assert_eq!(parsed.milli_cpu, Some(500));
        assert_eq!(parsed.memory_mb, Some(1024));
        for bad in [
            json!({"runtime": "firecracker"}),
            json!({"image": "x; rm -rf /"}),
            json!({"cpu": 0.0}),
            json!({"memory_mb": 64}),
            json!({"workspace_path": "/srv/../etc"}),
            json!({"workspace_path": "relative"}),
        ] {
            assert!(options(&config(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_finished_sandbox_is_resumable_only_with_a_snapshot() {
        let mut state = ProviderState::default();
        assert_eq!(finished_status(&state), SessionSandboxStatus::Lost);
        state.snapshot_image_id = Some("im-1".into());
        assert_eq!(finished_status(&state), SessionSandboxStatus::Paused);
    }

    #[test]
    fn relative_paths_resolve_against_the_workspace() {
        let instance = SessionSandboxInstance {
            workspace_path: Some("/workspace".into()),
            ..Default::default()
        };
        assert_eq!(resolve_path(&instance, "a/b.txt"), "/workspace/a/b.txt");
        assert_eq!(resolve_path(&instance, "/etc/hosts"), "/etc/hosts");
    }

    #[test]
    fn the_provider_is_registered() {
        let provider =
            everruns_contracts::session_sandbox::create_session_sandbox_provider(MODAL_PROVIDER);
        assert_eq!(provider.map(|p| p.id().to_string()), Some("modal".into()));
    }
}
