//! Provider-neutral managed session sandbox contracts.

use crate::tools::ToolExecutionResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;
use uuid::Uuid;

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
    /// Whether the control plane should pause this sandbox when the timeout elapses.
    #[serde(default = "default_true")]
    pub idle_pause_enabled: bool,
    /// Credential owner pinned when the Session is created. This contains only
    /// stable identifiers; credential material remains in the host store.
    #[serde(default)]
    pub credential: SessionSandboxCredential,
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
            idle_pause_enabled: true,
            credential: SessionSandboxCredential::default(),
            provider_config: default_provider_config(),
            init: SessionSandboxInitConfig::default(),
        }
    }
}

/// Where a managed Sandbox obtains its provider credential.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionSandboxCredentialSource {
    #[default]
    None,
    SessionUser,
    Agent,
    Organization,
}

/// Non-secret, session-pinned credential binding.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionSandboxCredential {
    pub source: SessionSandboxCredentialSource,
    /// Exact virtual-user owner for user, Agent, and organization grants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_user_id: Option<Uuid>,
    /// Exact connection for organization grants, which may have several
    /// accounts for one provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<Uuid>,
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

/// Provider-owned physical environment instance record.
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

/// Persisted managed environment state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSandboxState {
    /// Hosted logical-sandbox identity and incarnation fence. Legacy secret
    /// records omit it and acquire one when they are adopted into durable
    /// control-plane state.
    #[serde(skip)]
    pub sandbox: Option<crate::sandbox_checkpoint::SandboxRef>,
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

/// Session-scoped services used by a sandbox provider. Hosts supply the adapter;
/// provider implementations never need a runtime or control-plane context type.
#[async_trait::async_trait]
pub trait SessionSandboxContext: Send + Sync {
    fn session_id(&self) -> crate::typed_id::SessionId;

    /// Clone the service handle for long-running lease heartbeats.
    fn clone_context(&self) -> std::sync::Arc<dyn SessionSandboxContext>;

    async fn connection_token(&self, provider: &str)
    -> Result<Option<String>, ToolExecutionResult>;

    /// Resolve the provider credential selected by the Session sandbox policy.
    /// Unlike ordinary tool connections, this never falls back to another
    /// identity or account.
    async fn sandbox_connection_token(
        &self,
        provider: &str,
        credential: &SessionSandboxCredential,
    ) -> Result<Option<String>, ToolExecutionResult>;

    /// Non-secret labels for resources created on behalf of this session.
    async fn resource_labels(&self) -> serde_json::Map<String, Value>;

    /// Encrypted session-secret storage for provider bootstrap credentials that
    /// must survive pause/resume but must never enter the non-secret Sandbox
    /// snapshot.
    async fn get_provider_secret(
        &self,
        _name: &str,
    ) -> Result<Option<String>, ToolExecutionResult> {
        Ok(None)
    }

    async fn set_provider_secret(
        &self,
        _name: &str,
        _value: &str,
    ) -> Result<(), ToolExecutionResult> {
        Err(ToolExecutionResult::internal_error_msg(
            "Sandbox provider secret storage is unavailable",
        ))
    }

    async fn delete_provider_secret(&self, _name: &str) -> Result<(), ToolExecutionResult> {
        Ok(())
    }

    /// Refresh the cleanup lease, when the host supports leased resources.
    async fn refresh_lease(&self, lease: SessionSandboxLease) -> Result<(), ToolExecutionResult>;

    async fn release_lease(
        &self,
        provider: &str,
        external_id: &str,
    ) -> Result<(), ToolExecutionResult>;
}

/// Non-secret resource metadata for a sandbox cleanup lease. The context resolves
/// the owner from its bound connection rather than accepting ownership input.
#[derive(Debug, Clone)]
pub struct SessionSandboxLease {
    pub provider: String,
    pub external_id: String,
    pub display_name: Option<String>,
    pub duration_seconds: u32,
    pub credential: SessionSandboxCredential,
    pub metadata: Value,
}

/// Provider trait implemented by integration crates.
#[async_trait::async_trait]
pub trait SessionSandboxProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn create(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult>;

    async fn resume(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult>;

    async fn pause(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult>;

    async fn delete(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult>;

    async fn exec(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        request: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult>;

    async fn read_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult>;

    async fn write_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
        content: &[u8],
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult>;

    /// Persist the current workspace after a completed mutating operation.
    /// Providers without disposable local filesystems may keep the default.
    async fn checkpoint(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        Ok(instance.clone())
    }

    /// Point the provider's recovery state at `revision`, or clear it when
    /// `None`, without touching the running workspace.
    ///
    /// Reconciliation decides which revision a session may recover to from the
    /// checkpoint records; this is how that decision reaches the provider state
    /// the restore actually reads. Providers with no recovery state keep the
    /// default: there is nothing to rewind, and reconciliation is a no-op for
    /// them.
    async fn rewind_checkpoint(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        _revision: Option<&str>,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        Ok(instance.clone())
    }

    async fn status(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult>;
}

/// Inventory registration point for concrete session sandbox providers.
#[cfg(feature = "session-sandbox")]
pub struct SessionSandboxProviderPlugin {
    pub factory: fn() -> Box<dyn SessionSandboxProvider>,
}

#[cfg(feature = "session-sandbox")]
inventory::collect!(SessionSandboxProviderPlugin);

/// Look up a registered provider by id.
#[cfg(feature = "session-sandbox")]
pub fn create_session_sandbox_provider(
    provider_id: &str,
) -> Option<Box<dyn SessionSandboxProvider>> {
    inventory::iter::<SessionSandboxProviderPlugin>
        .into_iter()
        .map(|plugin| (plugin.factory)())
        .find(|provider| provider.id() == provider_id)
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
