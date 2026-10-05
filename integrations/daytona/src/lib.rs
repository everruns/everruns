#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Daytona cloud sandboxes for Everruns agents.
//!
//! `everruns-integrations-daytona` is part of the
//! [Everruns](https://everruns.com) ecosystem. It adds cloud-based sandboxed
//! code execution backed by the [Daytona](https://www.daytona.io) REST API,
//! letting agents create sandboxes, run commands with streamed output, and read
//! or write files inside an isolated environment. Sandboxes are managed per
//! session and authenticated with a user-supplied Daytona API key.
//!
//! # Example
//!
//! ```
//! use everruns_contracts::runtime::capabilities::Capability;
//! use everruns_integrations_daytona::DaytonaCapability;
//!
//! let capability = DaytonaCapability;
//! assert_eq!(capability.id(), "daytona");
//! ```
//!
//! # Design notes
//!
//! - External integration crate, auto-registered via the inventory plugin system.
//! - State (API key + per-sandbox connection info) lives in the secrets store.
//! - Two-tier Daytona API: Management API for lifecycle, Toolbox API for
//!   in-sandbox operations.
//! - All execution goes through the Session API (`/process/session`) using a
//!   shared session, async execution, and log polling for real-time streaming
//!   output.

pub mod client;
pub mod client_test_override;
pub mod connection;
mod naming;
pub mod openapi_spec;
mod session_sandbox_provider;
pub mod state;
pub mod tools;

use everruns_contracts::connector::ConnectorPlugin;
use everruns_contracts::runtime::LEASED_RESOURCES_FEATURE;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, MountDirectoryBuilder,
    MountPoint, RiskLevel, SystemPromptContext,
};
use everruns_contracts::runtime::tools::Tool;

use connection::DaytonaConnector;
use session_sandbox_provider::DaytonaSessionSandboxProvider;
use std::sync::LazyLock;
use std::time::Duration;

use tools::{
    DAYTONA_OPENAPI_MOUNT_PATH, DaytonaApiCallTool, DaytonaCreateSandboxTool,
    DaytonaDownloadWorkspaceTool, DaytonaExecTool, DaytonaGitCloneTool, DaytonaGitCredentialsTool,
    DaytonaListSandboxesTool, DaytonaListSnapshotsTool, DaytonaManageSandboxTool,
    DaytonaReadFileTool, DaytonaWriteFileTool,
};

// ============================================================================
// Plugin Registration
// ============================================================================

/// Capability plugins this crate contributes to a hosted catalog.
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(DaytonaCapability),
    },
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(SandboxFleetCapability),
    },
];

/// Connector plugins this crate contributes to a hosted catalog.
pub const CONNECTOR_PLUGINS: &[ConnectorPlugin] = &[ConnectorPlugin {
    experimental_only: true,
    factory: || Box::new(DaytonaConnector),
}];
inventory::submit! {
    everruns_contracts::session_sandbox::SessionSandboxProviderPlugin {
        factory: || Box::new(DaytonaSessionSandboxProvider),
    }
}

// ============================================================================
// Constants
// ============================================================================

const DAYTONA_API_BASE: &str = "https://app.daytona.io/api";
const DAYTONA_TOOLBOX_BASE: &str = "https://proxy.app.daytona.io/toolbox";
const DAYTONA_SANDBOX_SECRET_PREFIX: &str = "daytona_sandbox:";
const EXEC_TIMEOUT_MS: u64 = 300_000;
/// Interval for polling streaming exec output from the sandbox.
const EXEC_POLL_INTERVAL: Duration = Duration::from_millis(1_000);
/// Number of consecutive stale polls (no exitCode, no new output) before
/// probing session health with a heartbeat command.
const SESSION_STALE_THRESHOLD: u32 = 30;
/// Timeout (ms) for the heartbeat probe command (`true`).
const SESSION_HEARTBEAT_TIMEOUT_MS: u64 = 15_000;
const SANDBOX_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const SANDBOX_READY_MAX_WAIT: Duration = Duration::from_secs(60);
/// Auto-stop after 5 minutes of inactivity (safety net)
const AUTO_STOP_INTERVAL_MINUTES: u64 = 5;
/// Auto-archive after 30 minutes — archived sandboxes consume fewer Daytona resources
const AUTO_ARCHIVE_INTERVAL_MINUTES: u64 = 30;
/// Auto-delete after 60 minutes — Daytona-native safety net beyond our leased-resource cleanup
const AUTO_DELETE_INTERVAL_MINUTES: u64 = 60;
/// Default workspace path inside Daytona sandboxes
const DAYTONA_WORKSPACE_PATH: &str = "/home/daytona";
/// Heartbeat interval for renewing leases during long-running exec calls.
/// Set well below the auto-stop (5 min) so the sandbox stays alive.
const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3 * 60);

// ============================================================================
// DaytonaCapability
// ============================================================================

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        r#"Daytona sandboxes are isolated networked Linux environments. Create or select a sandbox before sandbox-scoped operations; inspect snapshots before creating custom environments. Sandboxes auto-stop/archive/delete, but delete them when done because stop leaves them visible in Daytona.

Workspace is `/home/daytona`. Clone repos under `/home/daytona/owner/repo`; connected GitHub accounts authenticate private clones. Configure sandbox git credentials before push/pull/fetch and refresh them if they expire."#,
    );
    prompt.push_str(everruns_contracts::runtime::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

/// Check if API calling is enabled in capability config.
///
/// When `enable_api_calling` is `true`, the `daytona_api_call` tool is added
/// and the OpenAPI spec mount path is referenced in the system prompt.
fn is_api_calling_enabled(config: &serde_json::Value) -> bool {
    config
        .get("enable_api_calling")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub struct DaytonaCapability;

/// Provider-neutral resource Sandbox fleet backed by Daytona today. Logical
/// IDs are registered per Session and translated inside the trusted control
/// plane; provider IDs and credentials never become model-facing handles.
pub struct SandboxFleetCapability;

#[derive(Clone, Copy)]
enum FleetOperation {
    Create,
    Exec,
    Read,
    Write,
    List,
    Inspect,
    Checkpoint,
    Manage,
}

struct SandboxFleetTool {
    name: &'static str,
    operation: FleetOperation,
    inner: Option<Box<dyn Tool>>,
}

impl SandboxFleetTool {
    fn delegated(name: &'static str, operation: FleetOperation, inner: Box<dyn Tool>) -> Self {
        Self {
            name,
            operation,
            inner: Some(inner),
        }
    }

    fn list() -> Self {
        Self {
            name: "sandbox_list",
            operation: FleetOperation::List,
            inner: None,
        }
    }

    fn inspect() -> Self {
        Self {
            name: "sandbox_inspect",
            operation: FleetOperation::Inspect,
            inner: None,
        }
    }

    async fn translate_id(
        &self,
        arguments: &mut serde_json::Value,
        context: &everruns_contracts::runtime::tool_context::ToolContext,
    ) -> Result<(), everruns_contracts::runtime::tools::ToolExecutionResult> {
        let Some(logical_id) = arguments.get("sandbox_id").and_then(|value| value.as_str()) else {
            return Err(
                everruns_contracts::runtime::tools::ToolExecutionResult::tool_error(
                    "sandbox_id is required",
                ),
            );
        };
        let Some(registry) = &context.session_resource_registry else {
            return Err(
                everruns_contracts::runtime::tools::ToolExecutionResult::internal_error_msg(
                    "Session resource registry is unavailable",
                ),
            );
        };
        // THREAT[TM-TOOL-054]: Never accept a provider id or a Sandbox owned
        // by another Session from model arguments. Resolve only the current
        // Session's logical resource handle inside the trusted control plane.
        let entry = registry
            .get(context.session_id, logical_id)
            .await
            .map_err(everruns_contracts::runtime::tools::ToolExecutionResult::internal_error)?
            .filter(|entry| {
                entry.kind == "sandbox"
                    && entry.metadata.get("role").and_then(|v| v.as_str()) == Some("resource")
                    && entry.metadata.get("provider").and_then(|v| v.as_str()) == Some("daytona")
            })
            .ok_or_else(|| {
                everruns_contracts::runtime::tools::ToolExecutionResult::tool_error(
                    "Sandbox was not created by this Session",
                )
            })?;
        let external_id = entry
            .metadata
            .get("provider_external_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                everruns_contracts::runtime::tools::ToolExecutionResult::internal_error_msg(
                    "Sandbox provider binding is missing",
                )
            })?;
        arguments["sandbox_id"] = serde_json::json!(external_id);
        Ok(())
    }
}

#[async_trait::async_trait]
impl Tool for SandboxFleetTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        match self.operation {
            FleetOperation::Create => {
                "Create a resource Sandbox. Returns a stable logical sandbox_id scoped to this Session."
            }
            FleetOperation::Exec => "Execute a command in the explicitly named resource Sandbox.",
            FleetOperation::Read => "Read a file from the explicitly named resource Sandbox.",
            FleetOperation::Write => "Write a file in the explicitly named resource Sandbox.",
            FleetOperation::List => "List resource Sandboxes owned by this Session.",
            FleetOperation::Inspect => "Inspect one resource Sandbox owned by this Session.",
            FleetOperation::Checkpoint => {
                "Copy a resource Sandbox workspace into durable Session file storage."
            }
            FleetOperation::Manage => "Stop or delete the explicitly named resource Sandbox.",
        }
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.inner.as_ref().map(|tool| tool.parameters_schema()).unwrap_or_else(|| match self.operation {
            FleetOperation::Inspect => serde_json::json!({
                "type":"object",
                "properties":{"sandbox_id":{"type":"string","description":"Logical resource Sandbox ID"}},
                "required":["sandbox_id"],
                "additionalProperties":false
            }),
            _ => serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
        })
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        if matches!(self.operation, FleetOperation::Checkpoint) {
            return everruns_contracts::tool_types::ToolHints::default()
                .with_open_world(true)
                .with_requires_secrets(true)
                .with_long_running(true);
        }
        self.inner
            .as_ref()
            .map(|tool| tool.hints())
            .unwrap_or_default()
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(
        &self,
    ) -> &'static [everruns_contracts::runtime::tool_context::ToolContextService] {
        &[everruns_contracts::runtime::tool_context::ToolContextService::SessionResourceRegistry]
    }

    async fn execute(
        &self,
        _arguments: serde_json::Value,
    ) -> everruns_contracts::runtime::tools::ToolExecutionResult {
        everruns_contracts::runtime::tools::ToolExecutionResult::tool_error(
            "sandbox_fleet tools require Session context",
        )
    }

    async fn execute_with_context(
        &self,
        mut arguments: serde_json::Value,
        context: &everruns_contracts::runtime::tool_context::ToolContext,
    ) -> everruns_contracts::runtime::tools::ToolExecutionResult {
        if matches!(self.operation, FleetOperation::List) {
            let Some(registry) = &context.session_resource_registry else {
                return everruns_contracts::runtime::tools::ToolExecutionResult::internal_error_msg(
                    "Session resource registry is unavailable",
                );
            };
            return match registry.list(context.session_id, None).await {
                Ok(entries) => everruns_contracts::runtime::tools::ToolExecutionResult::success(
                    serde_json::json!({
                        "sandboxes": entries.into_iter().filter(|entry| entry.kind == "sandbox" && entry.metadata.get("role").and_then(|v| v.as_str()) == Some("resource")).map(|entry| serde_json::json!({
                            "sandbox_id": entry.resource_id,
                            "name": entry.display_name,
                            "status": entry.status.to_string(),
                            "provider": entry.metadata.get("provider").cloned().unwrap_or(serde_json::Value::Null),
                        })).collect::<Vec<_>>()
                    }),
                ),
                Err(error) => {
                    everruns_contracts::runtime::tools::ToolExecutionResult::internal_error(error)
                }
            };
        }

        if matches!(self.operation, FleetOperation::Inspect) {
            let Some(logical_id) = arguments.get("sandbox_id").and_then(|value| value.as_str())
            else {
                return everruns_contracts::runtime::tools::ToolExecutionResult::tool_error(
                    "sandbox_id is required",
                );
            };
            let Some(registry) = &context.session_resource_registry else {
                return everruns_contracts::runtime::tools::ToolExecutionResult::internal_error_msg(
                    "Session resource registry is unavailable",
                );
            };
            return match registry.get(context.session_id, logical_id).await {
                Ok(Some(entry))
                    if entry.kind == "sandbox"
                        && entry.metadata.get("role").and_then(|value| value.as_str())
                            == Some("resource") =>
                {
                    everruns_contracts::runtime::tools::ToolExecutionResult::success(
                        serde_json::json!({
                            "sandbox_id": entry.resource_id,
                            "name": entry.display_name,
                            "status": entry.status.to_string(),
                            "provider": entry.metadata.get("provider").cloned().unwrap_or(serde_json::Value::Null),
                        }),
                    )
                }
                Ok(_) => everruns_contracts::runtime::tools::ToolExecutionResult::tool_error(
                    "Sandbox was not created by this Session",
                ),
                Err(error) => {
                    everruns_contracts::runtime::tools::ToolExecutionResult::internal_error(error)
                }
            };
        }

        let logical_id = arguments
            .get("sandbox_id")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        if !matches!(self.operation, FleetOperation::Create)
            && let Err(error) = self.translate_id(&mut arguments, context).await
        {
            return error;
        }
        let Some(inner) = &self.inner else {
            unreachable!()
        };
        let result = inner.execute_with_context(arguments, context).await;
        if !matches!(self.operation, FleetOperation::Create) {
            if let everruns_contracts::runtime::tools::ToolExecutionResult::Success(mut value) =
                result
            {
                if let Some(logical_id) = logical_id {
                    if value.get("sandbox_id").is_some() {
                        value["sandbox_id"] = serde_json::json!(logical_id.clone());
                    }
                    if matches!(self.operation, FleetOperation::Manage)
                        && value.get("action").and_then(|action| action.as_str()) == Some("delete")
                        && let Some(registry) = &context.session_resource_registry
                    {
                        let _ = registry
                            .update_status(
                                context.session_id,
                                &logical_id,
                                everruns_contracts::runtime::session_resource::SessionResourceStatus::Released,
                            )
                            .await;
                    }
                }
                return everruns_contracts::runtime::tools::ToolExecutionResult::Success(value);
            }
            return result;
        }
        let everruns_contracts::runtime::tools::ToolExecutionResult::Success(mut value) = result
        else {
            return result;
        };
        let Some(external_id) = value
            .get("sandbox_id")
            .and_then(|id| id.as_str())
            .map(str::to_string)
        else {
            return everruns_contracts::runtime::tools::ToolExecutionResult::internal_error_msg(
                "Sandbox provider returned no id",
            );
        };
        let logical_id = everruns_contracts::typed_id::SandboxId::new().to_string();
        let display_name = value
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or("Resource Sandbox")
            .to_string();
        let Some(registry) = &context.session_resource_registry else {
            unreachable!()
        };
        if let Err(error) = registry.register(everruns_contracts::runtime::session_resource::RegisterSessionResource {
            session_id: context.session_id,
            resource_id: logical_id.clone(),
            kind: "sandbox".into(),
            display_name,
            status: everruns_contracts::runtime::session_resource::SessionResourceStatus::Active,
            metadata: serde_json::json!({"role":"resource","provider":"daytona","provider_external_id":external_id}),
        }).await {
            return everruns_contracts::runtime::tools::ToolExecutionResult::internal_error(error);
        }
        value["sandbox_id"] = serde_json::json!(logical_id);
        if let Some(object) = value.as_object_mut() {
            object.remove("requested_name");
        }
        everruns_contracts::runtime::tools::ToolExecutionResult::Success(value)
    }
}

#[async_trait::async_trait]
impl Capability for SandboxFleetCapability {
    fn id(&self) -> &str {
        "sandbox_fleet"
    }
    fn name(&self) -> &str {
        "Sandbox Fleet"
    }
    fn description(&self) -> &str {
        "Create and operate multiple explicitly addressed resource Sandboxes without changing the Session primary Sandbox."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }
    fn icon(&self) -> Option<&str> {
        Some("boxes")
    }
    fn category(&self) -> Option<&str> {
        Some("Sandboxes")
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Resource Sandboxes are explicitly addressed by sandbox_id. They never change where ordinary bash or file tools run. Delete them when finished.",
        )
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(SandboxFleetTool::delegated(
                "sandbox_create",
                FleetOperation::Create,
                Box::new(DaytonaCreateSandboxTool),
            )),
            Box::new(SandboxFleetTool::delegated(
                "sandbox_exec",
                FleetOperation::Exec,
                Box::new(DaytonaExecTool),
            )),
            Box::new(SandboxFleetTool::delegated(
                "sandbox_read_file",
                FleetOperation::Read,
                Box::new(DaytonaReadFileTool),
            )),
            Box::new(SandboxFleetTool::delegated(
                "sandbox_write_file",
                FleetOperation::Write,
                Box::new(DaytonaWriteFileTool),
            )),
            Box::new(SandboxFleetTool::list()),
            Box::new(SandboxFleetTool::inspect()),
            Box::new(SandboxFleetTool::delegated(
                "sandbox_checkpoint",
                FleetOperation::Checkpoint,
                Box::new(DaytonaDownloadWorkspaceTool),
            )),
            Box::new(SandboxFleetTool::delegated(
                "sandbox_manage",
                FleetOperation::Manage,
                Box::new(DaytonaManageSandboxTool),
            )),
        ]
    }
    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }
    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }
}

#[async_trait::async_trait]
impl Capability for DaytonaCapability {
    fn id(&self) -> &str {
        "daytona"
    }

    fn name(&self) -> &str {
        "Daytona"
    }

    fn description(&self) -> &str {
        "Run code in cloud-based sandboxes powered by Daytona. \
         Create multiple isolated Linux environments per session, execute commands, \
         manage files, and download results. EXPERIMENTAL: This capability may change."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("daytona")
    }

    fn category(&self) -> Option<&str> {
        Some("Sandboxes")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        // Tool names and descriptions are NOT listed here — they are already
        // provided via the tools parameter in the API request.  Duplicating
        // them in the system prompt wastes tokens and can confuse models
        // (especially GPT-5.4+ with tool_search).
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(DaytonaCreateSandboxTool),
            Box::new(DaytonaExecTool),
            Box::new(DaytonaReadFileTool),
            Box::new(DaytonaWriteFileTool),
            Box::new(DaytonaDownloadWorkspaceTool),
            Box::new(DaytonaListSnapshotsTool),
            Box::new(DaytonaListSandboxesTool),
            Box::new(DaytonaManageSandboxTool),
            Box::new(DaytonaGitCloneTool),
            Box::new(DaytonaGitCredentialsTool),
        ]
    }

    fn tools_with_config(&self, config: &serde_json::Value) -> Vec<Box<dyn Tool>> {
        let mut tools = self.tools();
        if is_api_calling_enabled(config) {
            tools.push(Box::new(DaytonaApiCallTool));
        }
        tools
    }

    fn mounts(&self) -> Vec<MountPoint> {
        // OpenAPI spec is always mounted so it's available if API calling is enabled.
        // The tool itself is only added when config has enable_api_calling: true.
        let daytona_dir = MountDirectoryBuilder::new()
            .file("openapi.yaml", openapi_spec::DAYTONA_OPENAPI_SPEC)
            .build();
        vec![MountPoint::readonly("/daytona", daytona_dir, self.id())]
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &SystemPromptContext,
        config: &serde_json::Value,
    ) -> Option<String> {
        let base = self.system_prompt_addition()?;
        if is_api_calling_enabled(config) {
            let api_calling_addition = format!(
                "\n\nDirect Daytona API access is enabled. Read `{DAYTONA_OPENAPI_MOUNT_PATH}` before calling endpoints not covered by dedicated tools; resources created this way are tracked for cleanup."
            );
            Some(format!(
                "<capability id=\"{}\">\n{}{}\n</capability>",
                self.id(),
                base,
                api_calling_addition
            ))
        } else {
            Some(format!(
                "<capability id=\"{}\">\n{}\n</capability>",
                self.id(),
                base
            ))
        }
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }

    /// `enable_api_calling` is the only config flag this capability reads
    /// (see `is_api_calling_enabled`), so it is the only exposed field.
    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "enable_api_calling": {
                    "type": "boolean",
                    "title": "Allow direct API calls",
                    "description": format!(
                        "Adds the daytona_api_call tool for Daytona REST endpoints not \
                         covered by dedicated tools; the OpenAPI spec is mounted at \
                         {DAYTONA_OPENAPI_MOUNT_PATH} for reference."
                    ),
                    "default": false
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if !config.is_object() {
            return Err("daytona config must be an object".to_string());
        }
        match config.get("enable_api_calling") {
            None | Some(serde_json::Value::Bool(_)) => Ok(()),
            Some(other) => Err(format!("enable_api_calling must be a boolean, got {other}")),
        }
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Controls whether the agent may call the Daytona REST API directly \
                     via the daytona_api_call tool.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Daytona"),
                description: Some(
                    "Виконуйте код у хмарних пісочницях на основі Daytona. Створюйте кілька \
                     ізольованих Linux-середовищ у межах сесії, виконуйте команди, керуйте \
                     файлами та завантажуйте результати. ЕКСПЕРИМЕНТАЛЬНО: ця можливість \
                     може змінюватися.",
                ),
                config_description: Some(
                    "Визначає, чи може агент напряму викликати REST API Daytona через \
                     інструмент daytona_api_call.",
                ),
                config_overlay: Some(serde_json::json!({
                    "properties": {
                        "enable_api_calling": {
                            "title": "Дозволити прямі виклики API",
                            "description": "Додає інструмент daytona_api_call для викликів REST API Daytona, які не покриваються спеціалізованими інструментами; специфікація OpenAPI змонтована за шляхом /daytona/openapi.yaml."
                        }
                    }
                })),
            },
        ]
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use everruns_contracts::error::Result as AgentResult;
    use everruns_contracts::runtime::capabilities::CapabilityStatus;
    use everruns_contracts::runtime::session_resource::{
        RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
    };
    use everruns_contracts::runtime::session_services::SessionResourceRegistry;
    use everruns_contracts::typed_id::SessionId;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct TestResourceRegistry(Mutex<Vec<SessionResourceEntry>>);

    #[async_trait]
    impl SessionResourceRegistry for TestResourceRegistry {
        async fn register(
            &self,
            entry: RegisterSessionResource,
        ) -> AgentResult<SessionResourceEntry> {
            let now = Utc::now();
            let entry = SessionResourceEntry {
                resource_id: entry.resource_id,
                session_id: entry.session_id,
                kind: entry.kind,
                display_name: entry.display_name,
                status: entry.status,
                metadata: entry.metadata,
                created_at: now,
                updated_at: now,
            };
            self.0.lock().unwrap().push(entry.clone());
            Ok(entry)
        }

        async fn update_status(
            &self,
            _session_id: SessionId,
            _resource_id: &str,
            _status: SessionResourceStatus,
        ) -> AgentResult<Option<SessionResourceEntry>> {
            Ok(None)
        }

        async fn get(
            &self,
            session_id: SessionId,
            resource_id: &str,
        ) -> AgentResult<Option<SessionResourceEntry>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|entry| entry.session_id == session_id && entry.resource_id == resource_id)
                .cloned())
        }

        async fn list(
            &self,
            session_id: SessionId,
            _filter: Option<&SessionResourceFilter>,
        ) -> AgentResult<Vec<SessionResourceEntry>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.session_id == session_id)
                .cloned()
                .collect())
        }

        async fn deregister(
            &self,
            _session_id: SessionId,
            _resource_id: &str,
        ) -> AgentResult<bool> {
            Ok(false)
        }
    }

    // Host reserves this prefix from secret_store but cannot import the
    // constant (crate layering). Pin them so a rename cannot reopen forgery.
    #[test]
    fn the_daytona_sandbox_secret_prefix_is_reserved_from_session_storage() {
        assert!(everruns_core::host::is_internal_session_secret_name(
            &format!("{DAYTONA_SANDBOX_SECRET_PREFIX}sbx-example")
        ));
    }

    #[test]
    fn test_capability_metadata() {
        let cap = DaytonaCapability;
        assert_eq!(cap.id(), "daytona");
        assert_eq!(cap.name(), "Daytona");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("daytona"));
        assert_eq!(cap.category(), Some("Sandboxes"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = DaytonaCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 10);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"daytona_create_sandbox"));
        assert!(names.contains(&"daytona_exec"));
        assert!(names.contains(&"daytona_read_file"));
        assert!(names.contains(&"daytona_write_file"));
        assert!(names.contains(&"daytona_download_workspace"));
        assert!(names.contains(&"daytona_list_snapshots"));
        assert!(names.contains(&"daytona_list_sandboxes"));
        assert!(names.contains(&"daytona_manage_sandbox"));
        assert!(names.contains(&"daytona_git_clone"));
        assert!(names.contains(&"daytona_git_credentials"));
    }

    #[test]
    fn sandbox_fleet_exposes_provider_neutral_tools() {
        let names = SandboxFleetCapability
            .tools()
            .into_iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "sandbox_create",
                "sandbox_exec",
                "sandbox_read_file",
                "sandbox_write_file",
                "sandbox_list",
                "sandbox_inspect",
                "sandbox_checkpoint",
                "sandbox_manage",
            ]
        );
    }

    #[tokio::test]
    async fn sandbox_fleet_rejects_a_logical_id_owned_by_another_session() {
        let owner = SessionId::new();
        let caller = SessionId::new();
        let registry = Arc::new(TestResourceRegistry::default());
        registry
            .register(RegisterSessionResource {
                session_id: owner,
                resource_id: "sandbox_owned".into(),
                kind: "sandbox".into(),
                display_name: "Owned".into(),
                status: SessionResourceStatus::Active,
                metadata: json!({
                    "role": "resource",
                    "provider": "daytona",
                    "provider_external_id": "provider-secret-id"
                }),
            })
            .await
            .unwrap();
        let context = everruns_contracts::runtime::tool_context::ToolContext::new(caller)
            .with_session_resource_registry(registry);

        let result = SandboxFleetTool::inspect()
            .execute_with_context(json!({"sandbox_id": "sandbox_owned"}), &context)
            .await;

        assert!(
            matches!(result, everruns_contracts::runtime::tools::ToolExecutionResult::ToolError(message) if message.contains("not created by this Session"))
        );
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = DaytonaCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("Daytona"));
        assert!(prompt.contains("/home/daytona"));
        assert!(prompt.contains("delete them when done"));
        assert!(prompt.contains("sandbox git credentials"));
    }

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let cap = DaytonaCapability;
        let ctx =
            SystemPromptContext::without_file_store(everruns_contracts::typed_id::SessionId::new());
        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
        // Bumped 1300 → 1600: EVE-778 grew the shared EXEC_OUTPUT_HINT with the
        // single-read/contextual-search policy (+438 bytes), taking this
        // contribution to 1567 bytes.
        assert!(prompt.len() <= 1600, "prompt is {} bytes", prompt.len());
    }

    #[test]
    fn test_all_tools_require_context() {
        let cap = DaytonaCapability;
        for tool in cap.tools() {
            assert!(
                tool.requires_context(),
                "Tool {} should require context",
                tool.name()
            );
        }
    }

    #[test]
    fn test_capability_dependencies() {
        let cap = DaytonaCapability;
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }

    // --- API calling opt-in tests ---

    #[test]
    fn test_is_api_calling_enabled_default_false() {
        assert!(!is_api_calling_enabled(&json!({})));
        assert!(!is_api_calling_enabled(&json!(null)));
        assert!(!is_api_calling_enabled(
            &json!({"enable_api_calling": false})
        ));
    }

    #[test]
    fn test_is_api_calling_enabled_true() {
        assert!(is_api_calling_enabled(&json!({"enable_api_calling": true})));
    }

    #[test]
    fn test_tools_with_config_default_no_api_call() {
        let cap = DaytonaCapability;
        let tools = cap.tools_with_config(&json!({}));
        assert_eq!(tools.len(), 10);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(!names.contains(&"daytona_api_call"));
    }

    #[test]
    fn test_tools_with_config_api_calling_enabled() {
        let cap = DaytonaCapability;
        let tools = cap.tools_with_config(&json!({"enable_api_calling": true}));
        assert_eq!(tools.len(), 11);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"daytona_api_call"));
    }

    #[test]
    fn test_api_call_tool_requires_context() {
        let cap = DaytonaCapability;
        let tools = cap.tools_with_config(&json!({"enable_api_calling": true}));
        let api_tool = tools
            .iter()
            .find(|t| t.name() == "daytona_api_call")
            .unwrap();
        assert!(api_tool.requires_context());
    }

    #[test]
    fn test_config_schema_and_validate_config() {
        let cap = DaytonaCapability;

        let schema = cap.config_schema().expect("config schema");
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["enable_api_calling"].is_object());
        assert_eq!(
            schema["properties"]["enable_api_calling"]["default"],
            json!(false)
        );

        // Null, empty, and valid configs are accepted.
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
        assert!(cap.validate_config(&json!({})).is_ok());
        assert!(
            cap.validate_config(&json!({"enable_api_calling": true}))
                .is_ok()
        );

        // Non-boolean values are rejected.
        let err = cap
            .validate_config(&json!({"enable_api_calling": "yes"}))
            .unwrap_err();
        assert!(err.contains("enable_api_calling"));
    }

    #[test]
    fn test_localizations_resolve_uk() {
        let cap = DaytonaCapability;
        // The capability name is a brand name and stays Latin in Ukrainian.
        assert_eq!(cap.localized_name(Some("uk-UA")), "Daytona");
        assert!(
            cap.localized_description(Some("uk-UA"))
                .contains("пісочницях")
        );
        assert!(cap.describe_schema(None).is_some());
    }

    #[test]
    fn test_mounts_include_openapi_spec() {
        let cap = DaytonaCapability;
        let mounts = cap.mounts();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].path, "/daytona");
        assert!(mounts[0].is_readonly());
    }

    #[test]
    fn embedded_openapi_contains_sandbox_and_toolbox_documentation() {
        let spec = openapi_spec::DAYTONA_OPENAPI_SPEC;
        assert!(spec.contains("openapi: 3.0.3"));
        assert!(spec.contains("/sandbox"));
        assert!(spec.contains("/toolbox/"));
    }

    #[tokio::test]
    async fn test_system_prompt_with_api_calling_enabled() {
        let cap = DaytonaCapability;
        let ctx =
            SystemPromptContext::without_file_store(everruns_contracts::typed_id::SessionId::new());
        let prompt = cap
            .system_prompt_contribution_with_config(&ctx, &json!({"enable_api_calling": true}))
            .await
            .unwrap();
        assert!(prompt.contains(DAYTONA_OPENAPI_MOUNT_PATH));
        assert!(prompt.contains("Direct Daytona API access"));
    }

    #[tokio::test]
    async fn test_system_prompt_without_api_calling() {
        let cap = DaytonaCapability;
        let ctx =
            SystemPromptContext::without_file_store(everruns_contracts::typed_id::SessionId::new());
        let prompt = cap
            .system_prompt_contribution_with_config(&ctx, &json!({}))
            .await
            .unwrap();
        assert!(!prompt.contains("Direct Daytona API access"));
        // Still has the base prompt
        assert!(prompt.contains("Daytona"));
    }
}
