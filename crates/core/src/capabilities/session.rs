//! Session Capability
//!
//! Provides session metadata tools:
//! - `write_session_title`: update session title
//! - `get_session_info`: return session id, title, agent name, and cumulative usage

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::events::TokenUsage;
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

pub const SESSION_CAPABILITY_ID: &str = "session";

/// Session capability - read/update session metadata.
pub struct SessionCapability;

impl Capability for SessionCapability {
    fn id(&self) -> &str {
        SESSION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Session"
    }

    fn description(&self) -> &str {
        "Read and update current session metadata like title and agent info."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Сесія",
            "Читання та оновлення метаданих поточної сесії, як-от назви й інформації про агента.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("panel-left")
    }

    fn category(&self) -> Option<&str> {
        Some("Session")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(WriteSessionTitleTool),
            Box::new(GetSessionInfoTool),
        ]
    }
}

/// Tool: write_session_title
pub struct WriteSessionTitleTool;

#[async_trait]
impl Tool for WriteSessionTitleTool {
    fn name(&self) -> &str {
        "write_session_title"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Write Session Title")
    }

    fn description(&self) -> &str {
        "Update the current session title."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "New session title"
                }
            },
            "required": ["title"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "write_session_title requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let title = match arguments.get("title").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: title"),
        };

        let Some(mutator) = &context.session_mutator else {
            return ToolExecutionResult::tool_error(
                "Session mutator not available in this context",
            );
        };

        match mutator
            .update_session_title(context.session_id, title.clone())
            .await
        {
            Ok(session) => ToolExecutionResult::success(json!({
                "session_id": session.id.to_string(),
                "title": session.title,
                "updated": true,
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }
}

/// Tool: get_session_info
pub struct GetSessionInfoTool;

#[async_trait]
impl Tool for GetSessionInfoTool {
    fn name(&self) -> &str {
        "get_session_info"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Session Info")
    }

    fn description(&self) -> &str {
        "Get current session metadata: id, title, locale, agent name, and cumulative token usage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "get_session_info requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(session_store) = &context.session_store else {
            return ToolExecutionResult::tool_error("Session store not available in this context");
        };

        let session = match session_store.get_session(context.session_id).await {
            Ok(Some(session)) => session,
            Ok(None) => return ToolExecutionResult::tool_error("Session not found"),
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        let agent_name = if let (Some(agent_id), Some(agent_store)) =
            (session.agent_id, &context.agent_store)
        {
            match agent_store.get_agent(agent_id).await {
                Ok(Some(agent)) => Some(agent.display_name.unwrap_or_else(|| agent.name.clone())),
                Ok(None) => None,
                Err(e) => return ToolExecutionResult::internal_error(e),
            }
        } else {
            None
        };

        ToolExecutionResult::success(json!({
            "session_id": session.id.to_string(),
            "title": session.title,
            "locale": session.locale,
            "agent_name": agent_name,
            "usage": session.usage.as_ref().map(usage_json),
        }))
    }
}

fn usage_json(usage: &TokenUsage) -> Value {
    json!({
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
        "cache_read_tokens": usage.cache_read_tokens,
        "cache_creation_tokens": usage.cache_creation_tokens,
        "total_tokens": usage.total_tokens(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentStatus};
    use crate::error::Result;
    use crate::session::{Session, SessionStatus};
    use crate::typed_id::{AgentId, HarnessId, ModelId, SessionId};
    use crate::{AgentCapabilityConfig, Tool};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockSessionStore {
        session: Arc<Mutex<Option<Session>>>,
    }

    #[async_trait]
    impl crate::traits::SessionStore for MockSessionStore {
        async fn get_session(&self, _session_id: SessionId) -> Result<Option<Session>> {
            Ok(self.session.lock().expect("poisoned").clone())
        }
    }

    #[derive(Clone)]
    struct MockSessionMutator {
        session: Arc<Mutex<Session>>,
    }

    #[async_trait]
    impl crate::traits::SessionMutator for MockSessionMutator {
        async fn update_session_title(
            &self,
            _session_id: SessionId,
            title: String,
        ) -> Result<Session> {
            let mut session = self.session.lock().expect("poisoned");
            session.title = Some(title);
            Ok(session.clone())
        }

        async fn upsert_session_mcp_servers(
            &self,
            _session_id: SessionId,
            servers: crate::mcp_server::ScopedMcpServers,
        ) -> Result<()> {
            // MERGE (last-wins by name), mirroring the production semantics.
            let mut session = self.session.lock().expect("poisoned");
            session.mcp_servers.extend(servers);
            Ok(())
        }
    }

    struct MockAgentStore {
        agent: Option<Agent>,
    }

    #[async_trait]
    impl crate::traits::AgentStore for MockAgentStore {
        async fn get_agent(&self, _agent_id: AgentId) -> Result<Option<Agent>> {
            Ok(self.agent.clone())
        }
    }

    fn build_session(agent_id: Option<AgentId>) -> Session {
        let session_id = SessionId::new();
        Session {
            id: session_id,
            // Default 1:1 session<->workspace: workspace.id mirrors the session id.
            workspace_id: crate::WorkspaceId::from_uuid(session_id.uuid()),
            organization_id: "org_00000000000000000000000000000001".to_string(),
            harness_id: HarnessId::new(),
            agent_id,
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: crate::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: Some("Old title".to_string()),
            locale: None,
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: Some(ModelId::new()),
            capabilities: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            status: SessionStatus::Idle,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            active_schedule_count: None,
            features: vec![],
            parent_session_id: None,
            blueprint_id: None,
            blueprint_config: None,
        }
    }

    #[tokio::test]
    async fn write_session_title_updates_title() {
        let session = build_session(None);
        let session_id = session.id;
        let mut context = ToolContext::new(session_id);
        context.session_mutator = Some(Arc::new(MockSessionMutator {
            session: Arc::new(Mutex::new(session)),
        }));

        let tool = WriteSessionTitleTool;
        let result = tool
            .execute_with_context(json!({"title": "New title"}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["title"], "New title");
                assert_eq!(value["updated"], true);
            }
            _ => panic!("expected success"),
        }
    }

    #[tokio::test]
    async fn get_session_info_returns_agent_name_when_assigned() {
        let agent_id = AgentId::new();
        let session = build_session(Some(agent_id));
        let session_id = session.id;

        let agent = Agent {
            public_id: agent_id,
            internal_id: agent_id.uuid(),
            name: "research-agent".to_string(),
            display_name: Some("Research Agent".to_string()),
            description: Some("desc".to_string()),
            system_prompt: "prompt".to_string(),
            default_model_id: None,
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("session")],
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            tools: vec![],
            mcp_servers: Default::default(),
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        };

        let context = ToolContext::new(session_id)
            .with_session_store(Arc::new(MockSessionStore {
                session: Arc::new(Mutex::new(Some(session))),
            }))
            .with_agent_store(Arc::new(MockAgentStore { agent: Some(agent) }));

        let tool = GetSessionInfoTool;
        let result = tool.execute_with_context(json!({}), &context).await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["title"], "Old title");
                assert_eq!(value["agent_name"], "Research Agent");
                assert!(value["usage"].is_null());
            }
            _ => panic!("expected success"),
        }
    }

    #[tokio::test]
    async fn get_session_info_returns_cumulative_usage() {
        let mut session = build_session(None);
        session.usage = Some(TokenUsage::with_cache(120, 45, Some(30), Some(10)));
        let session_id = session.id;

        let context = ToolContext::new(session_id).with_session_store(Arc::new(MockSessionStore {
            session: Arc::new(Mutex::new(Some(session))),
        }));

        let tool = GetSessionInfoTool;
        let result = tool.execute_with_context(json!({}), &context).await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["usage"]["input_tokens"], 120);
                assert_eq!(value["usage"]["output_tokens"], 45);
                assert_eq!(value["usage"]["cache_read_tokens"], 30);
                assert_eq!(value["usage"]["cache_creation_tokens"], 10);
                assert_eq!(value["usage"]["total_tokens"], 165);
            }
            _ => panic!("expected success"),
        }
    }

    #[tokio::test]
    async fn upsert_session_mcp_servers_merges_not_replaces() {
        use crate::mcp_server::{ScopedMcpServer, ScopedMcpServers};
        use crate::traits::SessionMutator;

        // Seed the session with one pre-existing scoped MCP server.
        let mut session = build_session(None);
        let mut existing: ScopedMcpServers = Default::default();
        existing.insert(
            "existing".to_string(),
            ScopedMcpServer {
                url: "https://existing.example.com/mcp".to_string(),
                ..Default::default()
            },
        );
        session.mcp_servers = existing;

        let shared = Arc::new(Mutex::new(session));
        let mutator = MockSessionMutator {
            session: shared.clone(),
        };

        // Upsert a new, differently-named server.
        let mut incoming: ScopedMcpServers = Default::default();
        incoming.insert(
            "attached".to_string(),
            ScopedMcpServer {
                url: "https://attached.example.com/mcp".to_string(),
                ..Default::default()
            },
        );
        mutator
            .upsert_session_mcp_servers(SessionId::new(), incoming)
            .await
            .expect("upsert succeeds");

        let after = shared.lock().expect("poisoned").mcp_servers.clone();
        // MERGE: the pre-existing entry is preserved alongside the new one.
        assert_eq!(after.len(), 2);
        assert!(after.contains_key("existing"));
        assert_eq!(
            after.get("attached").map(|s| s.url.as_str()),
            Some("https://attached.example.com/mcp")
        );

        // Last-wins on the same name.
        let mut overlay: ScopedMcpServers = Default::default();
        overlay.insert(
            "attached".to_string(),
            ScopedMcpServer {
                url: "https://attached-v2.example.com/mcp".to_string(),
                ..Default::default()
            },
        );
        mutator
            .upsert_session_mcp_servers(SessionId::new(), overlay)
            .await
            .expect("upsert succeeds");
        let after = shared.lock().expect("poisoned").mcp_servers.clone();
        assert_eq!(after.len(), 2);
        assert_eq!(
            after.get("attached").map(|s| s.url.as_str()),
            Some("https://attached-v2.example.com/mcp")
        );
    }
}
