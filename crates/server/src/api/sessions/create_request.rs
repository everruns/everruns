use super::*;

/// Request to create a session
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// How this session was started. Clients may declare `chat`, `playground`, or `api`
    /// (the default); every other source is
    /// server-owned so the sessions facet rail stays trustworthy.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "chat")]
    pub source: Option<crate::records::SessionSource>,
    /// ID of the harness for this session (format: harness_{32-hex}).
    /// If omitted, the harness is derived from the agent (when one is supplied),
    /// else the org default harness, else the built-in fallback. New orgs default
    /// that to Conversation. Mutually exclusive with `harness_name`.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: Option<HarnessId>,
    /// Harness name (e.g. "conversation", "deep-research").
    /// Alternative to `harness_id` — looked up by name within the org.
    /// Mutually exclusive with `harness_id`.
    #[serde(default)]
    #[schema(example = "conversation")]
    pub harness_name: Option<String>,
    /// ID of the agent to work in this session (optional, format: agent_{32-hex}).
    /// When supplied without a harness, the session inherits the agent's harness.
    /// Mutually exclusive with `agent_name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// Name of the agent to work in this session (optional).
    /// Alternative to `agent_id` — looked up by name within the org.
    /// Mutually exclusive with `agent_id`.
    #[serde(default)]
    #[schema(example = "support")]
    pub agent_name: Option<String>,
    /// Optional resident virtual user used for unattended/background execution.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001")]
    pub virtual_user_id: Option<VirtualUserId>,
    /// Fixed Playground end user. Defaults to the caller's linked virtual user. Only valid with source=playground.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub playground_user_id: Option<VirtualUserId>,
    /// Human-readable title for the session.
    #[serde(default)]
    #[schema(example = "Debug login issue")]
    pub title: Option<String>,
    /// Optional objective for the session. Visible to the agent at system-prompt level.
    #[serde(default)]
    #[schema(example = "Investigate the queue latency regression and propose a fix")]
    pub goal: Option<String>,
    /// Session locale (BCP 47, e.g. `uk-UA`).
    #[serde(default)]
    #[schema(example = "uk-UA")]
    pub locale: Option<String>,
    /// Tags for organizing and filtering sessions.
    #[serde(default)]
    #[schema(example = json!(["debugging", "urgent"]))]
    pub tags: Vec<String>,
    /// The ID of the LLM model to use for this session.
    /// Overrides the agent's default model if specified.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (additive to agent capabilities).
    /// Applied after agent capabilities when building RuntimeAgent.
    #[serde(default)]
    #[schema(
        value_type = Vec<crate::records::CapabilityRefSchema>,
        example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}])
    )]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Execution environment for this Session. Omit to inherit the Agent's
    /// default profile, use `{ "use": "name" }` to select a named profile, or
    /// inline a profile for a caller-authored one-off environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentSelection>,
    /// Client-side tools for this session (additive to agent tools).
    /// These tools are sent to the LLM but executed by the client.
    #[serde(default, deserialize_with = "deserialize_client_side_tools")]
    #[schema(example = json!([{"type": "client_side", "name": "open_url", "description": "Open URL in the user's browser", "parameters": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}}]))]
    pub tools: Vec<ToolDefinition>,
    /// Remote MCP servers scoped to this session only.
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: ScopedMcpServers,
    /// Optional session-level system prompt override.
    /// Prepended to the agent's system prompt when building RuntimeAgent.
    #[serde(default)]
    #[schema(
        example = "You are debugging a production incident. Be concise and cite log lines verbatim."
    )]
    pub system_prompt: Option<String>,
    /// Session-level initial files (additive to agent initial_files).
    /// Files with matching paths override agent/harness files; new paths are appended.
    #[serde(default)]
    #[schema(example = json!([{"path": "README.md", "content": "# Project notes\n"}]))]
    pub initial_files: Vec<everruns_core::InitialFile>,
    /// Session-level client hints — arbitrary key-value pairs that tell the
    /// server what the client can handle. These are defaults for every turn;
    /// per-message `controls.hints` override these key-by-key (shallow merge).
    ///
    /// Three hints decide whether a turn may pause rather than talk past the
    /// user: `setup_connection`, `url_elicitation`, and `ask_user` (each names
    /// the card the client renders). A pause whose hint is absent does not park
    /// — an unhinted `ask_user` resolves with the model's defaults (EVE-1057).
    #[serde(default)]
    #[schema(example = json!({"setup_connection": true, "url_elicitation": true, "ask_user": true, "rich_media": true}))]
    pub hints: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Network access list controlling which hosts/URLs this session can reach.
    /// Merged with harness and agent layers (allowed: intersect, blocked: union).
    /// Example shape is defined on `NetworkAccessList`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 20)]
    pub max_iterations: Option<usize>,
    /// Request-level parallel tool calling preference (EVE-598). `true` signals
    /// the provider that parallel tool calls are wanted; `false` requests at
    /// most one tool call per turn and forces serial execution. Omit to inherit
    /// the agent/harness preference or the provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub parallel_tool_calls: Option<bool>,
    /// Internal: parent session for governed subagent depth tracking.
    /// Set by the worker when spawning a child session so nested delegation can
    /// be bounded by max_subagent_depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(ignore)]
    pub parent_session_id: Option<SessionId>,
    /// Internal: lineage source when creating a detached peer session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(ignore)]
    pub forked_from_session_id: Option<SessionId>,
    /// Internal: org-validated budget root for detached peer sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(ignore)]
    pub budget_root_session_id: Option<SessionId>,
    /// Internal: how to seed a detached peer session from its lineage source.
    #[serde(default)]
    #[schema(ignore)]
    pub seed: SessionSeedMode,
    /// Attach this session to an existing Workspace (format: `wsp_<32-hex>`)
    /// instead of auto-creating a default per-session workspace. The workspace
    /// must exist in the caller's org and be `active`. Lets multiple sessions
    /// share one working filesystem. Omit for the default 1:1 behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "wsp_01933b5a00007000800000000000001")]
    pub workspace_id: Option<WorkspaceId>,
}
