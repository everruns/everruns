//! Session-owned managed sandbox abstractions.
//!
//! `session_sandbox` is the provider-neutral contract for "one managed sandbox
//! per session". The capability in `capabilities/session_sandbox.rs` exposes
//! generic `sandbox_*` tools, while integration crates register concrete
//! providers (Daytona first) through the inventory plugin system below.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use everruns_core::capability_types::AgentCapabilityConfig;
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::ToolExecutionResult;

/// Capability id for the managed session sandbox capability.
pub const SESSION_SANDBOX_CAPABILITY_ID: &str = "session_sandbox";
/// Secret name used to persist the managed sandbox record for a session.
pub const SESSION_SANDBOX_SECRET_NAME: &str = "session_sandbox";
/// Default idle timeout for auto-pausing the managed sandbox.
pub const DEFAULT_SESSION_SANDBOX_IDLE_TIMEOUT_SECS: u64 = 180;

/// Session sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSandboxConfig {
    /// Concrete provider id (e.g. `daytona`).
    pub provider: String,
    /// Start the sandbox proactively when the session is created.
    #[serde(default = "default_true")]
    pub auto_start: bool,
    /// Pause the sandbox after this much session inactivity.
    #[serde(default = "default_idle_timeout")]
    pub idle_pause_after_seconds: u64,
    /// Provider-specific extra configuration.
    #[serde(default = "default_provider_config")]
    pub provider_config: Value,
    /// Optional one-time initialization commands executed after create.
    #[serde(default)]
    pub init: SessionSandboxInitConfig,
}

impl Default for SessionSandboxConfig {
    fn default() -> Self {
        Self {
            provider: String::new(),
            auto_start: true,
            idle_pause_after_seconds: DEFAULT_SESSION_SANDBOX_IDLE_TIMEOUT_SECS,
            provider_config: default_provider_config(),
            init: SessionSandboxInitConfig::default(),
        }
    }
}

/// One-time sandbox initialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSandboxInitConfig {
    #[serde(default)]
    pub commands: Vec<String>,
}

/// Runtime lifecycle status of the managed sandbox.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionSandboxStatus {
    Running,
    Paused,
    /// The provider resource disappeared and must be replaced before use.
    Lost,
}

/// Provider-owned sandbox instance record persisted in session secrets.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionSandboxInstance {
    /// Provider-specific stable identifier (e.g. Daytona sandbox id).
    pub external_id: String,
    /// Optional human-readable name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Optional default workspace path inside the sandbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    /// Provider-specific non-secret payload needed for resume/ops.
    #[serde(default)]
    pub provider_state: Value,
    /// Provider-specific non-secret metadata for UI/debugging.
    #[serde(default)]
    pub metadata: Value,
}

/// Persisted managed sandbox state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSandboxState {
    pub provider: String,
    pub status: SessionSandboxStatus,
    pub instance: SessionSandboxInstance,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init_completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_init_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Provider-neutral exec request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSandboxExecRequest {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default = "default_output_mode")]
    pub output_mode: String,
}

/// Provider-neutral exec result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSandboxExecResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub truncated: bool,
    pub total_lines: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Provider-neutral file read result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSandboxReadFileResponse {
    pub path: String,
    pub content: String,
    pub encoding: String,
}

/// Provider-neutral file write result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSandboxWriteFileResponse {
    pub path: String,
    pub bytes_written: usize,
}

/// Provider-neutral status view.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSandboxStatusResponse {
    pub provider: String,
    pub session_status: SessionSandboxStatus,
    pub external_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// Provider trait implemented by integration crates.
#[async_trait::async_trait]
pub trait SessionSandboxProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn create(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult>;

    async fn resume(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult>;

    async fn pause(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult>;

    async fn delete(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult>;

    async fn exec(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        request: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult>;

    async fn read_file(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult>;

    async fn write_file(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
        content: &str,
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult>;

    /// Persist the current workspace after a completed mutating operation.
    /// Providers without disposable local filesystems may keep the default.
    async fn checkpoint(
        &self,
        _context: &ToolContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        Ok(instance.clone())
    }

    async fn status(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult>;
}

/// Inventory registration point for concrete session sandbox providers.
pub struct SessionSandboxProviderPlugin {
    pub factory: fn() -> Box<dyn SessionSandboxProvider>,
}

inventory::collect!(SessionSandboxProviderPlugin);

/// Look up a registered provider by id.
pub fn create_session_sandbox_provider(
    provider_id: &str,
) -> Option<Box<dyn SessionSandboxProvider>> {
    inventory::iter::<SessionSandboxProviderPlugin>
        .into_iter()
        .map(|plugin| (plugin.factory)())
        .find(|provider| provider.id() == provider_id)
}

/// Extract the effective session sandbox config from a capability list.
pub fn session_sandbox_config_from_capabilities(
    capability_configs: &[AgentCapabilityConfig],
) -> Result<Option<SessionSandboxConfig>, String> {
    let Some(capability) = capability_configs
        .iter()
        .find(|cap| cap.capability_id() == SESSION_SANDBOX_CAPABILITY_ID)
    else {
        return Ok(None);
    };

    let config: SessionSandboxConfig = serde_json::from_value(capability.config_value().clone())
        .map_err(|e| format!("Invalid session_sandbox config: {e}"))?;

    if config.provider.trim().is_empty() {
        return Err("session_sandbox config requires non-empty 'provider'".to_string());
    }
    if config.idle_pause_after_seconds == 0 {
        return Err("session_sandbox config requires idle_pause_after_seconds >= 1".to_string());
    }

    Ok(Some(config))
}

/// Load the managed sandbox state from session secret storage.
pub async fn load_session_sandbox_state(
    context: &ToolContext,
) -> Result<Option<SessionSandboxState>, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let Some(raw) = storage
        .get_secret(context.session_id, SESSION_SANDBOX_SECRET_NAME)
        .await
        .map_err(ToolExecutionResult::internal_error)?
    else {
        return Ok(None);
    };

    let state: SessionSandboxState = serde_json::from_str(&raw).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Corrupt session sandbox state: {e}"))
    })?;
    Ok(Some(state))
}

/// Persist the managed sandbox state into session secret storage.
pub async fn save_session_sandbox_state(
    context: &ToolContext,
    state: &SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let raw = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!(
            "Failed to encode session sandbox state: {e}"
        ))
    })?;

    storage
        .set_secret(context.session_id, SESSION_SANDBOX_SECRET_NAME, &raw)
        .await
        .map_err(ToolExecutionResult::internal_error)
}

/// Delete the managed sandbox state from session secret storage.
pub async fn delete_session_sandbox_state(
    context: &ToolContext,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    storage
        .delete_secret(context.session_id, SESSION_SANDBOX_SECRET_NAME)
        .await
        .map_err(ToolExecutionResult::internal_error)?;
    Ok(())
}

/// Start the managed sandbox when absent and resume it when paused.
pub async fn ensure_session_sandbox_running(
    context: &ToolContext,
    config: &SessionSandboxConfig,
) -> Result<SessionSandboxState, ToolExecutionResult> {
    let Some(provider) = create_session_sandbox_provider(&config.provider) else {
        return Err(ToolExecutionResult::tool_error(format!(
            "Session sandbox provider '{}' is not registered",
            config.provider
        )));
    };

    match load_session_sandbox_state(context).await? {
        Some(existing) => {
            if existing.provider != config.provider {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Session sandbox provider mismatch: state has '{}', config requests '{}'",
                    existing.provider, config.provider
                )));
            }

            let mut state = existing;
            let needs_resume = match state.status {
                SessionSandboxStatus::Paused | SessionSandboxStatus::Lost => true,
                SessionSandboxStatus::Running => {
                    let status = provider.status(context, config, &state).await?;
                    status.session_status != SessionSandboxStatus::Running
                }
            };

            if needs_resume {
                state.instance = provider.resume(context, config, &state.instance).await?;
                state.status = SessionSandboxStatus::Running;
                state.last_init_error = None;
                state.updated_at = now_rfc3339();
                save_session_sandbox_state(context, &state).await?;
            }

            run_session_sandbox_init_if_needed(context, provider.as_ref(), config, &mut state)
                .await?;
            Ok(state)
        }
        None => {
            let instance = provider.create(context, config).await?;
            let mut state = SessionSandboxState {
                provider: config.provider.clone(),
                status: SessionSandboxStatus::Running,
                instance,
                init_completed_at: None,
                last_init_error: None,
                created_at: now_rfc3339(),
                updated_at: now_rfc3339(),
            };
            save_session_sandbox_state(context, &state).await?;
            run_session_sandbox_init_if_needed(context, provider.as_ref(), config, &mut state)
                .await?;
            Ok(state)
        }
    }
}

/// Pause the managed sandbox if it exists and is running.
pub async fn pause_session_sandbox(
    context: &ToolContext,
    config: &SessionSandboxConfig,
) -> Result<Option<SessionSandboxState>, ToolExecutionResult> {
    let Some(mut state) = load_session_sandbox_state(context).await? else {
        return Ok(None);
    };

    if state.provider != config.provider {
        return Err(ToolExecutionResult::tool_error(format!(
            "Session sandbox provider mismatch: state has '{}', config requests '{}'",
            state.provider, config.provider
        )));
    }
    if state.status == SessionSandboxStatus::Paused {
        return Ok(Some(state));
    }

    let Some(provider) = create_session_sandbox_provider(&config.provider) else {
        return Err(ToolExecutionResult::tool_error(format!(
            "Session sandbox provider '{}' is not registered",
            config.provider
        )));
    };

    state.instance = provider.pause(context, config, &state.instance).await?;
    state.status = SessionSandboxStatus::Paused;
    state.updated_at = now_rfc3339();
    save_session_sandbox_state(context, &state).await?;
    Ok(Some(state))
}

/// Delete the managed sandbox if it exists.
pub async fn delete_session_sandbox(
    context: &ToolContext,
    config: &SessionSandboxConfig,
) -> Result<bool, ToolExecutionResult> {
    let Some(state) = load_session_sandbox_state(context).await? else {
        return Ok(false);
    };

    if state.provider != config.provider {
        return Err(ToolExecutionResult::tool_error(format!(
            "Session sandbox provider mismatch: state has '{}', config requests '{}'",
            state.provider, config.provider
        )));
    }

    let Some(provider) = create_session_sandbox_provider(&config.provider) else {
        return Err(ToolExecutionResult::tool_error(format!(
            "Session sandbox provider '{}' is not registered",
            config.provider
        )));
    };

    provider.delete(context, config, &state.instance).await?;
    delete_session_sandbox_state(context).await?;
    Ok(true)
}

/// Execute one-time initialization commands when configured and not yet completed.
pub async fn run_session_sandbox_init_if_needed(
    context: &ToolContext,
    provider: &dyn SessionSandboxProvider,
    config: &SessionSandboxConfig,
    state: &mut SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    if state.init_completed_at.is_some() || config.init.commands.is_empty() {
        return Ok(());
    }

    for command in &config.init.commands {
        let response = provider
            .exec(
                context,
                config,
                &state.instance,
                &SessionSandboxExecRequest {
                    command: command.clone(),
                    cwd: state.instance.workspace_path.clone(),
                    timeout_ms: None,
                    output_mode: "concise".to_string(),
                },
            )
            .await?;

        checkpoint_session_sandbox(context, provider, config, state).await?;

        if response.exit_code != 0 {
            state.last_init_error = Some(format!(
                "Init command failed with exit code {}: {}",
                response.exit_code, command
            ));
            state.updated_at = now_rfc3339();
            save_session_sandbox_state(context, state).await?;
            return Err(ToolExecutionResult::tool_error(format!(
                "Session sandbox init failed for command '{}': {}",
                command,
                if response.stderr.is_empty() {
                    response.stdout
                } else if response.stdout.is_empty() {
                    response.stderr
                } else {
                    format!("{}\n{}", response.stdout, response.stderr)
                }
            )));
        }
    }

    state.init_completed_at = Some(now_rfc3339());
    state.last_init_error = None;
    state.updated_at = now_rfc3339();
    save_session_sandbox_state(context, state).await?;
    Ok(())
}

/// Persist the provider checkpoint binding before the mutating tool result is
/// returned to the runtime.
pub async fn checkpoint_session_sandbox(
    context: &ToolContext,
    provider: &dyn SessionSandboxProvider,
    config: &SessionSandboxConfig,
    state: &mut SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    let checkpointed = provider
        .checkpoint(context, config, &state.instance)
        .await?;
    if checkpointed != state.instance {
        state.instance = checkpointed;
        state.updated_at = now_rfc3339();
        save_session_sandbox_state(context, state).await?;
    }
    Ok(())
}

/// Shared hints for stateful remote sandbox tools.
pub fn session_sandbox_tool_hints() -> ToolHints {
    ToolHints::default()
        .with_concurrency_class("session_sandbox")
        .with_open_world(true)
        .with_requires_secrets(true)
        .with_long_running(true)
}

fn default_true() -> bool {
    true
}

fn default_idle_timeout() -> u64 {
    DEFAULT_SESSION_SANDBOX_IDLE_TIMEOUT_SECS
}

fn default_provider_config() -> Value {
    json!({})
}

fn default_output_mode() -> String {
    // EVE-489: persistence-first default for exec-style sandbox tools.
    "auto".to_string()
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use everruns_core::{session_services::SecretInfo, session_services::SessionStorageStore};
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock, Mutex};

    #[derive(Clone, Default)]
    struct MemorySecrets {
        secrets: Arc<Mutex<HashMap<String, String>>>,
    }

    #[async_trait]
    impl SessionStorageStore for MemorySecrets {
        async fn set_value(
            &self,
            _session_id: everruns_core::SessionId,
            _key: &str,
            _value: &str,
        ) -> everruns_core::Result<()> {
            unreachable!()
        }

        async fn get_value(
            &self,
            _session_id: everruns_core::SessionId,
            _key: &str,
        ) -> everruns_core::Result<Option<String>> {
            unreachable!()
        }

        async fn delete_value(
            &self,
            _session_id: everruns_core::SessionId,
            _key: &str,
        ) -> everruns_core::Result<bool> {
            unreachable!()
        }

        async fn list_keys(
            &self,
            _session_id: everruns_core::SessionId,
        ) -> everruns_core::Result<Vec<everruns_core::session_services::KeyInfo>> {
            unreachable!()
        }

        async fn set_secret(
            &self,
            _session_id: everruns_core::SessionId,
            name: &str,
            value: &str,
        ) -> everruns_core::Result<()> {
            self.secrets
                .lock()
                .unwrap()
                .insert(name.to_string(), value.to_string());
            Ok(())
        }

        async fn get_secret(
            &self,
            _session_id: everruns_core::SessionId,
            name: &str,
        ) -> everruns_core::Result<Option<String>> {
            Ok(self.secrets.lock().unwrap().get(name).cloned())
        }

        async fn delete_secret(
            &self,
            _session_id: everruns_core::SessionId,
            name: &str,
        ) -> everruns_core::Result<bool> {
            Ok(self.secrets.lock().unwrap().remove(name).is_some())
        }

        async fn list_secrets(
            &self,
            _session_id: everruns_core::SessionId,
        ) -> everruns_core::Result<Vec<SecretInfo>> {
            Ok(self
                .secrets
                .lock()
                .unwrap()
                .keys()
                .map(|name| SecretInfo {
                    name: name.clone(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                })
                .collect())
        }
    }

    #[test]
    fn extracts_valid_session_sandbox_config() {
        let config =
            session_sandbox_config_from_capabilities(&[AgentCapabilityConfig::with_config(
                SESSION_SANDBOX_CAPABILITY_ID,
                serde_json::json!({
                    "provider": "daytona",
                    "auto_start": false,
                    "idle_pause_after_seconds": 90,
                    "init": { "commands": ["echo ready"] }
                }),
            )])
            .unwrap()
            .unwrap();

        assert_eq!(config.provider, "daytona");
        assert!(!config.auto_start);
        assert_eq!(config.idle_pause_after_seconds, 90);
        assert_eq!(config.provider_config, json!({}));
        assert_eq!(config.init.commands, vec!["echo ready"]);
    }

    #[test]
    fn rejects_missing_provider() {
        let err = session_sandbox_config_from_capabilities(&[AgentCapabilityConfig::with_config(
            SESSION_SANDBOX_CAPABILITY_ID,
            serde_json::json!({ "auto_start": true }),
        )])
        .unwrap_err();

        assert!(err.contains("provider"));
    }

    #[tokio::test]
    async fn session_sandbox_state_round_trip() {
        let storage = Arc::new(MemorySecrets::default());
        let context = ToolContext::with_storage_store(everruns_core::SessionId::new(), storage);

        let state = SessionSandboxState {
            provider: "daytona".to_string(),
            status: SessionSandboxStatus::Running,
            instance: SessionSandboxInstance {
                external_id: "sb_test".to_string(),
                display_name: Some("Sandbox".to_string()),
                workspace_path: Some("/home/daytona".to_string()),
                provider_state: serde_json::json!({"sandbox_id":"sb_test"}),
                metadata: serde_json::json!({"state":"started"}),
            },
            init_completed_at: Some(now_rfc3339()),
            last_init_error: None,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };

        save_session_sandbox_state(&context, &state).await.unwrap();
        let loaded = load_session_sandbox_state(&context).await.unwrap().unwrap();
        assert_eq!(loaded.provider, "daytona");
        assert_eq!(loaded.instance.external_id, "sb_test");
        assert_eq!(loaded.status, SessionSandboxStatus::Running);
    }

    #[derive(Clone)]
    struct TestProviderSandboxState {
        remote_status: SessionSandboxStatus,
        resume_calls: usize,
        exec_commands: Vec<String>,
    }

    #[derive(Default)]
    struct TestProviderState {
        sandboxes: HashMap<String, TestProviderSandboxState>,
    }

    static TEST_PROVIDER_STATE: LazyLock<Mutex<TestProviderState>> =
        LazyLock::new(|| Mutex::new(TestProviderState::default()));

    fn sandbox_state_mut<'a>(
        state: &'a mut TestProviderState,
        external_id: &str,
    ) -> &'a mut TestProviderSandboxState {
        state
            .sandboxes
            .entry(external_id.to_string())
            .or_insert_with(|| TestProviderSandboxState {
                remote_status: SessionSandboxStatus::Running,
                resume_calls: 0,
                exec_commands: Vec::new(),
            })
    }

    fn test_provider_state(external_id: &str) -> TestProviderSandboxState {
        TEST_PROVIDER_STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .sandboxes
            .get(external_id)
            .cloned()
            .unwrap_or(TestProviderSandboxState {
                remote_status: SessionSandboxStatus::Running,
                resume_calls: 0,
                exec_commands: Vec::new(),
            })
    }

    fn reset_test_provider_state(external_id: &str, remote_status: SessionSandboxStatus) {
        let mut state = TEST_PROVIDER_STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.sandboxes.insert(
            external_id.to_string(),
            TestProviderSandboxState {
                remote_status,
                resume_calls: 0,
                exec_commands: Vec::new(),
            },
        );
    }

    struct CoreTestSessionSandboxProvider;

    inventory::submit! {
        SessionSandboxProviderPlugin {
            factory: || Box::new(CoreTestSessionSandboxProvider),
        }
    }

    #[async_trait]
    impl SessionSandboxProvider for CoreTestSessionSandboxProvider {
        fn id(&self) -> &str {
            "core-test-session-sandbox"
        }

        async fn create(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
        ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
            let instance = test_instance("sb_created");
            let mut state = TEST_PROVIDER_STATE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            sandbox_state_mut(&mut state, &instance.external_id).remote_status =
                SessionSandboxStatus::Running;
            Ok(instance)
        }

        async fn resume(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
            let mut state = TEST_PROVIDER_STATE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let sandbox_state = sandbox_state_mut(&mut state, &instance.external_id);
            sandbox_state.resume_calls += 1;
            sandbox_state.remote_status = SessionSandboxStatus::Running;

            let mut resumed = instance.clone();
            resumed.metadata = json!({ "resumed": true });
            Ok(resumed)
        }

        async fn pause(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
            Ok(instance.clone())
        }

        async fn delete(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
        ) -> Result<(), ToolExecutionResult> {
            Ok(())
        }

        async fn exec(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            request: &SessionSandboxExecRequest,
        ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
            let mut state = TEST_PROVIDER_STATE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            sandbox_state_mut(&mut state, &_instance.external_id)
                .exec_commands
                .push(request.command.clone());

            Ok(SessionSandboxExecResponse {
                exit_code: 0,
                stdout: "ok".to_string(),
                stderr: String::new(),
                success: true,
                truncated: false,
                total_lines: 1,
                raw_output: Some("ok".to_string()),
                hint: None,
            })
        }

        async fn read_file(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            path: &str,
        ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
            Ok(SessionSandboxReadFileResponse {
                path: path.to_string(),
                content: "data".to_string(),
                encoding: "text".to_string(),
            })
        }

        async fn write_file(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            path: &str,
            content: &str,
        ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
            Ok(SessionSandboxWriteFileResponse {
                path: path.to_string(),
                bytes_written: content.len(),
            })
        }

        async fn status(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            state: &SessionSandboxState,
        ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
            let provider_state = test_provider_state(&state.instance.external_id);

            Ok(SessionSandboxStatusResponse {
                provider: state.provider.clone(),
                session_status: provider_state.remote_status,
                external_id: state.instance.external_id.clone(),
                display_name: state.instance.display_name.clone(),
                workspace_path: state.instance.workspace_path.clone(),
                metadata: json!({ "remote_status": provider_state.remote_status }),
            })
        }
    }

    fn test_instance(external_id: &str) -> SessionSandboxInstance {
        SessionSandboxInstance {
            external_id: external_id.to_string(),
            display_name: Some("Core Test Sandbox".to_string()),
            workspace_path: Some("/workspace".to_string()),
            provider_state: json!({}),
            metadata: json!({}),
        }
    }

    fn test_config_with_init(commands: Vec<&str>) -> SessionSandboxConfig {
        SessionSandboxConfig {
            provider: "core-test-session-sandbox".to_string(),
            auto_start: true,
            idle_pause_after_seconds: 180,
            provider_config: json!({}),
            init: SessionSandboxInitConfig {
                commands: commands.into_iter().map(ToString::to_string).collect(),
            },
        }
    }

    #[tokio::test]
    async fn ensure_running_resumes_when_remote_status_drifted_to_paused() {
        let external_id = "sb_drifted";
        reset_test_provider_state(external_id, SessionSandboxStatus::Paused);

        let storage = Arc::new(MemorySecrets::default());
        let context = ToolContext::with_storage_store(everruns_core::SessionId::new(), storage);
        let state = SessionSandboxState {
            provider: "core-test-session-sandbox".to_string(),
            status: SessionSandboxStatus::Running,
            instance: test_instance(external_id),
            init_completed_at: Some(now_rfc3339()),
            last_init_error: None,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };
        save_session_sandbox_state(&context, &state).await.unwrap();

        let resolved = ensure_session_sandbox_running(&context, &test_config_with_init(vec![]))
            .await
            .unwrap();

        let provider_state = test_provider_state(external_id);
        assert_eq!(provider_state.resume_calls, 1);
        assert_eq!(resolved.status, SessionSandboxStatus::Running);
        assert_eq!(resolved.instance.metadata, json!({ "resumed": true }));
    }

    #[tokio::test]
    async fn ensure_running_replaces_provider_instance_when_remote_status_is_lost() {
        let external_id = "sb_lost";
        reset_test_provider_state(external_id, SessionSandboxStatus::Lost);

        let storage = Arc::new(MemorySecrets::default());
        let context = ToolContext::with_storage_store(everruns_core::SessionId::new(), storage);
        let state = SessionSandboxState {
            provider: "core-test-session-sandbox".to_string(),
            status: SessionSandboxStatus::Running,
            instance: test_instance(external_id),
            init_completed_at: Some(now_rfc3339()),
            last_init_error: None,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };
        save_session_sandbox_state(&context, &state).await.unwrap();

        let resolved = ensure_session_sandbox_running(&context, &test_config_with_init(vec![]))
            .await
            .unwrap();

        let provider_state = test_provider_state(external_id);
        assert_eq!(provider_state.resume_calls, 1);
        assert_eq!(resolved.status, SessionSandboxStatus::Running);
        assert_eq!(resolved.instance.metadata, json!({ "resumed": true }));
    }

    #[tokio::test]
    async fn ensure_running_retries_init_when_state_is_running_but_init_unfinished() {
        let external_id = "sb_init_retry";
        reset_test_provider_state(external_id, SessionSandboxStatus::Running);

        let storage = Arc::new(MemorySecrets::default());
        let context = ToolContext::with_storage_store(everruns_core::SessionId::new(), storage);
        let state = SessionSandboxState {
            provider: "core-test-session-sandbox".to_string(),
            status: SessionSandboxStatus::Running,
            instance: test_instance(external_id),
            init_completed_at: None,
            last_init_error: Some("previous failure".to_string()),
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };
        save_session_sandbox_state(&context, &state).await.unwrap();

        let resolved =
            ensure_session_sandbox_running(&context, &test_config_with_init(vec!["echo ready"]))
                .await
                .unwrap();

        let provider_state = test_provider_state(external_id);
        assert_eq!(provider_state.exec_commands, vec!["echo ready"]);
        assert!(resolved.init_completed_at.is_some());
        assert_eq!(resolved.last_init_error, None);
    }
}
