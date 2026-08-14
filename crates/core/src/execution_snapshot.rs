// Canonical resolved execution snapshot (EVE-872).
//
// Decision: host turn execution consumes this neutral, value-first projection
// instead of stored Agent/Harness/Session aggregates (the session arrives as the portable `ExecutionSession` projection, EVE-882). Both the Framework
// in-process runtime and the hosted workers build the same value here, so
// harness → agent → session precedence is applied by one code path
// (`AgentConfigOverlay` merge semantics) regardless of which host executes the
// turn. Credentials, database lifecycle fields, organization records,
// timestamps, UI metadata, and CRUD status are excluded by construction, so
// Debug/serialization of this value is secret-free without redaction logic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agent_definition::AgentDefinition;
use crate::capability_types::AgentCapabilityConfig;
use crate::config_layer::AgentConfigOverlay;
use crate::error::{AgentLoopError, Result};
use crate::harness_definition::HarnessDefinition;
use crate::mcp_server::{
    McpProtocolMode, McpServerAuthMode, McpServerTransportType, ScopedMcpServer,
};
use crate::network_access::NetworkAccessList;
use crate::session::ExecutionSession;
use crate::session_file::InitialFile;
use crate::tool_types::ToolDefinition;
use crate::typed_id::{AgentId, HarnessId, ModelId, SessionId, WorkspaceId};
use crate::{
    execution_loading::AgentStore, execution_loading::HarnessStore, execution_loading::SessionStore,
};

/// Non-secret MCP scope entry retained by the execution snapshot.
///
/// Scoped MCP server configs can carry literal credentials in header and env
/// values (e.g. `Authorization: Bearer …`). The snapshot keeps the scope —
/// which servers exist and how they authenticate — but only the *names* of
/// headers and env vars. Execution resolves live connections (including
/// credential material) through the host's MCP seam, never from this value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMcpServer {
    pub transport_type: McpServerTransportType,
    pub url: String,
    /// Header names only; header values never enter the snapshot.
    pub header_names: Vec<String>,
    /// Env var names only; env values never enter the snapshot.
    pub env_names: Vec<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub auth_mode: McpServerAuthMode,
    pub protocol_mode: McpProtocolMode,
    pub oauth_provider_id: Option<String>,
    pub tool_discovery: bool,
}

impl From<&ScopedMcpServer> for SnapshotMcpServer {
    fn from(server: &ScopedMcpServer) -> Self {
        let mut header_names: Vec<String> = server.headers.keys().cloned().collect();
        header_names.sort();
        let mut env_names: Vec<String> = server.env.keys().cloned().collect();
        env_names.sort();
        Self {
            transport_type: server.transport_type.clone(),
            url: server.url.clone(),
            header_names,
            env_names,
            command: server.command.clone(),
            args: server.args.clone(),
            auth_mode: server.auth_mode.clone(),
            protocol_mode: server.protocol_mode,
            oauth_provider_id: server.oauth_provider_id.clone(),
            tool_discovery: server.tool_discovery,
        }
    }
}

/// Canonical resolved execution value for one session's turn execution.
///
/// Contains only turn-relevant configuration plus the typed correlation
/// values execution needs. Produced by [`ResolvedExecutionSnapshot::project`]
/// — the platform projection boundary where missing, mismatched, or inactive
/// stored records fail before host execution begins.
///
/// Field order is fixed and map fields are `BTreeMap`s, so serialization of
/// equal values is byte-identical (deterministic).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedExecutionSnapshot {
    // --- Typed session correlation values required by execution ---
    pub session_id: SessionId,
    pub workspace_id: WorkspaceId,
    pub harness_id: HarnessId,
    pub agent_id: Option<AgentId>,
    /// Public (`org_…`) organization id used for execution scoping. This is a
    /// correlation value, not the organization record.
    pub organization_id: String,

    // --- Effective configuration (harness → agent → session, applied once) ---
    /// Effective instructions (merged system prompt, session goal included).
    pub instructions: Option<String>,
    /// Effective model specification; `None` selects the host default model.
    pub default_model_id: Option<ModelId>,
    /// Configured capability references with per-layer config merged.
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Explicitly configured tools (client-side or capability-provided).
    pub tools: Vec<ToolDefinition>,
    /// Effective initial workspace files.
    pub initial_files: Vec<InitialFile>,
    /// Effective scoped MCP servers, credential values excluded.
    pub mcp_servers: BTreeMap<String, SnapshotMcpServer>,
    /// Effective workspace network policy.
    pub network_access: Option<NetworkAccessList>,
    /// Maximum reason iterations per turn.
    pub max_iterations: Option<usize>,
    /// Request-level parallel tool calling preference.
    pub parallel_tool_calls: Option<bool>,

    // --- Non-secret execution metadata ---
    pub locale: Option<String>,
    pub tags: Vec<String>,
    pub blueprint_id: Option<String>,
    pub blueprint_config: Option<serde_json::Value>,
    /// Embedder metadata folded from the harness chain (root base, leaf wins).
    pub embedder_metadata: BTreeMap<String, String>,
}

impl ResolvedExecutionSnapshot {
    /// Project loaded records into the canonical execution value.
    ///
    /// This is the execution projection boundary: it fails when a referenced
    /// agent is missing or does not match the session's agent id. Harness
    /// lifecycle and inheritance validation happen one step earlier, at the
    /// platform loading seam (EVE-881): parent-chain loading, cycle/error
    /// handling, and archived/deleted validation run behind the
    /// [`HarnessStore`] contract, so only the effective, executable
    /// [`HarnessDefinition`] reaches this projection. Agent lifecycle
    /// validation likewise fails at the [`AgentStore`] seam (EVE-877). Host
    /// execution never sees a snapshot built from broken record wiring.
    ///
    /// Precedence is applied exactly once, through the same
    /// [`AgentConfigOverlay`] merge semantics the runtime capability
    /// resolution uses: harness → agent → session, leaf wins.
    pub fn project(
        harness: &HarnessDefinition,
        agent: Option<&AgentDefinition>,
        session: &ExecutionSession,
    ) -> Result<Self> {
        let agent = match (session.agent_id, agent) {
            (Some(agent_id), Some(agent)) => {
                if agent.id != agent_id {
                    return Err(AgentLoopError::config(format!(
                        "session {} references agent {} but agent {} was provided",
                        session.id, agent_id, agent.id
                    )));
                }
                Some(agent)
            }
            (Some(agent_id), None) => {
                return Err(AgentLoopError::agent_not_found(agent_id));
            }
            (None, _) => None,
        };

        let agent_layers = agent.into_iter().map(AgentConfigOverlay::from);
        let effective = AgentConfigOverlay::fold(
            [AgentConfigOverlay::from(harness)]
                .into_iter()
                .chain(agent_layers)
                .chain([AgentConfigOverlay::from(session)]),
        );

        // Chain folding (root base, leaf wins) already happened at the platform
        // seam; the definition carries the effective metadata.
        let embedder_metadata = harness
            .embedder_metadata
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();

        Ok(Self {
            session_id: session.id,
            workspace_id: session.workspace_id,
            harness_id: session.harness_id,
            agent_id: session.agent_id,
            organization_id: session.organization_id.clone(),
            instructions: effective.system_prompt,
            default_model_id: effective.default_model_id,
            capabilities: effective.capabilities,
            tools: effective.tools,
            initial_files: effective.initial_files,
            mcp_servers: effective
                .mcp_servers
                .iter()
                .map(|(name, server)| (name.clone(), SnapshotMcpServer::from(server)))
                .collect(),
            network_access: effective.network_access,
            max_iterations: effective.max_iterations,
            parallel_tool_calls: effective.parallel_tool_calls,
            locale: session.locale.clone(),
            tags: session.tags.clone(),
            blueprint_id: session.blueprint_id.clone(),
            blueprint_config: session.blueprint_config.clone(),
            embedder_metadata,
        })
    }
}

/// Load a session's records and project them into the canonical snapshot.
///
/// Store lookups are the cross-tenant boundary: org-scoped stores return
/// `None` for records outside the caller's organization, which this loader
/// turns into not-found projection failures before host execution.
pub async fn load_execution_snapshot(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session_store: &dyn SessionStore,
    session_id: SessionId,
) -> Result<ResolvedExecutionSnapshot> {
    let session = session_store
        .get_session(session_id)
        .await?
        .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;
    load_execution_snapshot_for_session(harness_store, agent_store, &session).await
}

/// Project a snapshot for an already-loaded session record.
///
/// Hosts that fold runtime attachments or other session-scoped overlays into
/// the record before resolution use this variant.
pub async fn load_execution_snapshot_for_session(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session: &ExecutionSession,
) -> Result<ResolvedExecutionSnapshot> {
    let harness = harness_store
        .get_harness(session.harness_id)
        .await?
        .ok_or_else(|| AgentLoopError::harness_not_found(session.harness_id))?;
    let agent = match session.agent_id {
        Some(agent_id) => agent_store.get_agent(agent_id).await?,
        None => None,
    };
    ResolvedExecutionSnapshot::project(&harness, agent.as_ref(), session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_access::NetworkAccessList;
    use crate::test_fixtures::{TestAgentStore, TestHarnessStore};
    use std::collections::HashMap;

    fn harness() -> HarnessDefinition {
        HarnessDefinition {
            capabilities: vec![AgentCapabilityConfig::new("session_file_system")],
            ..HarnessDefinition::new("base", "Harness prompt.")
        }
    }

    fn agent(agent_id: AgentId, _harness_id: HarnessId) -> AgentDefinition {
        AgentDefinition {
            max_iterations: Some(8),
            display_name: Some("Agent".into()),
            ..AgentDefinition::new(agent_id, "agent", "Agent prompt.")
        }
    }

    fn session(
        session_id: SessionId,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
    ) -> ExecutionSession {
        ExecutionSession {
            agent_id,
            title: Some("UI-TITLE-MARKER".into()),
            locale: Some("en-US".into()),
            tags: vec!["tag-a".into()],
            ..ExecutionSession::with_own_workspace(session_id, harness_id)
        }
    }

    fn ids() -> (HarnessId, AgentId, SessionId) {
        (
            HarnessId::from_seed(11),
            AgentId::from_seed(11),
            SessionId::from_seed(11),
        )
    }

    fn file(path: &str, content: &str) -> InitialFile {
        InitialFile {
            path: path.into(),
            content: content.into(),
            encoding: "text".into(),
            is_readonly: false,
        }
    }

    // --- Precedence matrix: harness → agent → session, leaf wins ---

    #[test]
    fn precedence_instructions_concatenate_root_to_leaf() {
        let (harness_id, agent_id, session_id) = ids();
        let harness = harness();
        let agent = agent(agent_id, harness_id);
        let mut session = session(session_id, harness_id, Some(agent_id));
        session.system_prompt = Some("Session prompt.".into());

        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        assert_eq!(
            snapshot.instructions.as_deref(),
            Some("Harness prompt.\n\nAgent prompt.\n\nSession prompt.")
        );
    }

    #[test]
    fn precedence_model_leaf_layer_wins() {
        let (harness_id, agent_id, session_id) = ids();
        let mut harness = harness();
        harness.default_model_id = Some(ModelId::from_seed(1));
        let mut agent = agent(agent_id, harness_id);
        agent.default_model_id = Some(ModelId::from_seed(2));
        let mut session = session(session_id, harness_id, Some(agent_id));

        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        assert_eq!(snapshot.default_model_id, Some(ModelId::from_seed(2)));

        session.model_id = Some(ModelId::from_seed(3));
        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        assert_eq!(snapshot.default_model_id, Some(ModelId::from_seed(3)));
    }

    #[test]
    fn precedence_capabilities_override_by_id() {
        let (harness_id, agent_id, session_id) = ids();
        let mut harness = harness();
        harness.capabilities = vec![AgentCapabilityConfig::with_config(
            "web_fetch",
            serde_json::json!({"enable_file_download": true}),
        )];
        let mut agent = agent(agent_id, harness_id);
        agent.capabilities = vec![AgentCapabilityConfig::new("current_time")];
        let mut session = session(session_id, harness_id, Some(agent_id));
        session.capabilities = vec![AgentCapabilityConfig::with_config(
            "web_fetch",
            serde_json::json!({"enable_file_download": false}),
        )];

        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        assert_eq!(snapshot.capabilities.len(), 2);
        assert_eq!(
            snapshot.capabilities[0],
            AgentCapabilityConfig::with_config(
                "web_fetch",
                serde_json::json!({"enable_file_download": false})
            )
        );
        assert_eq!(snapshot.capabilities[1].capability_id(), "current_time");
    }

    #[test]
    fn capability_ref_round_trips_framework_persistence_and_worker_resolution() {
        // EVE-873: one neutral reference/config representation. A Framework
        // `CapabilityRef` serializes to the persisted attachment shape, loads
        // back as `AgentCapabilityConfig` (the same type), and survives
        // snapshot projection (the worker resolution input) unchanged.
        let framework_ref = everruns_capability::CapabilityRef::new("web_fetch")
            .config(serde_json::json!({"enable_file_download": true}));
        let persisted = serde_json::to_value(&framework_ref).unwrap();
        assert_eq!(
            persisted,
            serde_json::json!({
                "ref": "web_fetch",
                "config": {"enable_file_download": true}
            }),
            "wire shape is the persisted attachment row shape"
        );

        let attachment: AgentCapabilityConfig = serde_json::from_value(persisted).unwrap();
        assert_eq!(attachment, framework_ref);

        let (harness_id, agent_id, session_id) = ids();
        let mut harness = harness();
        harness.capabilities = vec![attachment.clone()];
        let agent = agent(agent_id, harness_id);
        let session = session(session_id, harness_id, Some(agent_id));
        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        assert_eq!(snapshot.capabilities[0], framework_ref);
        assert_eq!(
            serde_json::to_value(&snapshot.capabilities[0]).unwrap(),
            serde_json::to_value(&framework_ref).unwrap()
        );
    }

    #[test]
    fn persisted_capability_config_debug_redacts_values() {
        // Attachment rows flow through logs and error contexts; their Debug
        // output must never leak config payloads (which may carry handles or
        // misplaced credentials).
        let attachment = AgentCapabilityConfig::with_config(
            "vendor.search",
            serde_json::json!({"api_key": "sk-super-secret"}),
        );
        let debug = format!("{attachment:?}");
        assert!(debug.contains("vendor.search"));
        assert!(!debug.contains("sk-super-secret"));
        assert!(!debug.contains("api_key"));
    }

    #[test]
    fn precedence_initial_files_override_by_path() {
        let (harness_id, agent_id, session_id) = ids();
        let mut harness = harness();
        harness.initial_files = vec![file("/config.txt", "harness"), file("/keep.txt", "keep")];
        let mut agent = agent(agent_id, harness_id);
        agent.initial_files = vec![file("config.txt", "agent")];
        let session = session(session_id, harness_id, Some(agent_id));

        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        assert_eq!(snapshot.initial_files.len(), 2);
        assert_eq!(snapshot.initial_files[0].content, "agent");
        assert_eq!(snapshot.initial_files[1].content, "keep");
    }

    #[test]
    fn precedence_mcp_servers_override_by_name() {
        let (harness_id, agent_id, session_id) = ids();
        let mut harness = harness();
        harness.mcp_servers.insert(
            "docs".into(),
            ScopedMcpServer {
                url: "https://harness.example.com/mcp".into(),
                ..Default::default()
            },
        );
        let agent = agent(agent_id, harness_id);
        let mut session = session(session_id, harness_id, Some(agent_id));
        session.mcp_servers.insert(
            "docs".into(),
            ScopedMcpServer {
                url: "https://session.example.com/mcp".into(),
                ..Default::default()
            },
        );

        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        assert_eq!(
            snapshot.mcp_servers.get("docs").map(|s| s.url.as_str()),
            Some("https://session.example.com/mcp")
        );
    }

    #[test]
    fn precedence_network_access_narrows() {
        let (harness_id, agent_id, session_id) = ids();
        let mut harness = harness();
        harness.network_access = Some(NetworkAccessList::allow_only([
            "*.example.com",
            "api.github.com",
        ]));
        let agent = agent(agent_id, harness_id);
        let mut session = session(session_id, harness_id, Some(agent_id));
        session.network_access = Some(NetworkAccessList::allow_only(["api.example.com"]));

        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        let acl = snapshot.network_access.unwrap();
        assert_eq!(acl.allowed, vec!["api.example.com".to_string()]);
    }

    #[test]
    fn precedence_iteration_controls_leaf_wins() {
        let (harness_id, agent_id, session_id) = ids();
        let harness = harness();
        let mut agent = agent(agent_id, harness_id);
        agent.max_iterations = Some(40);
        agent.parallel_tool_calls = Some(true);
        let mut session = session(session_id, harness_id, Some(agent_id));
        session.max_iterations = Some(5);
        session.parallel_tool_calls = Some(false);

        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        assert_eq!(snapshot.max_iterations, Some(5));
        assert_eq!(snapshot.parallel_tool_calls, Some(false));
    }

    #[test]
    fn correlation_values_are_copied() {
        let (harness_id, agent_id, session_id) = ids();
        let harness = harness();
        let agent = agent(agent_id, harness_id);
        let session = session(session_id, harness_id, Some(agent_id));

        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        assert_eq!(snapshot.session_id, session_id);
        assert_eq!(snapshot.workspace_id, session.workspace_id);
        assert_eq!(snapshot.harness_id, harness_id);
        assert_eq!(snapshot.agent_id, Some(agent_id));
        assert_eq!(snapshot.organization_id, session.organization_id);
        assert_eq!(snapshot.locale.as_deref(), Some("en-US"));
        assert_eq!(snapshot.tags, vec!["tag-a".to_string()]);
    }

    // --- Projection failures ---
    //
    // Harness lifecycle and inheritance failures (empty chain, chain/leaf
    // mismatch, archived/deleted records, cycles) moved to the platform
    // loading seam (EVE-881); they are covered by the `everruns-platform`
    // `resolve_execution_harness` tests and the hosted store adapters.

    #[test]
    fn projection_fails_on_missing_agent() {
        let (harness_id, agent_id, session_id) = ids();
        let harness = harness();
        let session = session(session_id, harness_id, Some(agent_id));
        assert!(ResolvedExecutionSnapshot::project(&harness, None, &session).is_err());
    }

    #[test]
    fn projection_fails_on_mismatched_agent() {
        let (harness_id, agent_id, session_id) = ids();
        let harness = harness();
        let other_agent = agent(AgentId::from_seed(99), harness_id);
        let session = session(session_id, harness_id, Some(agent_id));
        assert!(
            ResolvedExecutionSnapshot::project(&harness, Some(&other_agent), &session).is_err()
        );
    }

    #[tokio::test]
    async fn loader_fails_on_missing_records_before_execution() {
        let (harness_id, agent_id, session_id) = ids();
        let harness_store = TestHarnessStore::new();
        let agent_store = TestAgentStore::new();
        let session_store = crate::test_fixtures::TestSessionStore::new();

        // Missing session (also the cross-tenant shape: org-scoped stores
        // return None for records outside the caller's org).
        let error =
            load_execution_snapshot(&harness_store, &agent_store, &session_store, session_id)
                .await
                .unwrap_err();
        assert!(error.to_string().contains("not found"), "{error}");

        // Session present but harness/agent missing.
        session_store
            .add_session(session(session_id, harness_id, Some(agent_id)))
            .await;
        assert!(
            load_execution_snapshot(&harness_store, &agent_store, &session_store, session_id)
                .await
                .is_err()
        );

        harness_store.add_harness(harness_id, harness()).await;
        assert!(
            load_execution_snapshot(&harness_store, &agent_store, &session_store, session_id)
                .await
                .is_err()
        );

        agent_store.add_agent(agent(agent_id, harness_id)).await;
        assert!(
            load_execution_snapshot(&harness_store, &agent_store, &session_store, session_id)
                .await
                .is_ok()
        );
    }

    // --- Secret-free, deterministic Debug/serialization ---

    #[test]
    fn snapshot_excludes_platform_metadata_and_credential_values() {
        let (harness_id, agent_id, session_id) = ids();
        let harness = harness();
        let agent = agent(agent_id, harness_id);
        let mut session = session(session_id, harness_id, Some(agent_id));
        session.mcp_servers.insert(
            "docs".into(),
            ScopedMcpServer {
                url: "https://mcp.example.com".into(),
                headers: [(
                    "Authorization".to_string(),
                    "Bearer SECRET-HEADER-MARKER".to_string(),
                )]
                .into_iter()
                .collect(),
                env: [("API_KEY".to_string(), "SECRET-ENV-MARKER".to_string())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            },
        );

        let snapshot =
            ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();

        let serialized = serde_json::to_string(&snapshot).unwrap();
        let debugged = format!("{snapshot:?}");
        for surface in [serialized.as_str(), debugged.as_str()] {
            // Credential values from scoped MCP config never appear.
            assert!(!surface.contains("SECRET-HEADER-MARKER"), "{surface}");
            assert!(!surface.contains("SECRET-ENV-MARKER"), "{surface}");
            // UI/platform-only session metadata never appears.
            assert!(!surface.contains("UI-TITLE-MARKER"), "{surface}");
            assert!(!surface.contains("UI-PREVIEW-MARKER"), "{surface}");
        }

        // Non-secret scope survives: server name, url, and header *names*.
        let docs = snapshot.mcp_servers.get("docs").unwrap();
        assert_eq!(docs.url, "https://mcp.example.com");
        assert_eq!(docs.header_names, vec!["Authorization".to_string()]);
        assert_eq!(docs.env_names, vec!["API_KEY".to_string()]);
    }

    #[test]
    fn snapshot_serialization_is_deterministic_and_round_trips() {
        let (harness_id, agent_id, session_id) = ids();
        let mut harness = harness();
        harness.embedder_metadata = HashMap::from([
            ("zeta".to_string(), "z".to_string()),
            ("alpha".to_string(), "a".to_string()),
        ]);
        let agent = agent(agent_id, harness_id);
        let session = session(session_id, harness_id, Some(agent_id));

        let first = ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();
        let second = ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session).unwrap();

        let first_json = serde_json::to_string(&first).unwrap();
        let second_json = serde_json::to_string(&second).unwrap();
        assert_eq!(first_json, second_json);

        let round_tripped: ResolvedExecutionSnapshot = serde_json::from_str(&first_json).unwrap();
        assert_eq!(serde_json::to_string(&round_tripped).unwrap(), first_json);

        // Map metadata serializes in sorted key order regardless of input order.
        let alpha = first_json.find("alpha").unwrap();
        let zeta = first_json.find("zeta").unwrap();
        assert!(alpha < zeta);
    }
}
