// In-memory storage implementation for dev mode
// Decision: Use parking_lot for thread-safe access
// Decision: UUIDs generated via uuid v7 (time-ordered)
//
// This implementation provides a PostgreSQL-compatible API backed by in-memory
// HashMaps, allowing the server to run without a database for development.

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::{
    AgentId, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, EventId, ImageId, McpServerId, ModelId,
    ProviderId, SessionId,
};
use parking_lot::RwLock;
use std::collections::HashMap;
use uuid::Uuid;

use super::models::*;

/// In-memory database for dev mode
/// All data is stored in memory and lost on restart
pub struct InMemoryDatabase {
    // TODO: Used in Phase 3 when org APIs are implemented
    #[allow(dead_code)]
    organizations: RwLock<HashMap<i64, OrganizationRow>>,
    #[allow(dead_code)]
    organization_members: RwLock<HashMap<(i64, Uuid), OrganizationMemberRow>>,
    users: RwLock<HashMap<Uuid, UserRow>>,
    api_keys: RwLock<HashMap<Uuid, ApiKeyRow>>,
    refresh_tokens: RwLock<HashMap<Uuid, RefreshTokenRow>>,
    agents: RwLock<HashMap<AgentId, AgentRow>>,
    sessions: RwLock<HashMap<SessionId, SessionRow>>,
    events: RwLock<HashMap<EventId, EventRow>>,
    llm_providers: RwLock<HashMap<ProviderId, LlmProviderRow>>,
    llm_models: RwLock<HashMap<ModelId, LlmModelRow>>,
    agent_capabilities: RwLock<HashMap<(AgentId, String), AgentCapabilityRow>>,
    session_files: RwLock<HashMap<Uuid, SessionFileRow>>,
    mcp_servers: RwLock<HashMap<McpServerId, McpServerRow>>,
    images: RwLock<HashMap<ImageId, ImageRow>>,
    // Event sequence counter per session
    event_sequences: RwLock<HashMap<SessionId, i32>>,
    // Session storage
    session_key_values: RwLock<HashMap<(SessionId, String), SessionKeyValueRow>>,
    session_secrets: RwLock<HashMap<(SessionId, String), SessionSecretRow>>,
}

impl Default for InMemoryDatabase {
    fn default() -> Self {
        let now = Utc::now();

        // Pre-create default organization
        let mut organizations = HashMap::new();
        organizations.insert(
            DEFAULT_ORG_ID,
            OrganizationRow {
                org_id: DEFAULT_ORG_ID,
                public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
                name: "Default Organization".to_string(),
                created_at: now,
                updated_at: now,
            },
        );

        Self {
            organizations: RwLock::new(organizations),
            organization_members: RwLock::new(HashMap::new()),
            users: RwLock::new(HashMap::new()),
            api_keys: RwLock::new(HashMap::new()),
            refresh_tokens: RwLock::new(HashMap::new()),
            agents: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
            events: RwLock::new(HashMap::new()),
            llm_providers: RwLock::new(HashMap::new()),
            llm_models: RwLock::new(HashMap::new()),
            agent_capabilities: RwLock::new(HashMap::new()),
            session_files: RwLock::new(HashMap::new()),
            mcp_servers: RwLock::new(HashMap::new()),
            images: RwLock::new(HashMap::new()),
            event_sequences: RwLock::new(HashMap::new()),
            session_key_values: RwLock::new(HashMap::new()),
            session_secrets: RwLock::new(HashMap::new()),
        }
    }
}

impl InMemoryDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    // ============================================
    // Users
    // ============================================

    pub async fn create_user(&self, input: CreateUserRow) -> Result<UserRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = UserRow {
            id,
            email: input.email,
            name: input.name,
            avatar_url: input.avatar_url,
            roles: serde_json::to_value(&input.roles)?,
            password_hash: input.password_hash,
            email_verified: input.email_verified,
            auth_provider: input.auth_provider,
            auth_provider_id: input.auth_provider_id,
            created_at: now,
            updated_at: now,
        };
        self.users.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRow>> {
        Ok(self
            .users
            .read()
            .values()
            .find(|u| u.email == email)
            .cloned())
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<UserRow>> {
        Ok(self.users.read().get(&id).cloned())
    }

    pub async fn get_user_by_oauth(
        &self,
        provider: &str,
        provider_id: &str,
    ) -> Result<Option<UserRow>> {
        Ok(self
            .users
            .read()
            .values()
            .find(|u| {
                u.auth_provider.as_deref() == Some(provider)
                    && u.auth_provider_id.as_deref() == Some(provider_id)
            })
            .cloned())
    }

    pub async fn update_user(&self, id: Uuid, input: UpdateUser) -> Result<Option<UserRow>> {
        let mut users = self.users.write();
        if let Some(user) = users.get_mut(&id) {
            if let Some(name) = input.name {
                user.name = name;
            }
            if let Some(avatar_url) = input.avatar_url {
                user.avatar_url = Some(avatar_url);
            }
            if let Some(roles) = input.roles {
                user.roles = serde_json::to_value(&roles)?;
            }
            if let Some(password_hash) = input.password_hash {
                user.password_hash = Some(password_hash);
            }
            if let Some(email_verified) = input.email_verified {
                user.email_verified = email_verified;
            }
            user.updated_at = Self::now();
            return Ok(Some(user.clone()));
        }
        Ok(None)
    }

    pub async fn list_users(&self, search: Option<&str>) -> Result<Vec<UserRow>> {
        let users = self.users.read();
        let mut result: Vec<_> = match search {
            Some(query) if !query.trim().is_empty() => {
                let pattern = query.trim().to_lowercase();
                users
                    .values()
                    .filter(|u| {
                        u.name.to_lowercase().contains(&pattern)
                            || u.email.to_lowercase().contains(&pattern)
                    })
                    .cloned()
                    .collect()
            }
            _ => users.values().cloned().collect(),
        };
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    // ============================================
    // API Keys
    // ============================================

    pub async fn create_api_key(&self, input: CreateApiKeyRow) -> Result<ApiKeyRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = ApiKeyRow {
            id,
            org_id: input.org_id,
            user_id: input.user_id,
            name: input.name,
            key_hash: input.key_hash,
            key_prefix: input.key_prefix,
            scopes: serde_json::to_value(&input.scopes)?,
            expires_at: input.expires_at,
            last_used_at: None,
            created_at: now,
        };
        self.api_keys.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyRow>> {
        Ok(self
            .api_keys
            .read()
            .values()
            .find(|k| k.key_hash == key_hash)
            .cloned())
    }

    pub async fn list_api_keys_for_user(&self, user_id: Uuid) -> Result<Vec<ApiKeyRow>> {
        let keys = self.api_keys.read();
        let mut result: Vec<_> = keys
            .values()
            .filter(|k| k.user_id == user_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn update_api_key_last_used(&self, id: Uuid) -> Result<()> {
        if let Some(key) = self.api_keys.write().get_mut(&id) {
            key.last_used_at = Some(Self::now());
        }
        Ok(())
    }

    pub async fn delete_api_key(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        let mut keys = self.api_keys.write();
        if let Some(key) = keys.get(&id)
            && key.user_id == user_id
        {
            keys.remove(&id);
            return Ok(true);
        }
        Ok(false)
    }

    // ============================================
    // Refresh Tokens
    // ============================================

    pub async fn create_refresh_token(
        &self,
        input: CreateRefreshTokenRow,
    ) -> Result<RefreshTokenRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = RefreshTokenRow {
            id,
            user_id: input.user_id,
            token_hash: input.token_hash,
            expires_at: input.expires_at,
            created_at: now,
        };
        self.refresh_tokens.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>> {
        Ok(self
            .refresh_tokens
            .read()
            .values()
            .find(|t| t.token_hash == token_hash)
            .cloned())
    }

    pub async fn delete_refresh_token(&self, id: Uuid) -> Result<bool> {
        Ok(self.refresh_tokens.write().remove(&id).is_some())
    }

    pub async fn delete_expired_refresh_tokens(&self) -> Result<u64> {
        let now = Self::now();
        let mut tokens = self.refresh_tokens.write();
        let to_remove: Vec<Uuid> = tokens
            .iter()
            .filter(|(_, t)| t.expires_at < now)
            .map(|(id, _)| *id)
            .collect();
        let count = to_remove.len() as u64;
        for id in to_remove {
            tokens.remove(&id);
        }
        Ok(count)
    }

    pub async fn delete_user_refresh_tokens(&self, user_id: Uuid) -> Result<u64> {
        let mut tokens = self.refresh_tokens.write();
        let to_remove: Vec<Uuid> = tokens
            .iter()
            .filter(|(_, t)| t.user_id == user_id)
            .map(|(id, _)| *id)
            .collect();
        let count = to_remove.len() as u64;
        for id in to_remove {
            tokens.remove(&id);
        }
        Ok(count)
    }

    // ============================================
    // Agents
    // ============================================

    pub async fn create_agent(&self, org_id: i64, input: CreateAgentRow) -> Result<AgentRow> {
        let now = Self::now();
        let id = AgentId::new();
        let row = AgentRow {
            id,
            public_id: input.public_id,
            org_id,
            name: input.name,
            description: input.description,
            system_prompt: input.system_prompt,
            default_model_id: input.default_model_id,
            tags: input.tags,
            tools: input.tools,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
        };
        self.agents.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create agent with a specific ID, idempotent (returns None if exists)
    pub async fn create_agent_with_id(
        &self,
        org_id: i64,
        id: AgentId,
        input: CreateAgentRow,
    ) -> Result<Option<AgentRow>> {
        let mut agents = self.agents.write();
        if agents.contains_key(&id) {
            return Ok(None); // Already exists
        }
        let now = Self::now();
        let row = AgentRow {
            id,
            public_id: input.public_id,
            org_id,
            name: input.name,
            description: input.description,
            system_prompt: input.system_prompt,
            default_model_id: input.default_model_id,
            tags: input.tags,
            tools: input.tools,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
        };
        agents.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_agent(&self, org_id: i64, id: AgentId) -> Result<Option<AgentRow>> {
        Ok(self
            .agents
            .read()
            .get(&id)
            .filter(|a| a.org_id == org_id)
            .cloned())
    }

    pub async fn get_agent_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentRow>> {
        Ok(self
            .agents
            .read()
            .values()
            .find(|a| a.org_id == org_id && a.public_id == public_id)
            .cloned())
    }

    pub async fn list_agents(&self, org_id: i64) -> Result<Vec<AgentRow>> {
        let agents = self.agents.read();
        let mut result: Vec<_> = agents
            .values()
            .filter(|a| a.org_id == org_id && a.status == "active")
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn get_agent_by_name(&self, org_id: i64, name: &str) -> Result<Option<AgentRow>> {
        Ok(self
            .agents
            .read()
            .values()
            .find(|a| a.org_id == org_id && a.name == name && a.status == "active")
            .cloned())
    }

    pub async fn update_agent(
        &self,
        org_id: i64,
        id: AgentId,
        input: UpdateAgent,
    ) -> Result<Option<AgentRow>> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&id).filter(|a| a.org_id == org_id) {
            if let Some(name) = input.name {
                agent.name = name;
            }
            if let Some(description) = input.description {
                agent.description = Some(description);
            }
            if let Some(system_prompt) = input.system_prompt {
                agent.system_prompt = system_prompt;
            }
            if let Some(default_model_id) = input.default_model_id {
                agent.default_model_id = Some(default_model_id);
            }
            if let Some(tags) = input.tags {
                agent.tags = tags;
            }
            if let Some(status) = input.status {
                agent.status = status;
            }
            agent.updated_at = Self::now();
            return Ok(Some(agent.clone()));
        }
        Ok(None)
    }

    pub async fn delete_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        // Delete capabilities first
        {
            let mut caps = self.agent_capabilities.write();
            let to_remove: Vec<_> = caps.keys().filter(|(aid, _)| *aid == id).cloned().collect();
            for key in to_remove {
                caps.remove(&key);
            }
        }
        // Only delete if org_id matches
        let mut agents = self.agents.write();
        if agents.get(&id).map(|a| a.org_id) == Some(org_id) {
            return Ok(agents.remove(&id).is_some());
        }
        Ok(false)
    }

    /// Upsert agent by public_id. Returns (row, was_created).
    pub async fn upsert_agent(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        let mut agents = self.agents.write();
        let existing_key = agents
            .iter()
            .find(|(_, a)| a.org_id == org_id && a.public_id == input.public_id)
            .map(|(k, _)| *k);

        if let Some(key) = existing_key {
            let agent = agents.get_mut(&key).unwrap();
            agent.name = input.name;
            agent.description = input.description;
            agent.system_prompt = input.system_prompt;
            agent.default_model_id = input.default_model_id;
            agent.tags = input.tags;
            agent.tools = input.tools;
            agent.status = "active".to_string();
            agent.updated_at = Self::now();
            Ok((agent.clone(), false))
        } else {
            let now = Self::now();
            let id = AgentId::new();
            let row = AgentRow {
                id,
                public_id: input.public_id,
                org_id,
                name: input.name,
                description: input.description,
                system_prompt: input.system_prompt,
                default_model_id: input.default_model_id,
                tags: input.tags,
                tools: input.tools,
                status: "active".to_string(),
                created_at: now,
                updated_at: now,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
            };
            agents.insert(id, row.clone());
            Ok((row, true))
        }
    }

    /// Get agent public_id from internal UUID
    pub async fn get_agent_public_id(&self, org_id: i64, id: AgentId) -> Result<Option<String>> {
        Ok(self
            .agents
            .read()
            .get(&id)
            .filter(|a| a.org_id == org_id)
            .map(|a| a.public_id.clone()))
    }

    // ============================================
    // Sessions
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        let now = Self::now();
        let id = SessionId::new();
        let row = SessionRow {
            id,
            org_id: input.org_id,
            agent_id: input.agent_id,
            title: input.title,
            tags: input.tags,
            model_id: input.model_id,
            capabilities: input.capabilities,
            tools: input.tools,
            status: "pending".to_string(), // Default status for new sessions
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
        };
        self.sessions.write().insert(id, row.clone());
        Ok(row)
    }

    /// Get session, validating org ownership directly
    pub async fn get_session(&self, org_id: i64, id: SessionId) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        if let Some(session) = sessions.get(&id) {
            // Validate that the session belongs to the org
            if session.org_id == org_id {
                return Ok(Some(session.clone()));
            }
        }
        Ok(None)
    }

    /// List sessions for an agent with pagination, validating org ownership.
    /// Returns (sessions, total_count).
    pub async fn list_sessions(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        // If agent_id is provided, validate it belongs to the org
        if let Some(aid) = agent_id {
            let agents = self.agents.read();
            if !agents
                .get(&aid)
                .map(|a| a.org_id == org_id)
                .unwrap_or(false)
            {
                return Ok((vec![], 0));
            }
        }

        let sessions = self.sessions.read();
        let mut result: Vec<_> = sessions
            .values()
            .filter(|s| {
                // Filter by org_id
                s.org_id == org_id
                    // Optionally filter by agent_id
                    && agent_id.is_none_or(|aid| s.agent_id == aid)
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let total = result.len() as u32;
        let offset = pagination.offset as usize;
        let limit = pagination.limit as usize;

        // Apply pagination
        let paginated = result.into_iter().skip(offset).take(limit).collect();

        Ok((paginated, total))
    }

    /// Update session, validating org ownership directly
    pub async fn update_session(
        &self,
        org_id: i64,
        id: SessionId,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        // First validate org ownership
        {
            let sessions = self.sessions.read();
            if let Some(session) = sessions.get(&id) {
                if session.org_id != org_id {
                    return Ok(None);
                }
            } else {
                return Ok(None);
            }
        }

        let mut sessions = self.sessions.write();
        if let Some(session) = sessions.get_mut(&id) {
            if let Some(title) = input.title {
                session.title = Some(title);
            }
            if let Some(tags) = input.tags {
                session.tags = tags;
            }
            if let Some(status) = input.status {
                session.status = status;
            }
            if let Some(started_at) = input.started_at {
                session.started_at = Some(started_at);
            }
            if input.finished_at.is_some() {
                session.finished_at = input.finished_at;
            }
            // Update updated_at on every update (mimics DB trigger)
            session.updated_at = Self::now();
            return Ok(Some(session.clone()));
        }
        Ok(None)
    }

    /// Delete session, validating org ownership directly
    pub async fn delete_session(&self, org_id: i64, id: SessionId) -> Result<bool> {
        // First validate org ownership
        {
            let sessions = self.sessions.read();
            if let Some(session) = sessions.get(&id) {
                if session.org_id != org_id {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }

        // Delete events first
        {
            let mut events = self.events.write();
            let to_remove: Vec<EventId> = events
                .iter()
                .filter(|(_, e)| e.session_id == id)
                .map(|(eid, _)| *eid)
                .collect();
            for eid in to_remove {
                events.remove(&eid);
            }
        }
        // Delete session files
        {
            let mut files = self.session_files.write();
            let to_remove: Vec<Uuid> = files
                .iter()
                .filter(|(_, f)| f.session_id == id)
                .map(|(fid, _)| *fid)
                .collect();
            for fid in to_remove {
                files.remove(&fid);
            }
        }
        Ok(self.sessions.write().remove(&id).is_some())
    }

    // ============================================
    // Events
    // ============================================

    pub async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        let now = Self::now();
        let id = EventId::new();

        // Get next sequence for this session
        let sequence = {
            let mut sequences = self.event_sequences.write();
            let seq = sequences.entry(input.session_id).or_insert(0);
            *seq += 1;
            *seq
        };

        let row = EventRow {
            id,
            session_id: input.session_id,
            sequence,
            event_type: input.event_type,
            ts: input.ts,
            context: input.context,
            data: input.data,
            metadata: input.metadata,
            tags: input.tags,
            created_at: now,
        };
        self.events.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn list_events(
        &self,
        session_id: SessionId,
        since_sequence: Option<i32>,
        since_id: Option<EventId>,
        exclude_types: &[String],
    ) -> Result<Vec<EventRow>> {
        let events = self.events.read();
        let mut result: Vec<_> = events
            .values()
            .filter(|e| {
                if e.session_id != session_id {
                    return false;
                }
                // Filter out excluded event types
                if !exclude_types.is_empty() && exclude_types.contains(&e.event_type) {
                    return false;
                }
                // Prefer since_id (UUID v7 monotonically increasing) over sequence
                if let Some(id) = since_id {
                    // Compare UUIDs directly for monotonic ordering
                    if e.id.uuid() <= id.uuid() {
                        return false;
                    }
                } else if let Some(seq) = since_sequence
                    && e.sequence <= seq
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();

        result.sort_by_key(|e| e.sequence);
        Ok(result)
    }

    pub async fn list_message_events(&self, session_id: SessionId) -> Result<Vec<EventRow>> {
        let message_types = [
            "input.message",
            "output.message.completed",
            "tool.completed",
        ];
        let events = self.events.read();
        let mut result: Vec<_> = events
            .values()
            .filter(|e| {
                e.session_id == session_id && message_types.contains(&e.event_type.as_str())
            })
            .cloned()
            .collect();
        result.sort_by_key(|e| e.sequence);
        Ok(result)
    }

    /// List message events for a session with filters applied.
    ///
    /// This method applies filters in-memory, mirroring the behavior of the
    /// PostgreSQL implementation but using Rust predicates instead of SQL.
    ///
    /// Note: Injections are NOT applied here - they should be applied at the
    /// MessageRetriever layer after converting events to messages.
    pub async fn list_message_events_filtered(
        &self,
        query: &MessageQuery,
    ) -> Result<Vec<EventRow>> {
        // Default event types if not specified
        let default_types = vec![
            "input.message".to_string(),
            "output.message.completed".to_string(),
            "tool.completed".to_string(),
        ];

        // Check for EventTypes filter, else use defaults
        let event_types = query
            .filters
            .iter()
            .find_map(|f| match f {
                MessageFilter::EventTypes(types) => Some(types.clone()),
                _ => None,
            })
            .unwrap_or(default_types);

        let events = self.events.read();
        let mut result: Vec<_> = events
            .values()
            .filter(|e| {
                // Session filter
                if e.session_id != query.session_id {
                    return false;
                }

                // Event type filter
                if !event_types.iter().any(|t| t == &e.event_type) {
                    return false;
                }

                // Apply all filters
                for filter in &query.filters {
                    match filter {
                        MessageFilter::EventTypes(_) => {
                            // Already handled above
                        }
                        MessageFilter::TimeRange { from, to } => {
                            if let Some(f) = from
                                && e.created_at < *f
                            {
                                return false;
                            }
                            if let Some(t) = to
                                && e.created_at > *t
                            {
                                return false;
                            }
                        }
                        MessageFilter::ToolName(name) => {
                            if e.event_type != "tool.completed" {
                                return false;
                            }
                            let tool_match = e
                                .data
                                .get("tool_name")
                                .and_then(|v| v.as_str())
                                .map(|n| n == name)
                                .unwrap_or(false);
                            if !tool_match {
                                return false;
                            }
                        }
                        MessageFilter::Search(search_query) => {
                            let data_str = e.data.to_string().to_lowercase();
                            if !data_str.contains(&search_query.to_lowercase()) {
                                return false;
                            }
                        }
                        MessageFilter::ExcludeIds(ids) => {
                            if ids.contains(&e.id) {
                                return false;
                            }
                        }
                        MessageFilter::IncludeIds(ids) => {
                            if !ids.contains(&e.id) {
                                return false;
                            }
                        }
                        MessageFilter::Custom(_) => {
                            // Custom filters are applied at the Message level,
                            // not the EventRow level. Skip here.
                        }
                    }
                }

                true
            })
            .cloned()
            .collect();

        result.sort_by_key(|e| e.sequence);

        // Apply offset and limit
        if let Some(offset) = query.offset {
            result = result.into_iter().skip(offset as usize).collect();
        }
        if let Some(limit) = query.limit {
            result.truncate(limit as usize);
        }

        Ok(result)
    }

    /// Get preview text for multiple sessions (in-memory implementation)
    pub async fn get_session_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        let mut previews = std::collections::HashMap::new();
        let events = self.events.read();

        for &session_id in session_ids {
            // Find the first user message for this session
            let first_user_msg = events
                .values()
                .filter(|e| e.session_id == session_id && e.event_type == "input.message")
                .min_by_key(|e| e.sequence);

            if let Some(event) = first_user_msg {
                // Extract text from the message data
                if let Some(text) = event
                    .data
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.get(0))
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                {
                    // Truncate to 200 chars
                    let preview: String = text.chars().take(200).collect();
                    previews.insert(session_id, preview);
                }
            }
        }

        Ok(previews)
    }

    /// Get output preview text for multiple sessions (in-memory implementation)
    pub async fn get_session_output_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        let mut previews = std::collections::HashMap::new();
        let events = self.events.read();

        for &session_id in session_ids {
            // Find the last agent message for this session
            let last_agent_msg = events
                .values()
                .filter(|e| {
                    e.session_id == session_id && e.event_type == "output.message.completed"
                })
                .max_by_key(|e| e.sequence);

            if let Some(event) = last_agent_msg {
                // Extract text from the message data
                if let Some(text) = event
                    .data
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.get(0))
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                {
                    // Truncate to 200 chars
                    let preview: String = text.chars().take(200).collect();
                    previews.insert(session_id, preview);
                }
            }
        }

        Ok(previews)
    }

    // ============================================
    // LLM Providers
    // ============================================

    pub async fn create_llm_provider(
        &self,
        org_id: i64,
        input: CreateLlmProviderRow,
    ) -> Result<LlmProviderRow> {
        let now = Self::now();
        let id = ProviderId::new();
        let api_key_set = input.api_key_encrypted.is_some();
        let row = LlmProviderRow {
            id,
            org_id,
            name: input.name,
            provider_type: input.provider_type,
            base_url: input.base_url,
            api_key_encrypted: input.api_key_encrypted,
            api_key_set,
            status: "active".to_string(), // Default status for new providers
            settings: input.settings.unwrap_or(serde_json::json!({})),
            last_synced_at: None,
            created_at: now,
            updated_at: now,
        };
        self.llm_providers.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create a provider with a specific ID (for seeding)
    /// Returns None if provider already exists (idempotent)
    pub async fn create_llm_provider_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmProviderRow,
    ) -> Result<Option<LlmProviderRow>> {
        let id = ProviderId::from_uuid(id);
        let mut providers = self.llm_providers.write();
        if providers.contains_key(&id) {
            return Ok(None); // Already exists
        }
        let now = Self::now();
        let api_key_set = input.api_key_encrypted.is_some();
        let row = LlmProviderRow {
            id,
            org_id,
            name: input.name,
            provider_type: input.provider_type,
            base_url: input.base_url,
            api_key_encrypted: input.api_key_encrypted,
            api_key_set,
            status: "active".to_string(),
            settings: input.settings.unwrap_or(serde_json::json!({})),
            last_synced_at: None,
            created_at: now,
            updated_at: now,
        };
        providers.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_llm_provider(&self, id: Uuid) -> Result<Option<LlmProviderRow>> {
        Ok(self
            .llm_providers
            .read()
            .get(&ProviderId::from_uuid(id))
            .cloned())
    }

    pub async fn list_llm_providers(&self) -> Result<Vec<LlmProviderRow>> {
        let providers = self.llm_providers.read();
        let mut result: Vec<_> = providers.values().cloned().collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    pub async fn update_llm_provider(
        &self,
        id: Uuid,
        input: UpdateLlmProvider,
    ) -> Result<Option<LlmProviderRow>> {
        let id = ProviderId::from_uuid(id);
        let mut providers = self.llm_providers.write();
        if let Some(provider) = providers.get_mut(&id) {
            if let Some(name) = input.name {
                provider.name = name;
            }
            if let Some(base_url) = input.base_url {
                provider.base_url = Some(base_url);
            }
            if let Some(api_key_encrypted) = input.api_key_encrypted {
                provider.api_key_encrypted = Some(api_key_encrypted);
                provider.api_key_set = true;
            }
            if let Some(status) = input.status {
                provider.status = status;
            }
            if let Some(settings) = input.settings {
                provider.settings = settings;
            }
            provider.updated_at = Self::now();
            return Ok(Some(provider.clone()));
        }
        Ok(None)
    }

    pub async fn delete_llm_provider(&self, id: Uuid) -> Result<bool> {
        let id = ProviderId::from_uuid(id);
        // Delete models first
        {
            let mut models = self.llm_models.write();
            let to_remove: Vec<ModelId> = models
                .iter()
                .filter(|(_, m)| m.provider_id == id)
                .map(|(mid, _)| *mid)
                .collect();
            for mid in to_remove {
                models.remove(&mid);
            }
        }
        Ok(self.llm_providers.write().remove(&id).is_some())
    }

    /// Update provider's last_synced_at timestamp
    pub async fn update_provider_last_synced(
        &self,
        id: Uuid,
        last_synced_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let id = ProviderId::from_uuid(id);
        if let Some(provider) = self.llm_providers.write().get_mut(&id) {
            provider.last_synced_at = Some(last_synced_at);
            provider.updated_at = Self::now();
        }
        Ok(())
    }

    /// Get a provider with its decrypted API key
    pub fn get_provider_with_api_key(
        &self,
        provider: &LlmProviderRow,
        encryption: &super::EncryptionService,
    ) -> Result<LlmProviderWithApiKey> {
        let api_key = if let Some(ref encrypted) = provider.api_key_encrypted {
            Some(encryption.decrypt_to_string(encrypted)?)
        } else {
            None
        };

        // Convert settings from sqlx JsonValue to serde_json::Value
        let settings: serde_json::Value =
            serde_json::from_str(&provider.settings.to_string()).unwrap_or_default();

        Ok(LlmProviderWithApiKey {
            id: provider.id,
            name: provider.name.clone(),
            provider_type: provider.provider_type.clone(),
            base_url: provider.base_url.clone(),
            api_key,
            settings,
        })
    }

    // ============================================
    // LLM Models
    // ============================================

    pub async fn get_default_llm_model(
        &self,
        org_id: i64,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        for model in models.values() {
            if model.is_default
                && model.org_id == org_id
                && let Some(provider) = providers.get(&model.provider_id)
            {
                return Ok(Some(LlmModelWithProviderRow {
                    id: model.id,
                    org_id: model.org_id,
                    provider_id: model.provider_id,
                    model_id: model.model_id.clone(),
                    display_name: model.display_name.clone(),
                    capabilities: model.capabilities.clone(),
                    is_default: model.is_default,
                    is_favorite: model.is_favorite,
                    status: model.status.clone(),
                    source: model.source.clone(),
                    last_seen_at: model.last_seen_at,
                    provider_metadata: model.provider_metadata.clone(),
                    created_at: model.created_at,
                    updated_at: model.updated_at,
                    provider_name: provider.name.clone(),
                    provider_type: provider.provider_type.clone(),
                }));
            }
        }
        Ok(None)
    }

    pub async fn clear_all_model_defaults(&self, org_id: i64) -> Result<()> {
        for model in self.llm_models.write().values_mut() {
            if model.org_id == org_id {
                model.is_default = false;
            }
        }
        Ok(())
    }

    pub async fn create_llm_model(
        &self,
        org_id: i64,
        input: CreateLlmModelRow,
    ) -> Result<LlmModelRow> {
        let now = Self::now();
        let id = ModelId::new();
        let row = LlmModelRow {
            id,
            org_id,
            provider_id: input.provider_id,
            model_id: input.model_id,
            display_name: input.display_name,
            capabilities: serde_json::to_value(&input.capabilities)?,
            is_default: input.is_default,
            is_favorite: input.is_favorite,
            status: "active".to_string(), // Default status for new models
            source: input.source,
            last_seen_at: None,
            provider_metadata: input.provider_metadata,
            created_at: now,
            updated_at: now,
        };
        self.llm_models.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create or update a model with a specific ID (for seeding)
    /// Uses upsert to update display_name, is_default, is_favorite if model exists
    pub async fn create_llm_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmModelRow,
    ) -> Result<Option<LlmModelRow>> {
        let id = ModelId::from_uuid(id);
        let mut models = self.llm_models.write();
        let now = Self::now();

        let row = if let Some(existing) = models.get(&id) {
            // Update existing model
            LlmModelRow {
                id,
                org_id: existing.org_id,
                provider_id: existing.provider_id,
                model_id: existing.model_id.clone(),
                display_name: input.display_name,
                capabilities: existing.capabilities.clone(),
                is_default: input.is_default,
                is_favorite: input.is_favorite,
                status: existing.status.clone(),
                source: existing.source.clone(),
                last_seen_at: existing.last_seen_at,
                provider_metadata: existing.provider_metadata.clone(),
                created_at: existing.created_at,
                updated_at: now,
            }
        } else {
            // Create new model
            LlmModelRow {
                id,
                org_id,
                provider_id: input.provider_id,
                model_id: input.model_id,
                display_name: input.display_name,
                capabilities: serde_json::to_value(&input.capabilities)?,
                is_default: input.is_default,
                is_favorite: input.is_favorite,
                status: "active".to_string(),
                source: input.source,
                last_seen_at: None,
                provider_metadata: input.provider_metadata,
                created_at: now,
                updated_at: now,
            }
        };

        models.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_llm_model(&self, id: Uuid) -> Result<Option<LlmModelRow>> {
        let id = ModelId::from_uuid(id);
        Ok(self.llm_models.read().get(&id).cloned())
    }

    pub async fn get_llm_model_with_provider(
        &self,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let id = ModelId::from_uuid(id);
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        if let Some(model) = models.get(&id)
            && let Some(provider) = providers.get(&model.provider_id)
        {
            return Ok(Some(LlmModelWithProviderRow {
                id: model.id,
                org_id: model.org_id,
                provider_id: model.provider_id,
                model_id: model.model_id.clone(),
                display_name: model.display_name.clone(),
                capabilities: model.capabilities.clone(),
                is_default: model.is_default,
                is_favorite: model.is_favorite,
                status: model.status.clone(),
                source: model.source.clone(),
                last_seen_at: model.last_seen_at,
                provider_metadata: model.provider_metadata.clone(),
                created_at: model.created_at,
                updated_at: model.updated_at,
                provider_name: provider.name.clone(),
                provider_type: provider.provider_type.clone(),
            }));
        }
        Ok(None)
    }

    pub async fn list_llm_models_for_provider(
        &self,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModelRow>> {
        let provider_id = ProviderId::from_uuid(provider_id);
        let models = self.llm_models.read();
        let mut result: Vec<_> = models
            .values()
            .filter(|m| m.provider_id == provider_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        Ok(result)
    }

    pub async fn list_all_llm_models(&self, org_id: i64) -> Result<Vec<LlmModelWithProviderRow>> {
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        let mut result: Vec<_> = models
            .values()
            .filter(|model| model.org_id == org_id)
            .filter_map(|model| {
                providers
                    .get(&model.provider_id)
                    .map(|provider| LlmModelWithProviderRow {
                        id: model.id,
                        org_id: model.org_id,
                        provider_id: model.provider_id,
                        model_id: model.model_id.clone(),
                        display_name: model.display_name.clone(),
                        capabilities: model.capabilities.clone(),
                        is_default: model.is_default,
                        is_favorite: model.is_favorite,
                        status: model.status.clone(),
                        source: model.source.clone(),
                        last_seen_at: model.last_seen_at,
                        provider_metadata: model.provider_metadata.clone(),
                        created_at: model.created_at,
                        updated_at: model.updated_at,
                        provider_name: provider.name.clone(),
                        provider_type: provider.provider_type.clone(),
                    })
            })
            .collect();
        // Sort by favorite first, then by display_name
        result.sort_by(|a, b| {
            b.is_favorite
                .cmp(&a.is_favorite)
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        Ok(result)
    }

    pub async fn update_llm_model(
        &self,
        id: Uuid,
        input: UpdateLlmModel,
    ) -> Result<Option<LlmModelRow>> {
        let id = ModelId::from_uuid(id);
        let mut models = self.llm_models.write();
        if let Some(model) = models.get_mut(&id) {
            if let Some(display_name) = input.display_name {
                model.display_name = display_name;
            }
            if let Some(capabilities) = input.capabilities {
                model.capabilities = serde_json::to_value(&capabilities)?;
            }
            if let Some(is_default) = input.is_default {
                model.is_default = is_default;
            }
            if let Some(is_favorite) = input.is_favorite {
                model.is_favorite = is_favorite;
            }
            if let Some(status) = input.status {
                model.status = status;
            }
            model.updated_at = Self::now();
            return Ok(Some(model.clone()));
        }
        Ok(None)
    }

    pub async fn delete_llm_model(&self, id: Uuid) -> Result<bool> {
        let id = ModelId::from_uuid(id);
        Ok(self.llm_models.write().remove(&id).is_some())
    }

    pub async fn get_llm_model_by_model_id(
        &self,
        org_id: i64,
        model_id: &str,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        for model in models.values() {
            if model.model_id == model_id
                && model.org_id == org_id
                && let Some(provider) = providers.get(&model.provider_id)
            {
                return Ok(Some(LlmModelWithProviderRow {
                    id: model.id,
                    org_id: model.org_id,
                    provider_id: model.provider_id,
                    model_id: model.model_id.clone(),
                    display_name: model.display_name.clone(),
                    capabilities: model.capabilities.clone(),
                    is_default: model.is_default,
                    is_favorite: model.is_favorite,
                    status: model.status.clone(),
                    source: model.source.clone(),
                    last_seen_at: model.last_seen_at,
                    provider_metadata: model.provider_metadata.clone(),
                    created_at: model.created_at,
                    updated_at: model.updated_at,
                    provider_name: provider.name.clone(),
                    provider_type: provider.provider_type.clone(),
                }));
            }
        }
        Ok(None)
    }

    // ============================================
    // Agent Capabilities
    // ============================================

    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        let agent_id = AgentId::from_uuid(agent_id);
        let caps = self.agent_capabilities.read();
        let mut result: Vec<_> = caps
            .iter()
            .filter(|((aid, _), _)| *aid == agent_id)
            .map(|(_, c)| c.clone())
            .collect();
        result.sort_by_key(|c| c.position);
        Ok(result)
    }

    pub async fn set_agent_capabilities(
        &self,
        agent_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<AgentCapabilityRow>> {
        let agent_id = AgentId::from_uuid(agent_id);
        let now = Self::now();
        let mut caps = self.agent_capabilities.write();

        // Remove existing capabilities for this agent
        let to_remove: Vec<_> = caps
            .keys()
            .filter(|(aid, _)| *aid == agent_id)
            .cloned()
            .collect();
        for key in to_remove {
            caps.remove(&key);
        }

        // Add new capabilities
        let mut result = Vec::new();
        for (capability_id, position, config) in capabilities.into_iter() {
            let row = AgentCapabilityRow {
                id: Uuid::now_v7(),
                agent_id,
                capability_id: capability_id.clone(),
                position,
                config,
                created_at: now,
            };
            caps.insert((agent_id, capability_id), row.clone());
            result.push(row);
        }

        Ok(result)
    }

    pub async fn add_agent_capability(
        &self,
        input: CreateAgentCapabilityRow,
    ) -> Result<AgentCapabilityRow> {
        let now = Self::now();
        let mut caps = self.agent_capabilities.write();

        let row = AgentCapabilityRow {
            id: Uuid::now_v7(),
            agent_id: input.agent_id,
            capability_id: input.capability_id.clone(),
            position: input.position,
            config: input.config,
            created_at: now,
        };
        caps.insert((input.agent_id, input.capability_id), row.clone());
        Ok(row)
    }

    pub async fn remove_agent_capability(
        &self,
        agent_id: Uuid,
        capability_id: &str,
    ) -> Result<bool> {
        let agent_id = AgentId::from_uuid(agent_id);
        Ok(self
            .agent_capabilities
            .write()
            .remove(&(agent_id, capability_id.to_string()))
            .is_some())
    }

    // ============================================
    // Session Files
    // ============================================

    pub async fn create_session_file(&self, input: CreateSessionFileRow) -> Result<SessionFileRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let content_len = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);
        let row = SessionFileRow {
            id,
            session_id: input.session_id,
            path: input.path,
            content: input.content,
            is_directory: input.is_directory,
            is_readonly: input.is_readonly,
            size_bytes: content_len,
            created_at: now,
            updated_at: now,
        };
        self.session_files.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_session_file(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileRow>> {
        Ok(self
            .session_files
            .read()
            .values()
            .find(|f| f.session_id == session_id && f.path == path)
            .cloned())
    }

    pub async fn get_session_file_by_id(&self, id: Uuid) -> Result<Option<SessionFileRow>> {
        Ok(self.session_files.read().get(&id).cloned())
    }

    /// Convert SessionFileRow to SessionFileInfoRow (strips content)
    fn file_to_info(f: &SessionFileRow) -> SessionFileInfoRow {
        SessionFileInfoRow {
            id: f.id,
            session_id: f.session_id,
            path: f.path.clone(),
            is_directory: f.is_directory,
            is_readonly: f.is_readonly,
            size_bytes: f.size_bytes,
            created_at: f.created_at,
            updated_at: f.updated_at,
        }
    }

    pub async fn list_session_files(
        &self,
        session_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<SessionFileInfoRow>> {
        let files = self.session_files.read();
        let prefix = if parent_path == "/" {
            "/".to_string()
        } else {
            format!("{}/", parent_path.trim_end_matches('/'))
        };

        let mut result: Vec<_> = files
            .values()
            .filter(|f| {
                if f.session_id != session_id {
                    return false;
                }
                if parent_path == "/" {
                    // Root level: files directly under /
                    f.path.starts_with('/') && !f.path[1..].contains('/')
                } else {
                    // Under specific directory
                    f.path.starts_with(&prefix) && !f.path[prefix.len()..].contains('/')
                }
            })
            .map(Self::file_to_info)
            .collect();
        result.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(result)
    }

    pub async fn list_all_session_files(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileInfoRow>> {
        let files = self.session_files.read();
        let mut result: Vec<_> = files
            .values()
            .filter(|f| f.session_id == session_id)
            .map(Self::file_to_info)
            .collect();
        result.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(result)
    }

    pub async fn update_session_file(
        &self,
        session_id: Uuid,
        path: &str,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        let mut files = self.session_files.write();
        if let Some(file) = files
            .values_mut()
            .find(|f| f.session_id == session_id && f.path == path)
        {
            if let Some(content) = input.content {
                file.size_bytes = content.len() as i64;
                file.content = Some(content);
            }
            if let Some(is_readonly) = input.is_readonly {
                file.is_readonly = is_readonly;
            }
            file.updated_at = Self::now();
            return Ok(Some(file.clone()));
        }
        Ok(None)
    }

    pub async fn delete_session_file(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        let mut files = self.session_files.write();
        let to_remove: Option<Uuid> = files
            .iter()
            .find(|(_, f)| f.session_id == session_id && f.path == path)
            .map(|(id, _)| *id);

        if let Some(id) = to_remove {
            files.remove(&id);
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn delete_session_file_recursive(&self, session_id: Uuid, path: &str) -> Result<u64> {
        let session_id = SessionId::from_uuid(session_id);
        let mut files = self.session_files.write();
        let prefix = format!("{}/", path.trim_end_matches('/'));

        let to_remove: Vec<Uuid> = files
            .iter()
            .filter(|(_, f)| {
                f.session_id == session_id && (f.path == path || f.path.starts_with(&prefix))
            })
            .map(|(id, _)| *id)
            .collect();

        let count = to_remove.len() as u64;
        for id in to_remove {
            files.remove(&id);
        }
        Ok(count)
    }

    pub async fn move_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        let session_id = SessionId::from_uuid(session_id);
        // Check if destination exists
        {
            let files = self.session_files.read();
            if files
                .values()
                .any(|f| f.session_id == session_id && f.path == dest_path)
            {
                return Err(anyhow!("Destination path already exists"));
            }
        }

        let mut files = self.session_files.write();
        if let Some(file) = files
            .values_mut()
            .find(|f| f.session_id == session_id && f.path == source_path)
        {
            file.path = dest_path.to_string();
            file.updated_at = Self::now();
            return Ok(Some(file.clone()));
        }
        Ok(None)
    }

    pub async fn copy_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        let session_id = SessionId::from_uuid(session_id);
        // Check if destination exists
        {
            let files = self.session_files.read();
            if files
                .values()
                .any(|f| f.session_id == session_id && f.path == dest_path)
            {
                return Err(anyhow!("Destination path already exists"));
            }
        }

        let source = {
            let files = self.session_files.read();
            files
                .values()
                .find(|f| f.session_id == session_id && f.path == source_path)
                .cloned()
        };

        if let Some(source) = source {
            let now = Self::now();
            let id = Uuid::now_v7();
            let new_file = SessionFileRow {
                id,
                session_id,
                path: dest_path.to_string(),
                content: source.content,
                is_directory: source.is_directory,
                is_readonly: source.is_readonly,
                size_bytes: source.size_bytes,
                created_at: now,
                updated_at: now,
            };
            self.session_files.write().insert(id, new_file.clone());
            return Ok(Some(new_file));
        }
        Ok(None)
    }

    pub async fn grep_session_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_prefix: Option<&str>,
    ) -> Result<Vec<SessionFileInfoRow>> {
        let session_id = SessionId::from_uuid(session_id);
        let regex = regex::Regex::new(pattern)?;
        let files = self.session_files.read();

        let result: Vec<_> = files
            .values()
            .filter(|f| {
                if f.session_id != session_id || f.is_directory {
                    return false;
                }
                if let Some(prefix) = path_prefix
                    && !f.path.starts_with(prefix)
                {
                    return false;
                }
                // Content is Vec<u8>, convert to str for regex matching
                f.content
                    .as_ref()
                    .and_then(|c| std::str::from_utf8(c).ok())
                    .map(|s| regex.is_match(s))
                    .unwrap_or(false)
            })
            .map(Self::file_to_info)
            .collect();

        Ok(result)
    }

    pub async fn session_file_exists(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .session_files
            .read()
            .values()
            .any(|f| f.session_id == session_id && f.path == path))
    }

    pub async fn session_directory_has_children(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        let prefix = format!("{}/", path.trim_end_matches('/'));
        Ok(self
            .session_files
            .read()
            .values()
            .any(|f| f.session_id == session_id && f.path.starts_with(&prefix)))
    }

    // ============================================
    // MCP Servers
    // ============================================

    pub async fn create_mcp_server(
        &self,
        org_id: i64,
        input: CreateMcpServerRow,
    ) -> Result<McpServerRow> {
        // Check for duplicate name within org
        if self
            .mcp_servers
            .read()
            .values()
            .any(|s| s.name == input.name && s.org_id == org_id)
        {
            return Err(anyhow!(
                "MCP server with name '{}' already exists",
                input.name
            ));
        }

        let now = Self::now();
        let id = McpServerId::new();
        let api_key_set = input.api_key_encrypted.is_some();

        let row = McpServerRow {
            id,
            org_id,
            name: input.name,
            description: input.description,
            url: input.url,
            transport_type: input.transport_type,
            status: "active".to_string(),
            api_key_encrypted: input.api_key_encrypted,
            api_key_set,
            headers: input.headers.unwrap_or(serde_json::json!({})),
            settings: input.settings.unwrap_or(serde_json::json!({})),
            cached_tools: serde_json::json!([]),
            tools_cached_at: None,
            created_at: now,
            updated_at: now,
        };

        self.mcp_servers.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create MCP server with a specific ID (for seeding)
    /// Returns None if server already exists with this ID
    pub async fn create_mcp_server_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateMcpServerRow,
    ) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        // Check if already exists
        if self.mcp_servers.read().contains_key(&id) {
            return Ok(None);
        }

        let now = Self::now();
        let api_key_set = input.api_key_encrypted.is_some();

        let row = McpServerRow {
            id,
            org_id,
            name: input.name,
            description: input.description,
            url: input.url,
            transport_type: input.transport_type,
            status: "active".to_string(),
            api_key_encrypted: input.api_key_encrypted,
            api_key_set,
            headers: input.headers.unwrap_or(serde_json::json!({})),
            settings: input.settings.unwrap_or(serde_json::json!({})),
            cached_tools: serde_json::json!([]),
            tools_cached_at: None,
            created_at: now,
            updated_at: now,
        };

        self.mcp_servers.write().insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_mcp_server(&self, id: Uuid) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        Ok(self.mcp_servers.read().get(&id).cloned())
    }

    pub async fn get_mcp_server_by_name(&self, name: &str) -> Result<Option<McpServerRow>> {
        Ok(self
            .mcp_servers
            .read()
            .values()
            .find(|s| s.name == name)
            .cloned())
    }

    pub async fn list_mcp_servers(&self) -> Result<Vec<McpServerRow>> {
        let mut servers: Vec<_> = self.mcp_servers.read().values().cloned().collect();
        servers.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(servers)
    }

    pub async fn list_active_mcp_servers(&self) -> Result<Vec<McpServerRow>> {
        let mut servers: Vec<_> = self
            .mcp_servers
            .read()
            .values()
            .filter(|s| s.status == "active")
            .cloned()
            .collect();
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(servers)
    }

    pub async fn update_mcp_server(
        &self,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if let Some(name) = input.name {
                server.name = name;
            }
            if let Some(description) = input.description {
                server.description = Some(description);
            }
            if let Some(url) = input.url {
                server.url = url;
            }
            if let Some(transport_type) = input.transport_type {
                server.transport_type = transport_type;
            }
            if let Some(status) = input.status {
                server.status = status;
            }
            if let Some(api_key_encrypted) = input.api_key_encrypted {
                server.api_key_encrypted = Some(api_key_encrypted);
                server.api_key_set = true;
            }
            if let Some(headers) = input.headers {
                server.headers = headers;
            }
            if let Some(settings) = input.settings {
                server.settings = settings;
            }
            server.updated_at = Self::now();
            return Ok(Some(server.clone()));
        }
        Ok(None)
    }

    pub async fn delete_mcp_server(&self, id: Uuid) -> Result<bool> {
        Ok(self.mcp_servers.write().remove(&id).is_some())
    }

    // ============================================
    // LLM Generations (Usage Tracking)
    // ============================================
    //
    // In-memory implementations for dev mode.
    // Note: llm_generations table is not stored in memory since it's only
    // used for analytics. We just update the denormalized totals.

    #[allow(clippy::too_many_arguments)]
    pub async fn create_llm_generation(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _turn_id: Option<Uuid>,
        _event_id: Option<Uuid>,
        _model: String,
        _provider: Option<String>,
        _input_tokens: i64,
        _output_tokens: i64,
        _cache_read_tokens: i64,
        _cache_creation_tokens: i64,
        _duration_ms: Option<i32>,
        _finish_reason: Option<String>,
        _created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        // In dev mode, we don't store individual generations
        // Usage totals are updated via increment_session_usage/increment_agent_usage
        Ok(())
    }

    pub async fn increment_session_usage(
        &self,
        session_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> Result<()> {
        let mut sessions = self.sessions.write();
        if let Some(session) = sessions.get_mut(&session_id) {
            session.total_input_tokens += input_tokens;
            session.total_output_tokens += output_tokens;
            session.total_cache_read_tokens += cache_read_tokens;
            session.total_cache_creation_tokens += cache_creation_tokens;
            // Update updated_at on every update (mimics DB trigger)
            session.updated_at = Self::now();
        }
        Ok(())
    }

    pub async fn increment_agent_usage(
        &self,
        agent_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> Result<()> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&agent_id) {
            agent.total_input_tokens += input_tokens;
            agent.total_output_tokens += output_tokens;
            agent.total_cache_read_tokens += cache_read_tokens;
            agent.total_cache_creation_tokens += cache_creation_tokens;
        }
        Ok(())
    }

    // ============================================
    // Images
    // ============================================

    pub async fn create_image(&self, input: CreateImageRow) -> Result<ImageRow> {
        let now = Self::now();
        let id = ImageId::new();
        let row = ImageRow {
            id,
            filename: input.filename,
            content_type: input.content_type,
            size_bytes: input.size_bytes,
            data: input.data,
            thumbnail_data: input.thumbnail_data,
            thumbnail_content_type: input.thumbnail_content_type,
            metadata: input.metadata,
            created_at: now,
        };
        self.images.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_image(&self, id: Uuid) -> Result<Option<ImageRow>> {
        let id = ImageId::from_uuid(id);
        Ok(self.images.read().get(&id).cloned())
    }

    pub async fn get_image_info(&self, id: Uuid) -> Result<Option<ImageInfoRow>> {
        let id = ImageId::from_uuid(id);
        Ok(self.images.read().get(&id).map(|img| ImageInfoRow {
            id: img.id,
            filename: img.filename.clone(),
            content_type: img.content_type.clone(),
            size_bytes: img.size_bytes,
            metadata: img.metadata.clone(),
            created_at: img.created_at,
        }))
    }

    pub async fn delete_image(&self, id: Uuid) -> Result<bool> {
        let id = ImageId::from_uuid(id);
        Ok(self.images.write().remove(&id).is_some())
    }

    pub async fn list_images(&self, limit: i64, offset: i64) -> Result<Vec<ImageInfoRow>> {
        let images = self.images.read();
        let mut result: Vec<_> = images
            .values()
            .map(|img| ImageInfoRow {
                id: img.id,
                filename: img.filename.clone(),
                content_type: img.content_type.clone(),
                size_bytes: img.size_bytes,
                metadata: img.metadata.clone(),
                created_at: img.created_at,
            })
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let result = result
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        Ok(result)
    }

    pub async fn update_mcp_server_tools(
        &self,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            server.cached_tools = input.cached_tools;
            server.tools_cached_at = Some(Self::now());
            server.updated_at = Self::now();
            return Ok(Some(server.clone()));
        }
        Ok(None)
    }

    // ============================================
    // Organizations
    // ============================================

    pub async fn create_organization(
        &self,
        input: CreateOrganizationRow,
    ) -> Result<OrganizationRow> {
        let now = Self::now();
        let mut orgs = self.organizations.write();
        let org_id = orgs.keys().max().unwrap_or(&0) + 1;
        let row = OrganizationRow {
            org_id,
            public_id: input.public_id,
            name: input.name,
            created_at: now,
            updated_at: now,
        };
        orgs.insert(org_id, row.clone());
        Ok(row)
    }

    /// Create organization with specific org_id (for seeding).
    /// Returns None if org_id already exists.
    pub async fn create_organization_with_id(
        &self,
        org_id: i64,
        input: CreateOrganizationRow,
    ) -> Result<Option<OrganizationRow>> {
        let now = Self::now();
        let mut orgs = self.organizations.write();
        if orgs.contains_key(&org_id) {
            return Ok(None);
        }
        let row = OrganizationRow {
            org_id,
            public_id: input.public_id,
            name: input.name,
            created_at: now,
            updated_at: now,
        };
        orgs.insert(org_id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_organization(&self, org_id: i64) -> Result<Option<OrganizationRow>> {
        Ok(self.organizations.read().get(&org_id).cloned())
    }

    pub async fn get_organization_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        Ok(self
            .organizations
            .read()
            .values()
            .find(|o| o.public_id == public_id)
            .cloned())
    }

    pub async fn list_organizations(&self) -> Result<Vec<OrganizationRow>> {
        let orgs = self.organizations.read();
        let mut result: Vec<_> = orgs.values().cloned().collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn update_organization(
        &self,
        org_id: i64,
        input: UpdateOrganization,
    ) -> Result<Option<OrganizationRow>> {
        let mut orgs = self.organizations.write();
        if let Some(org) = orgs.get_mut(&org_id) {
            if let Some(name) = input.name {
                org.name = name;
            }
            org.updated_at = Self::now();
            return Ok(Some(org.clone()));
        }
        Ok(None)
    }

    pub async fn delete_organization(&self, org_id: i64) -> Result<bool> {
        Ok(self.organizations.write().remove(&org_id).is_some())
    }

    // ============================================
    // Organization Members
    // ============================================

    pub async fn add_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<OrganizationMemberRow> {
        let now = Self::now();
        let row = OrganizationMemberRow {
            org_id,
            user_id,
            created_at: now,
        };
        self.organization_members
            .write()
            .insert((org_id, user_id), row.clone());
        Ok(row)
    }

    pub async fn remove_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        Ok(self
            .organization_members
            .write()
            .remove(&(org_id, user_id))
            .is_some())
    }

    pub async fn list_organization_members(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberRow>> {
        let members = self.organization_members.read();
        let mut result: Vec<_> = members
            .values()
            .filter(|m| m.org_id == org_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn list_user_organizations(&self, user_id: Uuid) -> Result<Vec<OrganizationRow>> {
        let members = self.organization_members.read();
        let orgs = self.organizations.read();
        let org_ids: Vec<i64> = members
            .values()
            .filter(|m| m.user_id == user_id)
            .map(|m| m.org_id)
            .collect();
        let mut result: Vec<_> = orgs
            .values()
            .filter(|o| org_ids.contains(&o.org_id))
            .cloned()
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    pub async fn is_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        Ok(self
            .organization_members
            .read()
            .contains_key(&(org_id, user_id)))
    }

    // ============================================
    // Session Storage (Key-Value & Secrets)
    // ============================================

    pub async fn list_session_keys(&self, session_id: Uuid) -> Result<Vec<SessionKeyInfoRow>> {
        let session_id = SessionId::from_uuid(session_id);
        let storage = self.session_key_values.read();
        let mut keys: Vec<_> = storage
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|((_, _), row)| SessionKeyInfoRow {
                key: row.key.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();
        keys.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(keys)
    }

    pub async fn get_session_key_value(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<SessionKeyValueRow>> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .session_key_values
            .read()
            .get(&(session_id, key.to_string()))
            .cloned())
    }

    pub async fn list_session_secrets(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionSecretInfoRow>> {
        let session_id = SessionId::from_uuid(session_id);
        let storage = self.session_secrets.read();
        let mut secrets: Vec<_> = storage
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|((_, _), row)| SessionSecretInfoRow {
                name: row.name.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();
        secrets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(secrets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::DEFAULT_ORG_ID;

    #[tokio::test]
    async fn test_create_and_get_agent() {
        let db = InMemoryDatabase::new();

        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: AgentId::new().to_string(),
                    name: "Test Agent".to_string(),
                    description: Some("A test agent".to_string()),
                    system_prompt: "You are helpful".to_string(),
                    default_model_id: None,
                    tags: vec!["test".to_string()],
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        assert_eq!(agent.name, "Test Agent");

        let fetched = db.get_agent(DEFAULT_ORG_ID, agent.id).await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().name, "Test Agent");
    }

    #[tokio::test]
    async fn test_create_and_list_sessions() {
        let db = InMemoryDatabase::new();

        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: AgentId::new().to_string(),
                    name: "Test Agent".to_string(),
                    description: None,
                    system_prompt: String::new(),
                    default_model_id: None,
                    tags: vec![],
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                agent_id: agent.id,
                title: Some("Test Session".to_string()),
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
            })
            .await
            .unwrap();

        let pagination = crate::api::common::Pagination::new(0, 20);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), pagination)
            .await
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(total, 1);
        assert_eq!(sessions[0].id, session.id);
    }

    #[tokio::test]
    async fn test_session_updated_at() {
        let db = InMemoryDatabase::new();

        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: AgentId::new().to_string(),
                    name: "Test Agent".to_string(),
                    description: None,
                    system_prompt: String::new(),
                    default_model_id: None,
                    tags: vec![],
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        // Create session - updated_at should equal created_at
        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                agent_id: agent.id,
                title: Some("Test Session".to_string()),
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
            })
            .await
            .unwrap();

        assert_eq!(session.created_at, session.updated_at);
        let original_updated_at = session.updated_at;

        // Small delay to ensure different timestamp
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Update session - updated_at should change
        let updated = db
            .update_session(
                DEFAULT_ORG_ID,
                session.id,
                UpdateSession {
                    title: Some("Updated Title".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();

        assert!(updated.updated_at > original_updated_at);
        assert_eq!(updated.title, Some("Updated Title".to_string()));
    }

    #[tokio::test]
    async fn test_events_sequence() {
        use chrono::Utc;

        let db = InMemoryDatabase::new();

        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: AgentId::new().to_string(),
                    name: "Test Agent".to_string(),
                    description: None,
                    system_prompt: String::new(),
                    default_model_id: None,
                    tags: vec![],
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                agent_id: agent.id,
                title: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
            })
            .await
            .unwrap();

        // Create multiple events
        for i in 0..3 {
            db.create_event(CreateEventRow {
                session_id: session.id,
                event_type: "input.message".to_string(),
                ts: Utc::now(),
                context: serde_json::json!({}),
                data: serde_json::json!({"content": format!("Message {}", i)}),
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
        }

        let events = db.list_events(session.id, None, None, &[]).await.unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);
        assert_eq!(events[2].sequence, 3);
    }

    #[tokio::test]
    async fn test_sessions_pagination() {
        let db = InMemoryDatabase::new();

        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: AgentId::new().to_string(),
                    name: "Test Agent".to_string(),
                    description: None,
                    system_prompt: String::new(),
                    default_model_id: None,
                    tags: vec![],
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        // Create 15 sessions
        for i in 0..15 {
            db.create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                agent_id: agent.id,
                title: Some(format!("Session {}", i)),
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
            })
            .await
            .unwrap();
        }

        // Test default pagination (all sessions fit within limit)
        let pagination = crate::api::common::Pagination::new(0, 20);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), pagination)
            .await
            .unwrap();
        assert_eq!(total, 15);
        assert_eq!(sessions.len(), 15);

        // Test with limit=5
        let pagination = crate::api::common::Pagination::new(0, 5);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), pagination)
            .await
            .unwrap();
        assert_eq!(total, 15);
        assert_eq!(sessions.len(), 5);

        // Test with offset=5, limit=5
        let pagination = crate::api::common::Pagination::new(5, 5);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), pagination)
            .await
            .unwrap();
        assert_eq!(total, 15);
        assert_eq!(sessions.len(), 5);

        // Test last partial page (offset=10, limit=10 should return 5)
        let pagination = crate::api::common::Pagination::new(10, 10);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), pagination)
            .await
            .unwrap();
        assert_eq!(total, 15);
        assert_eq!(sessions.len(), 5);

        // Test beyond range (offset=20)
        let pagination = crate::api::common::Pagination::new(20, 10);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), pagination)
            .await
            .unwrap();
        assert_eq!(total, 15);
        assert_eq!(sessions.len(), 0);
    }

    #[tokio::test]
    async fn test_sessions_pagination_ordering() {
        let db = InMemoryDatabase::new();

        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: AgentId::new().to_string(),
                    name: "Test Agent".to_string(),
                    description: None,
                    system_prompt: String::new(),
                    default_model_id: None,
                    tags: vec![],
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        // Create sessions with sequential titles
        for i in 1..=5 {
            db.create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                agent_id: agent.id,
                title: Some(format!("Session {}", i)),
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
            })
            .await
            .unwrap();
            // Small delay to ensure different created_at timestamps
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // Sessions should be ordered by created_at DESC (newest first)
        let pagination = crate::api::common::Pagination::new(0, 10);
        let (sessions, _) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), pagination)
            .await
            .unwrap();

        assert_eq!(sessions.len(), 5);
        // Most recent session should be first
        assert_eq!(sessions[0].title, Some("Session 5".to_string()));
        assert_eq!(sessions[4].title, Some("Session 1".to_string()));
    }
}
