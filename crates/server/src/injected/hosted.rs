// A hosted capability served as an injected one: the capability's own tools,
// run here with the same answers a data plane's host calls get.
//
// Design: knowledge/execution/injected-capabilities.md
//
// Decisions:
// - The tools are the capability's own, unchanged; only their context is
//   this server's. A tool that asked the control plane through a host call
//   on a cell gets the same handler here, in process
//   (`data_plane::host_calls::answer`), as the org's internal caller with
//   the org's feature flags: the same checks and failures, and nothing
//   crosses the link.
// - A session whose agent lists neither `injected:{name}` nor `{name}`
//   gets no tools, as for Knowledge Base.
// - The handler is bound after the API state exists ([`HostCalls::bind`]);
//   until then every call fails, which a data plane only sees if it calls
//   during startup.
// - Native tool names: the model keeps calling the tools by their names.

use super::InjectedCapabilities;
use super::knowledge_base::SessionCapabilityConfig;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_contracts::host_calls::{HostCall, HostCallAnswer};
use everruns_contracts::runtime::capabilities::DeclarativeCapabilityDefinition;
use everruns_contracts::runtime::capabilities::injected::{
    InjectedAttach, InjectedCapabilityDefinition, InjectedServer, InjectedToolNames,
    injected_capability_id,
};
use everruns_contracts::typed_id::{OrgId, SessionId};
use everruns_core::capabilities::Capability;
use everruns_core::mcp::injected_server::{InjectedCapabilityService, VerifiedSession};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::Tool;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// Runs a host call for an org, as the data plane route does.
#[async_trait]
pub trait HostCallRunner: Send + Sync {
    async fn answer(
        &self,
        org: OrgId,
        call: &'static HostCall,
        params: Value,
    ) -> Result<HostCallAnswer>;
}

/// The host call handler, bound once the API state exists.
#[derive(Clone, Default)]
pub struct HostCalls(Arc<OnceLock<Arc<dyn HostCallRunner>>>);

impl HostCalls {
    pub fn bind(&self, runner: Arc<dyn HostCallRunner>) {
        let _ = self.0.set(runner);
    }

    /// Run host call `name` for `org`.
    pub async fn answer(&self, org: OrgId, name: &str, params: Value) -> Result<HostCallAnswer> {
        let call = everruns_contracts::host_calls::by_name(name)
            .ok_or_else(|| anyhow!("no host call {name}"))?;
        let runner = self
            .0
            .get()
            .ok_or_else(|| anyhow!("the control plane is starting"))?;
        runner.answer(org, call, params).await
    }
}

/// What a session's tool context gets on top of its org and session; the
/// verified token names the person who opened the turn (`act`), if any.
pub type ContextFor =
    Arc<dyn Fn(ToolContext, &VerifiedSession, OrgId, SessionId) -> ToolContext + Send + Sync>;

pub struct HostedService {
    capability: Arc<dyn Capability>,
    definition: InjectedCapabilityDefinition,
    configs: Arc<dyn SessionCapabilityConfig>,
    context: ContextFor,
}

/// The injected definition of `capability`, served at `base_url`.
pub fn definition_of(capability: &dyn Capability, base_url: &str) -> InjectedCapabilityDefinition {
    InjectedCapabilityDefinition {
        capability: DeclarativeCapabilityDefinition {
            name: capability.id().to_string(),
            display_name: Some(capability.name().to_string()),
            description: capability.description().to_string(),
            icon: capability.icon().map(str::to_string),
            category: capability.category().map(str::to_string),
            system_prompt: capability.system_prompt_addition().map(str::to_string),
            features: capability
                .features()
                .into_iter()
                .map(str::to_string)
                .collect(),
            risk_level: capability.risk_level(),
            ..DeclarativeCapabilityDefinition::default()
        },
        server: InjectedServer {
            url: InjectedCapabilities::url_for(base_url, capability.id()),
            tool_timeout_ms: None,
        },
        attach: InjectedAttach::OnRequest,
        match_labels: BTreeMap::new(),
        tools: InjectedToolNames::Native,
    }
}

impl HostedService {
    pub fn new(
        capability: Arc<dyn Capability>,
        base_url: &str,
        configs: Arc<dyn SessionCapabilityConfig>,
        context: ContextFor,
    ) -> Self {
        Self {
            definition: definition_of(capability.as_ref(), base_url),
            capability,
            configs,
            context,
        }
    }

    fn org(session: &VerifiedSession) -> Result<OrgId> {
        session
            .claims
            .grp
            .as_deref()
            .ok_or_else(|| anyhow!("session token has no organization"))?
            .parse()
            .map_err(|_| anyhow!("session token group is not an organization"))
    }
}

#[async_trait]
impl InjectedCapabilityService for HostedService {
    fn definition(&self) -> &InjectedCapabilityDefinition {
        &self.definition
    }

    async fn tools(&self, session: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>> {
        let id = self.capability.id();
        let injected = injected_capability_id(id);
        let config = self
            .configs
            .config(
                Self::org(session)?,
                SessionId::from_uuid(session.session_id),
                &[injected.as_str(), id],
            )
            .await?;
        Ok(match config {
            Some(config) => self.capability.tools_with_config(&config),
            None => Vec::new(),
        })
    }

    async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext> {
        let org = Self::org(session)?;
        let id = SessionId::from_uuid(session.session_id);
        let context = ToolContext::new(id).with_org_id(org);
        Ok((self.context)(context, session, org, id))
    }
}

/// Poppy calls made here, with the session owner's keys and Session Tokens
/// that never leave the control plane. Same answers as a cell's.
pub mod poppy {
    use super::*;
    use everruns_capabilities::capabilities::{
        PoppyCall, PoppyClientCapability, PoppyInvoker, PoppyInvokerExt,
    };
    use everruns_contracts::host_calls::POPPY_CALL;

    struct Invoker {
        calls: HostCalls,
        org: OrgId,
        session: SessionId,
    }

    #[async_trait]
    impl PoppyInvoker for Invoker {
        async fn invoke(&self, call: PoppyCall) -> std::result::Result<Value, String> {
            let params =
                serde_json::json!({ "session_id": self.session.to_string(), "call": call });
            match self.calls.answer(self.org, POPPY_CALL, params).await {
                Ok(HostCallAnswer::Value(value)) => Ok(value),
                // A company's refusal or an unreachable company: for the model.
                Ok(HostCallAnswer::Error { kind, message })
                    if everruns_internal_protocol::commands::is_bad_request(
                        &everruns_internal_protocol::proto::CommandError {
                            kind,
                            message: message.clone(),
                        },
                    ) =>
                {
                    Err(message)
                }
                Ok(HostCallAnswer::Error { message, .. }) => {
                    tracing::warn!(%message, "Poppy call failed");
                    Err("Poppy is unavailable right now.".to_string())
                }
                Err(error) => {
                    tracing::warn!(%error, "Poppy call failed");
                    Err("Poppy is unavailable right now.".to_string())
                }
            }
        }
    }

    pub fn service(
        base_url: &str,
        configs: Arc<dyn SessionCapabilityConfig>,
        calls: HostCalls,
    ) -> HostedService {
        HostedService::new(
            Arc::new(PoppyClientCapability),
            base_url,
            configs,
            Arc::new(move |context, _, org, session| {
                context.with_extension(Arc::new(PoppyInvokerExt(Arc::new(Invoker {
                    calls: calls.clone(),
                    org,
                    session,
                }))))
            }),
        )
    }
}

/// Slack's tools, acting as the bot of the endpoint the session came
/// through (its token stays here). Slack's model view stays on the data
/// plane (`SlackModelViewCapability`), added beside it by [`super::attach`].
///
/// Decision: `slack_upload_file` reads the session's file from its cell
/// through the `fs` session command, until the `io.everruns/files`
/// extension lets the server ask the data plane for it. The view is
/// read-only: a Slack tool never writes the session's files.
pub mod slack {
    use super::*;
    use crate::domains::data_plane::session_plane::SessionPlane;
    use everruns_capabilities::capabilities::SlackCapability;
    use everruns_contracts::error::{AgentLoopError, Result as RuntimeResult};
    use everruns_contracts::host_calls::SLACK_ACTION;
    use everruns_contracts::runtime::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use everruns_contracts::session_commands;
    use everruns_contracts::slack_action::{
        SlackAction, SlackActionError, SlackActionInvoker, SlackActionInvokerExt,
        SlackActionOutcome,
    };
    use everruns_core::SessionFileSystem;

    struct Invoker {
        calls: HostCalls,
        org: OrgId,
        session: SessionId,
    }

    #[async_trait]
    impl SlackActionInvoker for Invoker {
        async fn invoke(
            &self,
            action: SlackAction,
        ) -> std::result::Result<SlackActionOutcome, SlackActionError> {
            let params =
                serde_json::json!({ "session_id": self.session.to_string(), "action": action });
            let answer = match self.calls.answer(self.org, SLACK_ACTION, params).await {
                Ok(HostCallAnswer::Value(answer)) => answer,
                Ok(HostCallAnswer::Error { message, .. }) => {
                    return Err(SlackActionError::Transient(message));
                }
                Err(error) => return Err(SlackActionError::Transient(error.to_string())),
            };
            let transient =
                |error: serde_json::Error| SlackActionError::Transient(error.to_string());
            if let Some(error) = answer.get("error") {
                return Err(serde_json::from_value(error.clone()).map_err(transient)?);
            }
            serde_json::from_value(answer["outcome"].clone()).map_err(transient)
        }
    }

    /// The session's files on its cell, read-only.
    struct CellFiles {
        db: Arc<crate::storage::StorageBackend>,
        org: OrgId,
    }

    fn read_only<T>() -> RuntimeResult<T> {
        Err(AgentLoopError::tool(
            "the session's files are read-only here",
        ))
    }

    #[async_trait]
    impl SessionFileSystem for CellFiles {
        fn is_mount_resolver(&self) -> bool {
            false
        }

        async fn read_file(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> RuntimeResult<Option<SessionFile>> {
            let failed = |error: anyhow::Error| AgentLoopError::store(error.to_string());
            let org_id = everruns_core::organization::org_internal_id_from_public(self.org);
            let plane = SessionPlane::of(&self.db, org_id, session_id)
                .await
                .map_err(failed)?
                .ok_or_else(|| AgentLoopError::store("the session is not on a data plane"))?;
            let path = self.resolve_path(path);
            let reply = plane
                .command(
                    &session_commands::FS,
                    &serde_json::json!({ "op": "raw", "path": path }),
                )
                .await
                .map_err(failed)?;
            if reply["result"] != "done" {
                return Ok(None);
            }
            let value = &reply["value"];
            let content = value["content"].as_str().unwrap_or_default().to_string();
            let now = chrono::Utc::now();
            Ok(Some(SessionFile {
                id: uuid::Uuid::nil(),
                session_id: session_id.uuid(),
                name: path.rsplit('/').next().unwrap_or_default().to_string(),
                size_bytes: content.len() as i64,
                path,
                content: Some(content),
                encoding: value["encoding"].as_str().unwrap_or("text").to_string(),
                is_directory: false,
                is_readonly: true,
                created_at: now,
                updated_at: now,
            }))
        }

        async fn write_file(
            &self,
            _: SessionId,
            _: &str,
            _: &str,
            _: &str,
        ) -> RuntimeResult<SessionFile> {
            read_only()
        }

        async fn delete_file(&self, _: SessionId, _: &str, _: bool) -> RuntimeResult<bool> {
            read_only()
        }

        async fn list_directory(&self, _: SessionId, _: &str) -> RuntimeResult<Vec<FileInfo>> {
            read_only()
        }

        async fn stat_file(&self, _: SessionId, _: &str) -> RuntimeResult<Option<FileStat>> {
            read_only()
        }

        async fn grep_files(
            &self,
            _: SessionId,
            _: &str,
            _: Option<&str>,
        ) -> RuntimeResult<Vec<GrepMatch>> {
            read_only()
        }

        async fn create_directory(&self, _: SessionId, _: &str) -> RuntimeResult<FileInfo> {
            read_only()
        }
    }

    pub fn service(
        base_url: &str,
        configs: Arc<dyn SessionCapabilityConfig>,
        calls: HostCalls,
        db: Arc<crate::storage::StorageBackend>,
    ) -> HostedService {
        HostedService::new(
            Arc::new(SlackCapability),
            base_url,
            configs,
            Arc::new(move |mut context: ToolContext, _, org, session| {
                context.file_store = Some(Arc::new(CellFiles {
                    db: db.clone(),
                    org,
                }));
                context.with_extension(Arc::new(SlackActionInvokerExt(Arc::new(Invoker {
                    calls: calls.clone(),
                    org,
                    session,
                }))))
            }),
        )
    }
}

/// The platform commands of a session's native Platform tools and shell
/// `everruns` builtin (`discover`, `query`, `execute`, `run`), run on the
/// command catalog as the person who opened the turn (the token's `act`),
/// with the same checks as the `platform_command` host call: the session
/// must resolve `platform`, and the person must still be a member, with
/// their current role.
///
/// Decisions:
/// - The Platform capability stays native on the data plane: its docs
///   mount, skills and shell builtin need the session's own runtime. Only
///   the command catalog, which is the control plane's, is served here, as
///   one tool (`platform_command`) a data plane calls for those tools, never
///   one an agent lists.
/// - A turn no person opened (a channel message, a trigger) runs no
///   platform command, as before: there is no one to act as.
pub mod platform {
    use super::*;
    use everruns_contracts::host_calls::PLATFORM_COMMAND;
    use everruns_contracts::tools::ToolExecutionResult;

    /// Its one tool.
    pub use everruns_contracts::runtime::capabilities::injected::PLATFORM_COMMAND_TOOL as TOOL;
    /// The server's name (`/injected/platform_commands/mcp`).
    pub use everruns_contracts::runtime::capabilities::injected::PLATFORM_COMMANDS as NAME;
    const OPERATIONS: [&str; 4] = ["discover", "query", "execute", "run"];

    struct Command {
        calls: HostCalls,
        org: OrgId,
        session: SessionId,
        actor: Option<String>,
    }

    #[async_trait]
    impl Tool for Command {
        fn name(&self) -> &str {
            TOOL
        }

        fn description(&self) -> &str {
            "Run one platform command for the session, as the person who opened its turn."
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": { "type": "string", "enum": OPERATIONS },
                    "arguments": { "type": "object" },
                },
                "required": ["operation"],
            })
        }

        async fn execute(&self, arguments: Value) -> ToolExecutionResult {
            let operation = arguments["operation"].as_str().unwrap_or_default();
            if !OPERATIONS.contains(&operation) {
                return ToolExecutionResult::tool_error(format!(
                    "unknown platform operation {operation:?}"
                ));
            }
            let Some(actor) = &self.actor else {
                return ToolExecutionResult::tool_error(
                    "Platform commands run only within a turn a person started",
                );
            };
            let params = serde_json::json!({
                "session_id": self.session.to_string(),
                "operation": operation,
                "arguments": arguments.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({})),
                "acting_user": actor,
            });
            match self.calls.answer(self.org, PLATFORM_COMMAND, params).await {
                Ok(HostCallAnswer::Value(Value::String(output))) => {
                    ToolExecutionResult::success(Value::String(output))
                }
                Ok(HostCallAnswer::Value(other)) => {
                    ToolExecutionResult::success(Value::String(other.to_string()))
                }
                Ok(HostCallAnswer::Error { message, .. }) => {
                    ToolExecutionResult::tool_error(message)
                }
                Err(error) => {
                    tracing::warn!(%error, "platform command failed");
                    ToolExecutionResult::tool_error("Platform commands are unavailable right now.")
                }
            }
        }
    }

    pub struct PlatformCommands {
        definition: InjectedCapabilityDefinition,
        calls: HostCalls,
    }

    pub fn service(base_url: &str, calls: HostCalls) -> PlatformCommands {
        PlatformCommands {
            definition: InjectedCapabilityDefinition {
                capability: DeclarativeCapabilityDefinition {
                    name: NAME.to_string(),
                    display_name: Some("Platform commands".to_string()),
                    description: "The platform commands of a session's Platform tools.".to_string(),
                    ..DeclarativeCapabilityDefinition::default()
                },
                server: InjectedServer {
                    url: InjectedCapabilities::url_for(base_url, NAME),
                    tool_timeout_ms: None,
                },
                attach: InjectedAttach::OnRequest,
                match_labels: BTreeMap::new(),
                tools: InjectedToolNames::Native,
            },
            calls,
        }
    }

    #[async_trait]
    impl InjectedCapabilityService for PlatformCommands {
        fn definition(&self) -> &InjectedCapabilityDefinition {
            &self.definition
        }

        async fn tools(&self, session: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>> {
            Ok(vec![Box::new(Command {
                calls: self.calls.clone(),
                org: HostedService::org(session)?,
                session: SessionId::from_uuid(session.session_id),
                actor: session.claims.act.as_ref().map(|act| act.sub.clone()),
            })])
        }

        async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext> {
            Ok(ToolContext::new(SessionId::from_uuid(session.session_id))
                .with_org_id(HostedService::org(session)?))
        }
    }
}

/// A session's Memory, as the files a data plane mounts
/// (`io.everruns/files`).
///
/// Decisions:
/// - One files server for every session; the token names the session and
///   its org, and the mounts it may reach come from the session's record
///   (`data_plane::memory`), the same check the `memory_file` host call
///   makes. Paths are the control plane's own (`/memory/user/...`).
/// - It is not a capability an agent lists: it has no tools, and its
///   definition's id never matches a hosted capability, so [`super::attach`]
///   leaves packages alone. A data plane gets its URL with each create.
pub mod memory {
    use super::*;
    use everruns_contracts::host_calls::MEMORY_FILE;

    /// The files server's name (`/injected/memory_files/mcp`).
    pub use everruns_contracts::runtime::capabilities::injected::MEMORY_FILES as NAME;

    pub struct MemoryFiles {
        definition: InjectedCapabilityDefinition,
        calls: HostCalls,
    }

    pub fn service(base_url: &str, calls: HostCalls) -> MemoryFiles {
        MemoryFiles {
            definition: InjectedCapabilityDefinition {
                capability: DeclarativeCapabilityDefinition {
                    name: NAME.to_string(),
                    display_name: Some("Memory".to_string()),
                    description: "The session's Memory files.".to_string(),
                    ..DeclarativeCapabilityDefinition::default()
                },
                server: InjectedServer {
                    url: InjectedCapabilities::url_for(base_url, NAME),
                    tool_timeout_ms: None,
                },
                attach: InjectedAttach::OnRequest,
                match_labels: BTreeMap::new(),
                tools: InjectedToolNames::Native,
            },
            calls,
        }
    }

    #[async_trait]
    impl InjectedCapabilityService for MemoryFiles {
        fn definition(&self) -> &InjectedCapabilityDefinition {
            &self.definition
        }

        async fn tools(&self, _session: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>> {
            Ok(Vec::new())
        }

        async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext> {
            Ok(ToolContext::new(SessionId::from_uuid(session.session_id))
                .with_org_id(HostedService::org(session)?))
        }

        fn serves_files(&self) -> bool {
            true
        }

        async fn files(
            &self,
            session: &VerifiedSession,
            op: &'static str,
            mut params: Value,
        ) -> Result<Value> {
            let org = HostedService::org(session)?;
            params["session_id"] = Value::String(session.session_id.to_string());
            params["op"] = Value::String(op.to_string());
            match self.calls.answer(org, MEMORY_FILE, params).await? {
                HostCallAnswer::Value(answer) => {
                    match answer.get("error").and_then(Value::as_str) {
                        Some(error) => Err(anyhow!("{error}")),
                        None => Ok(answer.get("ok").cloned().unwrap_or(Value::Null)),
                    }
                }
                HostCallAnswer::Error { message, .. } => Err(anyhow!("{message}")),
            }
        }
    }
}

/// The URLs of the servers data planes call for sessions' native
/// capabilities (`injected::CONTROLLER_SERVERS`), by name, once this
/// control plane serves them to data planes; empty before.
pub fn controller_servers() -> BTreeMap<String, String> {
    super::served()
        .into_iter()
        .filter(|definition| {
            everruns_contracts::runtime::capabilities::injected::CONTROLLER_SERVERS
                .contains(&definition.capability.name.as_str())
        })
        .map(|definition| (definition.capability.name.clone(), definition.server.url))
        .collect()
}
