//! boxd as a managed Sandboxes provider (`session_sandbox`, provider `boxd`).
//!
//! Decisions:
//! - Pause hibernates: memory goes to disk with every process intact, billing
//!   drops to disk only, and wake takes about 85 ms. Resume wakes the same
//!   machine, so the external id never changes. A machine someone stopped is
//!   booted again (disk kept, processes lost).
//! - boxd's own auto-suspend and auto-hibernate are off on these machines
//!   (`BoxdClient::create_machine`): they count network traffic, not CPU, so
//!   a long offline build would be hibernated mid-turn. The template's idle
//!   policy decides instead.
//! - Durability is `provider_snapshot`: the machine and its disk persist until
//!   deleted, but nothing portable leaves boxd, so physical loss loses the
//!   workspace. Sandbox Template validation enforces this.
//! - Egress rules go into the machine spec from birth, and are pushed again
//!   on resume so a resumed machine follows the current session policy.
//! - Files use boxd's filesystem API, not exec, so binary content and paths
//!   need no shell quoting.

use std::time::Duration;

use everruns_contracts::runtime::exec_tool_result::ExecToolResultPayload;
use everruns_contracts::session_sandbox::{
    OwnedSandbox, SandboxAdminError, SessionSandboxConfig, SessionSandboxContext,
    SessionSandboxExecRequest, SessionSandboxExecResponse, SessionSandboxInstance,
    SessionSandboxLease, SessionSandboxProvider, SessionSandboxReadFileResponse,
    SessionSandboxState, SessionSandboxStatus, SessionSandboxStatusResponse,
    SessionSandboxWriteFileResponse,
};
use everruns_contracts::tools::ToolExecutionResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::warn;

use super::client::{BoxdClient, CreateMachineParams, is_not_found};
use super::state::BOXD_SANDBOX_LEASE_DURATION_SECONDS;
use super::{
    BOXD_MAX_EXEC_TIMEOUT_SECS, BOXD_MAX_READ_FILE_BYTES, BOXD_PROVIDER, BOXD_REGIONS, BOXD_SIZES,
    BOXD_WORKSPACE_PATH, MachineRef, command_in, exit_code_hint, is_normalized_absolute_path,
    machine_name,
};

/// Default exec timeout when the host does not pass one.
const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(120);
/// How often a long exec refreshes the cleanup lease.
const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// The `boxd` managed Sandboxes provider.
pub struct BoxdSessionSandboxProvider;

/// Non-secret state needed to reach the machine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct ProviderState {
    region: String,
    machine_id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<String>,
}

impl ProviderState {
    fn from_instance(instance: &SessionSandboxInstance) -> Result<Self, ToolExecutionResult> {
        serde_json::from_value(instance.provider_state.clone()).map_err(|e| {
            ToolExecutionResult::internal_error_msg(format!("Corrupt boxd sandbox state: {e}"))
        })
    }

    fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
}

/// Template options (`target.options`), validated again here because the
/// provider must not trust its config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub region: Option<String>,
    pub size: Option<String>,
    pub snapshot: Option<String>,
    pub workspace_path: String,
}

fn optional_str<'a>(cfg: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match cfg.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() && value.len() <= 256 => {
            Ok(Some(value.as_str()))
        }
        Some(_) => Err(format!(
            "boxd option '{key}' must be a non-empty string no longer than 256 bytes"
        )),
    }
}

/// Parse provider options, with boxd's rules. The server calls this when a
/// template is saved, so a template never holds options creation refuses.
pub(crate) fn parse_options(cfg: &Value) -> Result<Options, String> {
    let region = optional_str(cfg, "region")?;
    if let Some(region) = region
        && !BOXD_REGIONS.contains(&region)
    {
        return Err(format!(
            "boxd option 'region' must be one of {}",
            BOXD_REGIONS.join(", ")
        ));
    }
    let size = optional_str(cfg, "size")?;
    if let Some(size) = size
        && !BOXD_SIZES.contains(&size)
    {
        return Err(format!(
            "boxd option 'size' must be one of {}",
            BOXD_SIZES.join(", ")
        ));
    }
    let snapshot = optional_str(cfg, "snapshot")?;
    if snapshot.is_some() && size.is_some() {
        // A machine from a snapshot keeps the snapshot's size.
        return Err(
            "boxd options 'snapshot' and 'size' cannot be combined: a snapshot keeps its own size"
                .to_string(),
        );
    }
    let workspace_path = match optional_str(cfg, "workspace_path")? {
        None => BOXD_WORKSPACE_PATH.to_string(),
        Some(path) if is_normalized_absolute_path(path) => path.to_string(),
        Some(_) => {
            return Err(
                "boxd option 'workspace_path' must be a normalized absolute non-root path"
                    .to_string(),
            );
        }
    };
    Ok(Options {
        region: region.map(str::to_string),
        size: size.map(str::to_string),
        snapshot: snapshot.map(str::to_string),
        workspace_path,
    })
}

pub(crate) fn options(cfg: &Value) -> Result<Options, ToolExecutionResult> {
    parse_options(cfg).map_err(ToolExecutionResult::tool_error)
}

async fn client(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
) -> Result<BoxdClient, ToolExecutionResult> {
    let key = match context
        .sandbox_connection_token(BOXD_PROVIDER, &config.credential)
        .await?
    {
        Some(key) if !key.trim().is_empty() => key,
        // THREAT[TM-AGENT-016]: never ask for credentials in chat.
        _ => return Err(ToolExecutionResult::connection_required(BOXD_PROVIDER)),
    };
    // Tests point the client at an in-process mock. Templates cannot set this
    // key (validation allowlists options) and shipped builds never read it.
    #[cfg(feature = "boxd-test-util")]
    if let Some(url) = config
        .provider_config
        .get("_test_server_url")
        .and_then(Value::as_str)
    {
        return BoxdClient::with_endpoints(key, &format!("{url}/auth/token"), url)
            .map_err(ToolExecutionResult::tool_error);
    }
    #[cfg(not(feature = "boxd-test-util"))]
    let _ = config;
    BoxdClient::new(key).map_err(ToolExecutionResult::tool_error)
}

/// Session-free client for the orphan scan.
fn admin_client(token: &str) -> Result<BoxdClient, SandboxAdminError> {
    BoxdClient::new(token).map_err(SandboxAdminError::Provider)
}

fn machine_ref(instance: &SessionSandboxInstance) -> Result<MachineRef, ToolExecutionResult> {
    MachineRef::parse(&instance.external_id).map_err(ToolExecutionResult::internal_error_msg)
}

async fn refresh_lease(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
    instance: &SessionSandboxInstance,
    state: &ProviderState,
) -> Result<(), ToolExecutionResult> {
    context
        .refresh_lease(SessionSandboxLease {
            provider: BOXD_PROVIDER.to_string(),
            external_id: instance.external_id.clone(),
            display_name: instance.display_name.clone(),
            duration_seconds: BOXD_SANDBOX_LEASE_DURATION_SECONDS,
            credential: config.credential.clone(),
            // THREAT[TM-API-015]: lease metadata is API-visible; non-secret only.
            metadata: json!({
                "region": state.region,
                "name": state.name,
                "snapshot": state.snapshot,
                "workspace_path": instance.workspace_path,
                "managed": true,
            }),
        })
        .await
}

fn resolve_path(instance: &SessionSandboxInstance, path: &str) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    let base = instance
        .workspace_path
        .as_deref()
        .unwrap_or(BOXD_WORKSPACE_PATH);
    let joined = format!("{}/{}", base.trim_end_matches('/'), path);
    joined.trim_end_matches("/.").to_string()
}

/// Create a replacement for a machine that is gone or failed, and release the
/// old one (destroying it when it still exists). The external id changes,
/// which the host already supports for replaced sandboxes.
async fn replace(
    provider: &BoxdSessionSandboxProvider,
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
    instance: &SessionSandboxInstance,
    client: &BoxdClient,
    old: &MachineRef,
) -> Result<SessionSandboxInstance, ToolExecutionResult> {
    let mut replacement = provider.create(context, config).await?;
    if let Err(err) = client.destroy(&old.region, &old.id).await
        && !is_not_found(&err)
    {
        warn!(machine = %old, "Failed to delete the failed boxd machine: {err}");
    }
    if let Err(err) = context
        .release_lease(BOXD_PROVIDER, &instance.external_id)
        .await
    {
        warn!(machine = %old, error = ?err, "Failed to release lease for replaced boxd machine");
    }
    replacement.display_name = instance.display_name.clone();
    if let Some(metadata) = replacement.metadata.as_object_mut() {
        metadata.insert("replaced".to_string(), json!(instance.external_id));
    }
    Ok(replacement)
}

/// A failed machine cannot be brought back, so it reads as lost and the host
/// replaces it.
fn session_status(status: &str) -> SessionSandboxStatus {
    match status {
        "suspended" | "hibernated" | "stopped" => SessionSandboxStatus::Paused,
        "failed" => SessionSandboxStatus::Lost,
        // running, pending, starting, migrating, and states boxd adds later.
        _ => SessionSandboxStatus::Running,
    }
}

#[async_trait::async_trait]
impl SessionSandboxProvider for BoxdSessionSandboxProvider {
    fn id(&self) -> &str {
        BOXD_PROVIDER
    }

    async fn list_owned(
        &self,
        token: &str,
        labels: &std::collections::BTreeMap<String, String>,
    ) -> Result<Vec<OwnedSandbox>, SandboxAdminError> {
        super::owned::list_owned(&admin_client(token)?, labels).await
    }

    async fn stop_by_id(&self, token: &str, external_id: &str) -> Result<(), SandboxAdminError> {
        super::owned::hibernate(&admin_client(token)?, external_id).await
    }

    async fn delete_by_id(&self, token: &str, external_id: &str) -> Result<(), SandboxAdminError> {
        super::owned::destroy(&admin_client(token)?, external_id).await
    }

    async fn create(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let options = options(&config.provider_config)?;
        let rules = super::egress::session_rules(context, config)?;
        let egress = super::egress::entries_for(&rules)?;
        let client = client(context, config).await?;
        let region = client
            .resolve_region(options.region.as_deref())
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        let labels = context
            .managed_resource_labels()
            .await
            .into_iter()
            .filter_map(|(key, value)| value.as_str().map(|v| (key, v.to_string())))
            .collect();
        let params = CreateMachineParams {
            name: machine_name(),
            size: options.size.clone(),
            snapshot: options.snapshot.clone(),
            labels,
            egress,
            delete_after_secs: None,
        };
        let machine = client
            .create_machine(&region.id, &params)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        let state = ProviderState {
            region: region.id.clone(),
            machine_id: machine.id.clone(),
            name: machine.name.clone(),
            snapshot: options.snapshot.clone(),
        };
        let display_name = config
            .provider_config
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .map(str::to_string)
            .or_else(|| Some(format!("Session Sandbox {}", context.session_id())));
        let instance = SessionSandboxInstance {
            external_id: MachineRef {
                region: region.id.clone(),
                id: machine.id.clone(),
            }
            .to_string(),
            display_name,
            workspace_path: Some(options.workspace_path.clone()),
            provider_state: state.to_value(),
            metadata: json!({
                "remote_state": machine.status,
                "region": region.id,
                "machine_name": machine.name,
                "url": machine.url,
                "restored_from_snapshot": options.snapshot.is_some(),
            }),
        };
        // A machine nothing can clean up must not outlive this call.
        if let Err(err) = refresh_lease(context, config, &instance, &state).await {
            let _ = client.destroy(&region.id, &machine.id).await;
            return Err(err);
        }
        if let Err(err) = client
            .mkdir(&region.id, &machine.id, &options.workspace_path)
            .await
        {
            warn!(machine = %machine.id, "Failed to create boxd workspace directory: {err}");
        }
        Ok(instance)
    }

    async fn resume(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let client = client(context, config).await?;
        let state = ProviderState::from_instance(instance)?;
        let machine = machine_ref(instance)?;
        let current = match client.get_machine(&machine.region, &machine.id).await {
            Ok(current) if current.status != "failed" => current,
            // Gone or broken: start over on a fresh machine, as the host does
            // for any lost sandbox, and let go of the old one.
            Ok(_) => return replace(self, context, config, instance, &client, &machine).await,
            Err(err) if is_not_found(&err) => {
                return replace(self, context, config, instance, &client, &machine).await;
            }
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        };
        match current.status.as_str() {
            "running" => {}
            "stopped" => client
                .start(&machine.region, &machine.id)
                .await
                .map_err(ToolExecutionResult::tool_error)?,
            _ => client
                .wake(&machine.region, &machine.id)
                .await
                .map_err(ToolExecutionResult::tool_error)?,
        }
        let running = client
            .wait_until_running(&machine.region, &machine.id)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        // Re-apply the current policy: the session access list may have changed.
        let rules = super::egress::session_rules(context, config)?;
        let egress = super::egress::entries_for(&rules)?;
        if running.egress_allow != egress {
            client
                .set_egress(&machine.region, &machine.id, &egress)
                .await
                .map_err(|error| {
                    ToolExecutionResult::tool_error(format!(
                        "boxd refused the sandbox network policy: {error}"
                    ))
                })?;
        }
        refresh_lease(context, config, instance, &state).await?;
        Ok(SessionSandboxInstance {
            metadata: json!({
                "remote_state": "running",
                "region": state.region,
                "machine_name": state.name,
                "url": running.url,
            }),
            ..instance.clone()
        })
    }

    async fn pause(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let client = client(context, config).await?;
        let state = ProviderState::from_instance(instance)?;
        let machine = machine_ref(instance)?;
        let remote_state = match client.get_machine(&machine.region, &machine.id).await {
            Ok(current) if matches!(current.status.as_str(), "hibernated" | "stopped") => {
                current.status
            }
            Ok(_) => {
                client
                    .hibernate(&machine.region, &machine.id)
                    .await
                    .map_err(ToolExecutionResult::tool_error)?;
                "hibernated".to_string()
            }
            Err(err) if is_not_found(&err) => "lost".to_string(),
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        };
        // Hibernated costs disk only, and the lease keeps the machine owned.
        refresh_lease(context, config, instance, &state).await?;
        Ok(SessionSandboxInstance {
            metadata: json!({
                "remote_state": remote_state,
                "region": state.region,
                "machine_name": state.name,
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
        let machine = machine_ref(instance)?;
        match client.destroy(&machine.region, &machine.id).await {
            Ok(()) => {}
            Err(err) if is_not_found(&err) => {}
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        }
        context
            .release_lease(BOXD_PROVIDER, &instance.external_id)
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
        let machine = machine_ref(instance)?;
        let cwd = resolve_path(instance, request.cwd.as_deref().unwrap_or("."));
        let timeout = request
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_EXEC_TIMEOUT)
            .min(Duration::from_secs(u64::from(BOXD_MAX_EXEC_TIMEOUT_SECS)));

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
                    warn!(error = ?err, "boxd session sandbox heartbeat failed");
                }
            }
        });
        let result = client
            .exec(
                &machine.region,
                &machine.id,
                &command_in(&cwd, &request.command),
                None,
                timeout,
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
        let machine = machine_ref(instance)?;
        let full_path = resolve_path(instance, path);
        let bytes = client
            .read_file(
                &machine.region,
                &machine.id,
                &full_path,
                BOXD_MAX_READ_FILE_BYTES,
            )
            .await
            .map_err(|e| {
                ToolExecutionResult::tool_error(format!("Failed to read {full_path}: {e}"))
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
        let machine = machine_ref(instance)?;
        let full_path = resolve_path(instance, path);
        client
            .write_file(&machine.region, &machine.id, &full_path, content)
            .await
            .map_err(|e| {
                ToolExecutionResult::tool_error(format!("Failed to write {full_path}: {e}"))
            })?;
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
        let machine = machine_ref(&state.instance)?;
        let (session_status, remote_state, url) =
            match client.get_machine(&machine.region, &machine.id).await {
                Ok(current) => (
                    self::session_status(&current.status),
                    current.status,
                    current.url,
                ),
                Err(err) if is_not_found(&err) => {
                    (SessionSandboxStatus::Lost, "not found".to_string(), None)
                }
                Err(err) => return Err(ToolExecutionResult::tool_error(err)),
            };
        Ok(SessionSandboxStatusResponse {
            provider: state.provider.clone(),
            session_status,
            external_id: state.instance.external_id.clone(),
            display_name: state.instance.display_name.clone(),
            workspace_path: state.instance.workspace_path.clone(),
            metadata: json!({
                "remote_state": remote_state,
                "region": provider_state.region,
                "machine_name": provider_state.name,
                "url": url,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_validated() {
        let parsed = options(&json!({
            "region": "nyc", "size": "4x16", "workspace_path": "/srv/app"
        }))
        .unwrap();
        assert_eq!(parsed.region.as_deref(), Some("nyc"));
        assert_eq!(parsed.size.as_deref(), Some("4x16"));
        assert_eq!(parsed.workspace_path, "/srv/app");
        assert_eq!(
            options(&json!({})).unwrap().workspace_path,
            BOXD_WORKSPACE_PATH
        );
        for bad in [
            json!({"region": "lon"}),
            json!({"region": 1}),
            json!({"size": "3x12"}),
            json!({"snapshot": "base", "size": "2x8"}),
            json!({"snapshot": ""}),
            json!({"workspace_path": "/srv/../etc"}),
            json!({"workspace_path": "relative"}),
        ] {
            assert!(options(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn statuses_map_to_session_states() {
        assert_eq!(session_status("running"), SessionSandboxStatus::Running);
        assert_eq!(session_status("starting"), SessionSandboxStatus::Running);
        assert_eq!(session_status("hibernated"), SessionSandboxStatus::Paused);
        assert_eq!(session_status("suspended"), SessionSandboxStatus::Paused);
        assert_eq!(session_status("stopped"), SessionSandboxStatus::Paused);
        assert_eq!(session_status("failed"), SessionSandboxStatus::Lost);
    }

    #[test]
    fn relative_paths_resolve_against_the_workspace() {
        let instance = SessionSandboxInstance {
            workspace_path: Some(BOXD_WORKSPACE_PATH.into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_path(&instance, "a/b.txt"),
            "/home/boxd/workspace/a/b.txt"
        );
        assert_eq!(resolve_path(&instance, "."), "/home/boxd/workspace");
        assert_eq!(resolve_path(&instance, "/etc/hosts"), "/etc/hosts");
    }
}
