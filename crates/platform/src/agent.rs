// Stored Agent and AgentVersion persistence records (EVE-877).
//
// Decision: Agent persistence and lifecycle are a platform concern. These
// records — lifecycle status, versioning and publication metadata, fork
// lineage, timestamps, usage — moved here from `everruns-core`. The execution
// kernel consumes only the portable `everruns_core::AgentDefinition`, produced
// at the loading seam by [`Agent::execution_definition`], which enforces
// archived/deleted validation before host execution.
//
// Design Decision: Dual-ID pattern (see knowledge/foundations/id-schema.md)
// - public_id: AgentId (external, API-facing, client-supplied or auto-generated)
// - internal_id: Uuid (internal PK, used for FK references, never exposed in API)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use everruns_capability::CapabilityRef as AgentCapabilityConfig;
use everruns_core::AgentDefinition;
use everruns_core::events::TokenUsage;
use everruns_core::mcp_server::{ScopedMcpServers, scoped_mcp_servers_is_empty};
use everruns_core::network_access::NetworkAccessList;
use everruns_core::session_file::InitialFile;
use everruns_provider::error::AgentLoopError;
use everruns_provider::tool_types::ToolDefinition;
use everruns_provider::typed_id::{AgentId, AgentVersionId, HarnessId, ModelId, PrincipalId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Agent lifecycle status.
/// - `active`: Agent is available for use
/// - `archived`: Agent is hidden from listings and cannot be modified or assigned
/// - `deleted`: Agent is a tombstone kept only for historical references
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "active"))]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// Agent is available for use.
    Active,
    /// Agent is hidden from listings and cannot be modified or assigned.
    Archived,
    /// Agent is deleted and should only survive as a tombstone for references.
    Deleted,
}

/// Reason a version was created. Stored as lower_snake_case text.
/// One of `auto`, `manual`, `patch`, `minor`, `major`, `import`, `rollback`, `fork`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "manual"))]
#[serde(rename_all = "snake_case")]
pub enum AgentVersionChangeKind {
    Auto,
    Manual,
    Patch,
    Minor,
    Major,
    Import,
    Rollback,
    Fork,
}

impl std::fmt::Display for AgentVersionChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentVersionChangeKind::Auto => write!(f, "auto"),
            AgentVersionChangeKind::Manual => write!(f, "manual"),
            AgentVersionChangeKind::Patch => write!(f, "patch"),
            AgentVersionChangeKind::Minor => write!(f, "minor"),
            AgentVersionChangeKind::Major => write!(f, "major"),
            AgentVersionChangeKind::Import => write!(f, "import"),
            AgentVersionChangeKind::Rollback => write!(f, "rollback"),
            AgentVersionChangeKind::Fork => write!(f, "fork"),
        }
    }
}

impl From<&str> for AgentVersionChangeKind {
    fn from(s: &str) -> Self {
        match s {
            "auto" => AgentVersionChangeKind::Auto,
            "patch" => AgentVersionChangeKind::Patch,
            "minor" => AgentVersionChangeKind::Minor,
            "major" => AgentVersionChangeKind::Major,
            "import" => AgentVersionChangeKind::Import,
            "rollback" => AgentVersionChangeKind::Rollback,
            "fork" => AgentVersionChangeKind::Fork,
            _ => AgentVersionChangeKind::Manual,
        }
    }
}

/// Immutable snapshot of an Agent's authored and resolved runtime config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AgentVersion {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "agentver_01933b5a000070008000000000000001"))]
    pub public_id: AgentVersionId,
    /// Internal database UUID. Not part of the public identifier surface; skipped during serialization.
    #[serde(skip, default = "uuid::Uuid::nil")]
    pub internal_id: uuid::Uuid,
    /// Owning agent's prefixed public identifier.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "agent_01933b5a000070008000000000000001"))]
    pub agent_id: AgentId,
    /// Monotonic per-agent version sequence number (1, 2, 3, ...). Increments on every snapshot.
    #[cfg_attr(feature = "openapi", schema(example = 7))]
    pub version_number: i32,
    /// Semantic version major component.
    #[cfg_attr(feature = "openapi", schema(example = 1))]
    pub semver_major: i32,
    /// Semantic version minor component.
    #[cfg_attr(feature = "openapi", schema(example = 4))]
    pub semver_minor: i32,
    /// Semantic version patch component.
    #[cfg_attr(feature = "openapi", schema(example = 2))]
    pub semver_patch: i32,
    /// Combined semver string for display (e.g. `1.4.2`).
    #[cfg_attr(feature = "openapi", schema(example = "1.4.2"))]
    pub version: String,
    /// Whether this version was explicitly published by a user. Published versions are user-controlled semver releases; unpublished rows are automatic draft snapshots kept for audit and rollback.
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub is_published: bool,
    /// Version this one was forked or branched from, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub parent_version_id: Option<AgentVersionId>,
    /// When this version is a copy of another version (e.g. a manual rollback), the original source. `None` for ordinary snapshots.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub source_version_id: Option<AgentVersionId>,
    /// Identity of the principal (user or agent identity) that created this version. `None` for system-generated snapshots.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub created_by_principal_id: Option<PrincipalId>,
    /// Classification of why this version was created (manual publish, automatic draft, rollback, fork, etc.).
    pub change_kind: AgentVersionChangeKind,
    /// Human-readable summary of changes in this version (release notes). `None` if not provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "Switched default model to claude-sonnet-4-6; added refund-runbook capability."
        )
    )]
    pub summary: Option<String>,
    /// Stable hash of `resolved_config` used to deduplicate adjacent identical snapshots.
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "blake3:9f1e2a4c3d5b6e8a0b2c4d6e8f0a1b3c5d7e9f0a1b2c4d6e8f0a1b2c4d6e8f0a"
        )
    )]
    pub config_hash: String,
    /// User-authored agent configuration JSON, exactly as submitted. Capabilities, MCP refs, model selection live here.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub authored_config: serde_json::Value,
    /// Resolved configuration after applying harness, capability, and platform layers. This is what the runtime executes against.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub resolved_config: serde_json::Value,
    /// Timestamp when this version was created (RFC 3339).
    #[cfg_attr(feature = "openapi", schema(example = "2026-04-20T14:22:00Z"))]
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Active => write!(f, "active"),
            AgentStatus::Archived => write!(f, "archived"),
            AgentStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for AgentStatus {
    fn from(s: &str) -> Self {
        match s {
            "archived" => AgentStatus::Archived,
            "deleted" => AgentStatus::Deleted,
            _ => AgentStatus::Active,
        }
    }
}

// Stored Agent record: authored configuration plus platform persistence
// metadata (lifecycle status, versioning, fork lineage, timestamps, usage).
// The doc comment below is part of the public OpenAPI schema description and
// intentionally unchanged by the EVE-877 move.
/// Agent configuration for agentic loop.
/// An agent defines the behavior and capabilities of an AI assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Agent {
    /// External identifier (agent_<32-hex>). Shown as "id" in API.
    /// Client-supplied or auto-generated.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "agent_01933b5a000070008000000000000001"))]
    pub public_id: AgentId,
    /// Internal UUID primary key. Used for FK references. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Name, unique per org (e.g. "customer-support").
    #[cfg_attr(feature = "openapi", schema(example = "customer-support"))]
    pub name: String,
    /// Human-readable display name shown in UI (e.g. "Customer Support Agent").
    /// Falls back to `name` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Customer Support Agent"))]
    pub display_name: Option<String>,
    /// Human-readable description of what the agent does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Handles refund and shipping questions; escalates billing disputes.")
    )]
    pub description: Option<String>,
    /// System prompt that defines the agent's behavior.
    /// Sent as the first message in every conversation.
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "You are a friendly customer support agent for Acme Corp. Verify orders before issuing refunds. Escalate any billing disputes to a human."
        )
    )]
    pub system_prompt: String,
    /// Default LLM model ID for this agent.
    /// Can be overridden at the session level.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub default_model_id: Option<ModelId>,
    /// Harness that supplies the base execution environment for this agent.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
    pub harness_id: HarnessId,
    /// Default immutable version used by deployments that choose the default policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agentver_01933b5a00007000800000000000001"))]
    pub default_version_id: Option<AgentVersionId>,
    /// Source agent for a forked agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001"))]
    pub forked_from_agent_id: Option<AgentId>,
    /// Source version for a forked agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agentver_01933b5a00007000800000000000001"))]
    pub forked_from_version_id: Option<AgentVersionId>,
    /// Root agent lineage identifier for grouping fork families.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001"))]
    pub root_agent_id: Option<AgentId>,
    /// Tags for organizing and filtering agents.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = json!(["support", "production"])))]
    pub tags: Vec<String>,
    /// Capabilities enabled for this agent with per-agent configuration.
    /// Capabilities add tools and system prompt modifications.
    #[serde(default)]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Vec<crate::CapabilityRefSchema>)
    )]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_files: Vec<InitialFile>,
    /// Network access list controlling which hosts/URLs agent sessions can reach.
    /// Merged with harness and session layers (allowed: intersect, blocked: union).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 50))]
    pub max_iterations: Option<usize>,
    /// Request-level parallel tool calling preference (EVE-598).
    ///
    /// `None` (default) preserves provider defaults. `Some(true)` signals the
    /// provider that parallel tool calls are wanted; `Some(false)` requests at
    /// most one tool call per turn and forces serial execution. Merged across
    /// harness/agent/session layers (overlay wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub parallel_tool_calls: Option<bool>,
    /// Client-side tools registered for this agent.
    /// These tools are executed by the client, not the server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    /// Remote MCP servers scoped to this agent and inherited by its sessions.
    #[serde(
        default,
        rename = "mcpServers",
        alias = "mcp_servers",
        skip_serializing_if = "scoped_mcp_servers_is_empty"
    )]
    pub mcp_servers: ScopedMcpServers,
    /// Current lifecycle status of the agent.
    pub status: AgentStatus,
    /// Timestamp when the agent was created.
    #[cfg_attr(feature = "openapi", schema(example = "2026-04-01T10:00:00Z"))]
    pub created_at: DateTime<Utc>,
    /// Timestamp when the agent was last updated.
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-20T14:00:00Z"))]
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the agent was archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-26T00:00:00Z"))]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when the agent was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-26T00:00:00Z"))]
    pub deleted_at: Option<DateTime<Utc>>,
    /// Cumulative token usage across all sessions for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

impl Agent {
    /// Project this stored record into the portable authored execution
    /// configuration, without lifecycle validation.
    ///
    /// Use [`Agent::execution_definition`] at execution loading seams; this
    /// plain projection serves non-execution consumers (e.g. handoff target
    /// overlay assembly) that historically saw the record regardless of
    /// status.
    pub fn definition(&self) -> AgentDefinition {
        AgentDefinition {
            id: self.public_id,
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            description: self.description.clone(),
            system_prompt: self.system_prompt.clone(),
            default_model_id: self.default_model_id,
            capabilities: self.capabilities.clone(),
            initial_files: self.initial_files.clone(),
            network_access: self.network_access.clone(),
            max_iterations: self.max_iterations,
            parallel_tool_calls: self.parallel_tool_calls,
            tools: self.tools.clone(),
            mcp_servers: self.mcp_servers.clone(),
        }
    }

    /// Project this stored record into the execution definition, enforcing
    /// lifecycle validation at the platform loading seam (EVE-877).
    ///
    /// Archived and deleted records fail here — before host execution — with
    /// the same error the snapshot projection historically produced.
    pub fn execution_definition(&self) -> everruns_provider::error::Result<AgentDefinition> {
        match self.status {
            AgentStatus::Active => Ok(self.definition()),
            AgentStatus::Archived | AgentStatus::Deleted => Err(AgentLoopError::config(format!(
                "agent {} is {} and cannot execute turns",
                self.public_id, self.status
            ))),
        }
    }

    /// Dependency-blocker probe answer for this record's lifecycle status.
    pub fn dependency_blocker(&self) -> Option<everruns_core::DependencyBlocker> {
        match self.status {
            AgentStatus::Active => None,
            AgentStatus::Archived => Some(everruns_core::DependencyBlocker::AgentArchived),
            AgentStatus::Deleted => Some(everruns_core::DependencyBlocker::AgentDeleted),
        }
    }
}

impl From<&Agent> for everruns_core::AgentConfigOverlay {
    fn from(agent: &Agent) -> Self {
        (&agent.definition()).into()
    }
}

/// Maximum length for an addressable name (agent, harness, etc.).
pub const MAX_ADDRESSABLE_NAME_LEN: usize = 64;

/// Validate an addressable name: `[a-z0-9]([a-z0-9-]*[a-z0-9])?`.
/// Max 64 chars, no consecutive hyphens, no leading/trailing hyphens.
/// Returns `Ok(())` or a human-readable error message.
pub fn validate_addressable_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if name.len() > MAX_ADDRESSABLE_NAME_LEN {
        return Err(format!(
            "name must be at most {MAX_ADDRESSABLE_NAME_LEN} characters"
        ));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("name must contain only lowercase letters, digits, and hyphens".to_string());
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("name must not start or end with a hyphen".to_string());
    }
    if name.contains("--") {
        return Err("name must not contain consecutive hyphens".to_string());
    }
    Ok(())
}

/// Generate a new agent public_id using UUIDv7.
pub fn generate_agent_public_id() -> AgentId {
    AgentId::new()
}

/// Validate an agent public_id string.
/// Must match format: agent_<32-lowercase-hex-chars>
pub fn validate_agent_public_id(s: &str) -> bool {
    s.parse::<AgentId>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_agent() -> Agent {
        Agent {
            public_id: "agent_01933b5a000070008000000000000001".parse().unwrap(),
            internal_id: Uuid::nil(),
            name: "test".to_string(),
            display_name: Some("Test".to_string()),
            description: None,
            system_prompt: "test".to_string(),
            default_model_id: None,
            harness_id: "harness_01933b5a000070008000000000000001".parse().unwrap(),
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            tools: vec![],
            mcp_servers: ScopedMcpServers::default(),
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        }
    }

    #[test]
    fn test_generate_agent_public_id() {
        let id = generate_agent_public_id();
        let s = id.to_string();
        assert!(s.starts_with("agent_"));
        assert_eq!(s.len(), 38); // "agent_" (6) + 32 hex chars
        assert!(validate_agent_public_id(&s));
    }

    #[test]
    fn test_validate_agent_public_id() {
        assert!(validate_agent_public_id(
            "agent_01933b5a000070008000000000000001"
        ));
        assert!(validate_agent_public_id(
            "agent_4ab3e8452f1442e9865e11d2032a579c"
        ));

        // Invalid cases
        assert!(!validate_agent_public_id(""));
        assert!(!validate_agent_public_id("agent_"));
        assert!(!validate_agent_public_id(
            "session_01933b5a000070008000000000000001"
        ));
        assert!(!validate_agent_public_id(
            "agent_4AB3E8452F1442E9865E11D2032A579C"
        )); // uppercase
        assert!(!validate_agent_public_id("agent_short"));
        assert!(!validate_agent_public_id("my-custom-agent"));
    }

    #[test]
    fn test_validate_addressable_name_valid() {
        assert!(validate_addressable_name("my-agent").is_ok());
        assert!(validate_addressable_name("agent1").is_ok());
        assert!(validate_addressable_name("a").is_ok());
        assert!(validate_addressable_name("customer-support").is_ok());
        assert!(validate_addressable_name("a-b-c").is_ok());
    }

    #[test]
    fn test_validate_addressable_name_invalid() {
        assert!(validate_addressable_name("").is_err());
        assert!(validate_addressable_name("-leading").is_err());
        assert!(validate_addressable_name("trailing-").is_err());
        assert!(validate_addressable_name("bad--double").is_err());
        assert!(validate_addressable_name("UPPERCASE").is_err());
        assert!(validate_addressable_name("has space").is_err());
        assert!(validate_addressable_name("Customer Support Agent").is_err());
        let long = "a".repeat(65);
        assert!(validate_addressable_name(&long).is_err());
    }

    #[test]
    fn test_agent_serde_public_id_as_id() {
        let agent = test_agent();
        let json = serde_json::to_value(&agent).unwrap();
        // public_id should serialize as "id"
        assert_eq!(json["id"], "agent_01933b5a000070008000000000000001");
        // internal_id should be skipped
        assert!(json.get("internal_id").is_none());
    }

    #[test]
    fn test_agent_deserialize_from_api_json() {
        let json = serde_json::json!({
            "id": "agent_01933b5a000070008000000000000001",
            "name": "test",
            "display_name": "Test",
            "system_prompt": "test",
            "harness_id": "harness_01933b5a000070008000000000000001",
            "status": "active",
            "tags": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        });

        let agent: Agent = serde_json::from_value(json).unwrap();
        assert_eq!(
            agent.public_id.to_string(),
            "agent_01933b5a000070008000000000000001"
        );
        // internal_id defaults to nil when not present in JSON
        assert_eq!(agent.internal_id, Uuid::nil());
    }

    #[test]
    fn definition_projects_authored_execution_configuration() {
        let mut agent = test_agent();
        agent.max_iterations = Some(7);
        agent.capabilities = vec![AgentCapabilityConfig::new("current_time")];

        let definition = agent.definition();
        assert_eq!(definition.id, agent.public_id);
        assert_eq!(definition.name, "test");
        assert_eq!(definition.system_prompt, "test");
        assert_eq!(definition.max_iterations, Some(7));
        assert_eq!(definition.capabilities.len(), 1);

        // Persistence metadata never enters the portable definition.
        let json = serde_json::to_value(&definition).unwrap();
        for field in ["status", "created_at", "default_version_id", "usage"] {
            assert!(json.get(field).is_none(), "{field} leaked into definition");
        }
    }

    #[test]
    fn execution_definition_fails_for_archived_and_deleted_records() {
        // EVE-877: lifecycle validation moved from snapshot projection to this
        // platform loading seam. The error text is unchanged.
        for status in [AgentStatus::Archived, AgentStatus::Deleted] {
            let mut agent = test_agent();
            agent.status = status.clone();
            let error = agent.execution_definition().unwrap_err().to_string();
            assert!(
                error.contains(&format!("is {status} and cannot execute turns")),
                "unexpected error for {status:?}: {error}"
            );
        }
        assert!(test_agent().execution_definition().is_ok());
    }

    #[test]
    fn dependency_blocker_distinguishes_archived_from_deleted() {
        let mut agent = test_agent();
        assert!(agent.dependency_blocker().is_none());
        agent.status = AgentStatus::Archived;
        assert!(matches!(
            agent.dependency_blocker(),
            Some(everruns_core::DependencyBlocker::AgentArchived)
        ));
        agent.status = AgentStatus::Deleted;
        assert!(matches!(
            agent.dependency_blocker(),
            Some(everruns_core::DependencyBlocker::AgentDeleted)
        ));
    }
}
