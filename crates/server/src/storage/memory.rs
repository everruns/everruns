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
    AgentId, AgentIdentityId, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, EventId, HarnessId, ImageId,
    LeasedResourceId, McpServerId, MessageId, ModelId, NotificationId, ProviderId, ScheduleId,
    SessionId, SkillId,
};
use parking_lot::RwLock;
use std::collections::HashMap;
use uuid::Uuid;

use super::models::*;

/// Max search tokens to prevent performance degradation from long inputs.
const MAX_SEARCH_TOKENS: usize = 8;

/// Multi-word tokenized search. Each whitespace-separated token must match
/// somewhere in the combined text (case-insensitive). Tokens beyond
/// [`MAX_SEARCH_TOKENS`] are ignored.
fn matches_search_tokens(search: Option<&str>, texts: &[&str]) -> bool {
    let Some(q) = search.filter(|q| !q.trim().is_empty()) else {
        return true;
    };
    let combined: String = texts
        .iter()
        .map(|t| t.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    q.trim()
        .to_lowercase()
        .split_whitespace()
        .take(MAX_SEARCH_TOKENS)
        .all(|token| combined.contains(token))
}

/// Stored data for a pinned session: (org_id, pinned_at)
type PinnedSessionData = (i64, DateTime<Utc>);

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
    cli_auth_sessions: RwLock<HashMap<Uuid, CliAuthSessionRow>>,
    refresh_tokens: RwLock<HashMap<Uuid, RefreshTokenRow>>,
    agents: RwLock<HashMap<AgentId, AgentRow>>,
    sessions: RwLock<HashMap<SessionId, SessionRow>>,
    events: RwLock<HashMap<EventId, EventRow>>,
    llm_providers: RwLock<HashMap<ProviderId, LlmProviderRow>>,
    llm_models: RwLock<HashMap<ModelId, LlmModelRow>>,
    agent_capabilities: RwLock<HashMap<(AgentId, String), AgentCapabilityRow>>,
    harnesses: RwLock<HashMap<HarnessId, HarnessRow>>,
    harness_capabilities: RwLock<HashMap<(HarnessId, String), HarnessCapabilityRow>>,
    session_files: RwLock<HashMap<Uuid, SessionFileRow>>,
    mcp_servers: RwLock<HashMap<McpServerId, McpServerRow>>,
    images: RwLock<HashMap<ImageId, ImageRow>>,
    skills: RwLock<HashMap<SkillId, SkillRow>>,
    skill_files: RwLock<Vec<SkillFileRow>>,
    // Event sequence counter per session
    event_sequences: RwLock<HashMap<SessionId, i32>>,
    // Session storage
    session_key_values: RwLock<HashMap<(SessionId, String), SessionKeyValueRow>>,
    session_secrets: RwLock<HashMap<(SessionId, String), SessionSecretRow>>,
    // User connections (external service accounts)
    user_connections: RwLock<HashMap<Uuid, UserConnectionRow>>,
    // Pinned sessions: (user_id, session_id) -> (org_id, pinned_at)
    pinned_sessions: RwLock<HashMap<(Uuid, SessionId), PinnedSessionData>>,
    // Durable UI notifications
    notifications: RwLock<HashMap<NotificationId, NotificationRow>>,
    notification_turn_requests: RwLock<HashMap<MessageId, NotificationTurnRequestRow>>,
    // Session schedules
    session_schedules: RwLock<HashMap<ScheduleId, SessionScheduleRow>>,
    // Generic leased resources that require eventual cleanup.
    leased_resources: RwLock<HashMap<LeasedResourceId, LeasedResourceRow>>,
    // Audit logs (TM-OBS-007)
    audit_logs: RwLock<Vec<AuditLogRow>>,
    // Apps (deployable agent+harness bundles)
    apps: RwLock<HashMap<Uuid, AppRow>>,
    // Agent identities (virtual principals)
    agent_identities: RwLock<HashMap<AgentIdentityId, AgentIdentityRow>>,
    // Organization settings (default model, etc.)
    org_settings: RwLock<HashMap<i64, OrganizationSettingsRow>>,
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
                external_id: None,
                created_by: None,
            },
        );

        Self {
            organizations: RwLock::new(organizations),
            organization_members: RwLock::new(HashMap::new()),
            users: RwLock::new(HashMap::new()),
            api_keys: RwLock::new(HashMap::new()),
            cli_auth_sessions: RwLock::new(HashMap::new()),
            refresh_tokens: RwLock::new(HashMap::new()),
            agents: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
            events: RwLock::new(HashMap::new()),
            llm_providers: RwLock::new(HashMap::new()),
            llm_models: RwLock::new(HashMap::new()),
            agent_capabilities: RwLock::new(HashMap::new()),
            harnesses: RwLock::new(HashMap::new()),
            harness_capabilities: RwLock::new(HashMap::new()),
            session_files: RwLock::new(HashMap::new()),
            mcp_servers: RwLock::new(HashMap::new()),
            images: RwLock::new(HashMap::new()),
            skills: RwLock::new(HashMap::new()),
            skill_files: RwLock::new(Vec::new()),
            event_sequences: RwLock::new(HashMap::new()),
            session_key_values: RwLock::new(HashMap::new()),
            session_secrets: RwLock::new(HashMap::new()),
            user_connections: RwLock::new(HashMap::new()),
            pinned_sessions: RwLock::new(HashMap::new()),
            notifications: RwLock::new(HashMap::new()),
            notification_turn_requests: RwLock::new(HashMap::new()),
            session_schedules: RwLock::new(HashMap::new()),
            leased_resources: RwLock::new(HashMap::new()),
            audit_logs: RwLock::new(Vec::new()),
            apps: RwLock::new(HashMap::new()),
            agent_identities: RwLock::new(HashMap::new()),
            org_settings: RwLock::new(HashMap::new()),
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
            external_id: input.external_id,
            created_at: now,
            updated_at: now,
        };
        self.users.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create user with a specific UUID (for seeding).
    /// Returns None if id already exists.
    pub async fn create_user_with_id(
        &self,
        id: Uuid,
        input: CreateUserRow,
    ) -> Result<Option<UserRow>> {
        let now = Self::now();
        let mut users = self.users.write();
        if users.contains_key(&id) {
            return Ok(None);
        }
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
            external_id: input.external_id,
            created_at: now,
            updated_at: now,
        };
        users.insert(id, row.clone());
        Ok(Some(row))
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

    /// List users within an organization (TM-TENANT-008: org-scoped user listing)
    pub async fn list_users_by_org(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<UserRow>> {
        let members = self.organization_members.read();
        let users = self.users.read();

        // Get user IDs that belong to this org
        let org_user_ids: std::collections::HashSet<_> = members
            .keys()
            .filter(|(oid, _)| *oid == org_id)
            .map(|(_, uid)| *uid)
            .collect();

        let mut result: Vec<_> = users
            .values()
            .filter(|u| org_user_ids.contains(&u.id))
            .filter(|u| match search {
                Some(query) if !query.trim().is_empty() => {
                    let pattern = query.trim().to_lowercase();
                    u.name.to_lowercase().contains(&pattern)
                        || u.email.to_lowercase().contains(&pattern)
                }
                _ => true,
            })
            .cloned()
            .collect();
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
            metadata: input.metadata,
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
    // CLI Auth Sessions
    // ============================================

    pub async fn create_cli_auth_session(
        &self,
        input: CreateCliAuthSessionRow,
    ) -> Result<CliAuthSessionRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = CliAuthSessionRow {
            id,
            state: input.state,
            exchange_code: input.exchange_code,
            user_id: None,
            redirect_port: input.redirect_port,
            completed: false,
            expires_at: input.expires_at,
            created_at: now,
        };
        self.cli_auth_sessions.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_cli_auth_session_by_state(
        &self,
        state: &str,
    ) -> Result<Option<CliAuthSessionRow>> {
        let now = Self::now();
        Ok(self
            .cli_auth_sessions
            .read()
            .values()
            .find(|s| s.state == state && s.expires_at > now)
            .cloned())
    }

    pub async fn get_cli_auth_session_by_exchange_code(
        &self,
        code: &str,
    ) -> Result<Option<CliAuthSessionRow>> {
        let now = Self::now();
        Ok(self
            .cli_auth_sessions
            .read()
            .values()
            .find(|s| s.exchange_code == code && s.expires_at > now)
            .cloned())
    }

    pub async fn complete_cli_auth_session(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        if let Some(session) = self.cli_auth_sessions.write().get_mut(&id)
            && !session.completed
        {
            session.user_id = Some(user_id);
            session.completed = true;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn delete_expired_cli_auth_sessions(&self) -> Result<u64> {
        let now = Self::now();
        let mut sessions = self.cli_auth_sessions.write();
        let before = sessions.len();
        sessions.retain(|_, s| s.expires_at > now);
        Ok((before - sessions.len()) as u64)
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
            initial_files: input.initial_files,
            tools: input.tools,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
        };
        self.agents.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create agent with a specific ID, idempotent (returns None if exists)
    /// Create or update agent with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_agent_with_id(
        &self,
        org_id: i64,
        id: AgentId,
        input: CreateAgentRow,
    ) -> Result<Option<AgentRow>> {
        let mut agents = self.agents.write();
        let now = Self::now();

        if let Some(existing) = agents.get(&id) {
            // Check if seed-controlled fields differ
            if existing.name == input.name
                && existing.description == input.description
                && existing.system_prompt == input.system_prompt
                && existing.tags == input.tags
                && existing.initial_files == input.initial_files
                && existing.tools == input.tools
            {
                return Ok(None); // Unchanged
            }
            // Update changed fields
            let row = AgentRow {
                name: input.name,
                description: input.description,
                system_prompt: input.system_prompt,
                tags: input.tags,
                initial_files: input.initial_files,
                tools: input.tools,
                updated_at: now,
                ..existing.clone()
            };
            agents.insert(id, row.clone());
            return Ok(Some(row));
        }

        let row = AgentRow {
            id,
            public_id: input.public_id,
            org_id,
            name: input.name,
            description: input.description,
            system_prompt: input.system_prompt,
            default_model_id: input.default_model_id,
            tags: input.tags,
            initial_files: input.initial_files,
            tools: input.tools,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
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

    pub async fn list_agents(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<AgentRow>, u32)> {
        let agents = self.agents.read();
        let mut result: Vec<_> = agents
            .values()
            .filter(|a| {
                a.org_id == org_id
                    && if include_archived {
                        a.status != "deleted"
                    } else {
                        a.status == "active"
                    }
            })
            .filter(|a| {
                matches_search_tokens(search, &[&a.name, a.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let total = result.len() as u32;
        let offset = pagination.offset as usize;
        let limit = pagination.limit as usize;
        let paginated = result.into_iter().skip(offset).take(limit).collect();

        Ok((paginated, total))
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
            if let Some(initial_files) = input.initial_files {
                agent.initial_files = initial_files;
            }
            if let Some(status) = input.status {
                agent.status = status;
            }
            if let Some(tools) = input.tools {
                agent.tools = tools;
            }
            agent.updated_at = Self::now();
            return Ok(Some(agent.clone()));
        }
        Ok(None)
    }

    pub async fn delete_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&id) {
            if agent.org_id != org_id || agent.status != "active" {
                return Ok(false);
            }
            agent.status = "archived".to_string();
            agent.archived_at = Some(Self::now());
            agent.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn destroy_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&id) {
            if agent.org_id != org_id || agent.status != "archived" {
                return Ok(false);
            }
            agent.status = "deleted".to_string();
            agent.deleted_at = Some(Self::now());
            agent.updated_at = Self::now();
            return Ok(true);
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
            agent.initial_files = input.initial_files;
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
                initial_files: input.initial_files,
                tools: input.tools,
                status: "active".to_string(),
                created_at: now,
                updated_at: now,
                archived_at: None,
                deleted_at: None,
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
    // Harnesses
    // ============================================

    pub async fn create_harness(&self, org_id: i64, input: CreateHarnessRow) -> Result<HarnessRow> {
        let now = Self::now();
        let id = HarnessId::new();
        let row = HarnessRow {
            id,
            org_id,
            name: input.name,
            description: input.description,
            system_prompt: input.system_prompt,
            parent_harness_id: input.parent_harness_id,
            default_model_id: input.default_model_id,
            tags: input.tags,
            initial_files: input.initial_files,
            is_built_in: input.is_built_in,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        self.harnesses.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create harness with a specific ID, idempotent (returns None if exists)
    /// Create or update harness with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_harness_with_id(
        &self,
        org_id: i64,
        id: HarnessId,
        input: CreateHarnessRow,
    ) -> Result<Option<HarnessRow>> {
        let mut harnesses = self.harnesses.write();
        let now = Self::now();

        if let Some(existing) = harnesses.get(&id) {
            if existing.name == input.name
                && existing.description == input.description
                && existing.system_prompt == input.system_prompt
                && existing.parent_harness_id == input.parent_harness_id
                && existing.tags == input.tags
                && existing.initial_files == input.initial_files
            {
                return Ok(None); // Unchanged
            }
            let row = HarnessRow {
                name: input.name,
                description: input.description,
                system_prompt: input.system_prompt,
                parent_harness_id: input.parent_harness_id,
                tags: input.tags,
                initial_files: input.initial_files,
                updated_at: now,
                ..existing.clone()
            };
            harnesses.insert(id, row.clone());
            return Ok(Some(row));
        }

        let row = HarnessRow {
            id,
            org_id,
            name: input.name,
            description: input.description,
            system_prompt: input.system_prompt,
            parent_harness_id: input.parent_harness_id,
            default_model_id: input.default_model_id,
            tags: input.tags,
            initial_files: input.initial_files,
            is_built_in: input.is_built_in,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        harnesses.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_harness(&self, org_id: i64, id: HarnessId) -> Result<Option<HarnessRow>> {
        Ok(self
            .harnesses
            .read()
            .get(&id)
            .filter(|h| h.org_id == org_id)
            .cloned())
    }

    pub async fn list_harnesses(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<HarnessRow>> {
        let harnesses = self.harnesses.read();
        let mut result: Vec<_> = harnesses
            .values()
            .filter(|h| {
                h.org_id == org_id
                    && if include_archived {
                        h.status != "deleted"
                    } else {
                        h.status == "active"
                    }
            })
            .filter(|h| {
                matches_search_tokens(search, &[&h.name, h.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn update_harness(
        &self,
        org_id: i64,
        id: HarnessId,
        input: UpdateHarness,
    ) -> Result<Option<HarnessRow>> {
        let mut harnesses = self.harnesses.write();
        if let Some(harness) = harnesses.get_mut(&id).filter(|h| h.org_id == org_id) {
            if let Some(name) = input.name {
                harness.name = name;
            }
            if let Some(description) = input.description {
                harness.description = Some(description);
            }
            if let Some(system_prompt) = input.system_prompt {
                harness.system_prompt = system_prompt;
            }
            if let Some(parent_harness_id) = input.parent_harness_id {
                harness.parent_harness_id = parent_harness_id;
            }
            if let Some(default_model_id) = input.default_model_id {
                harness.default_model_id = Some(default_model_id);
            }
            if let Some(tags) = input.tags {
                harness.tags = tags;
            }
            if let Some(initial_files) = input.initial_files {
                harness.initial_files = initial_files;
            }
            if let Some(status) = input.status {
                harness.status = status;
            }
            harness.updated_at = Self::now();
            return Ok(Some(harness.clone()));
        }
        Ok(None)
    }

    pub async fn delete_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        let mut harnesses = self.harnesses.write();
        if let Some(harness) = harnesses.get_mut(&id) {
            if harness.org_id != org_id || harness.status != "active" {
                return Ok(false);
            }
            harness.status = "archived".to_string();
            harness.archived_at = Some(Self::now());
            harness.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn list_child_harnesses(
        &self,
        org_id: i64,
        parent_id: HarnessId,
    ) -> Result<Vec<HarnessRow>> {
        let harnesses = self.harnesses.read();
        let mut result: Vec<_> = harnesses
            .values()
            .filter(|h| {
                h.org_id == org_id
                    && h.parent_harness_id == Some(parent_id)
                    && h.status != "deleted"
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn destroy_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        let mut harnesses = self.harnesses.write();
        if let Some(harness) = harnesses.get_mut(&id) {
            if harness.org_id != org_id || harness.status != "archived" {
                return Ok(false);
            }
            harness.status = "deleted".to_string();
            harness.deleted_at = Some(Self::now());
            harness.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    // ============================================
    // Sessions
    // ============================================

    // Agent identities

    pub async fn create_agent_identity(
        &self,
        input: CreateAgentIdentityRow,
    ) -> Result<AgentIdentityRow> {
        let row = AgentIdentityRow {
            id: input.id,
            org_id: input.org_id,
            name: input.name,
            description: input.description,
            avatar_url: input.avatar_url,
            locale: input.locale,
            timezone: input.timezone,
            status: "active".to_string(),
            created_at: Self::now(),
            updated_at: Self::now(),
            archived_at: None,
            deleted_at: None,
        };
        self.agent_identities.write().insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
    ) -> Result<Option<AgentIdentityRow>> {
        Ok(self
            .agent_identities
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    pub async fn list_agent_identities(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AgentIdentityRow>> {
        let search = search.map(|s| s.to_lowercase());
        let mut rows: Vec<_> = self
            .agent_identities
            .read()
            .values()
            .filter(|row| row.org_id == org_id)
            .filter(|row| {
                include_archived || !matches!(row.status.as_str(), "archived" | "deleted")
            })
            .filter(|row| match &search {
                Some(search) => {
                    row.name.to_lowercase().contains(search)
                        || row
                            .description
                            .as_ref()
                            .map(|d: &String| d.to_lowercase().contains(search))
                            .unwrap_or(false)
                }
                None => true,
            })
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(rows)
    }

    pub async fn update_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
        input: UpdateAgentIdentity,
    ) -> Result<Option<AgentIdentityRow>> {
        let mut rows = self.agent_identities.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(name) = input.name {
            row.name = name;
        }
        if let Some(description) = input.description {
            row.description = Some(description);
        }
        if let Some(avatar_url) = input.avatar_url {
            row.avatar_url = Some(avatar_url);
        }
        if let Some(locale) = input.locale {
            row.locale = Some(locale);
        }
        if let Some(timezone) = input.timezone {
            row.timezone = Some(timezone);
        }
        if let Some(status) = input.status {
            row.status = status;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        let mut rows = self.agent_identities.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(false);
        };
        if row.org_id != org_id || row.status != "active" {
            return Ok(false);
        }
        row.status = "archived".to_string();
        row.archived_at = Some(Self::now());
        row.updated_at = Self::now();
        Ok(true)
    }

    pub async fn destroy_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        let mut rows = self.agent_identities.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(false);
        };
        if row.org_id != org_id || row.status != "archived" {
            return Ok(false);
        }
        row.status = "deleted".to_string();
        row.deleted_at = Some(Self::now());
        row.updated_at = Self::now();
        Ok(true)
    }

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        let now = Self::now();
        let id = SessionId::new();
        let row = SessionRow {
            id,
            org_id: input.org_id,
            harness_id: input.harness_id,
            agent_id: input.agent_id,
            agent_identity_id: input.agent_identity_id,
            title: input.title,
            locale: input.locale,
            tags: input.tags,
            model_id: input.model_id,
            capabilities: input.capabilities,
            tools: input.tools,
            hints: input.hints,
            status: "pending".to_string(), // Default status for new sessions
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            parent_session_id: None,
            subagent_name: None,
            subagent_task: None,
            subagent_status: None,
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

    /// Get session without org scoping. For internal system use only (e.g. usage tracking).
    pub async fn get_session_unscoped(&self, id: SessionId) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        Ok(sessions.get(&id).cloned())
    }

    /// List sessions for an agent with pagination, validating org ownership.
    /// Returns (sessions, total_count).
    pub async fn list_sessions(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        search: Option<&str>,
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
            .filter(|s| s.org_id == org_id && agent_id.is_none_or(|aid| s.agent_id == Some(aid)))
            .filter(|s| matches_search_tokens(search, &[s.title.as_deref().unwrap_or("")]))
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

    /// Count sessions grouped by status for an organization.
    pub async fn count_sessions_by_status(&self, org_id: i64) -> Result<Vec<(String, i64)>> {
        let sessions = self.sessions.read();
        let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for s in sessions.values() {
            if s.org_id == org_id {
                *counts.entry(s.status.clone()).or_default() += 1;
            }
        }
        Ok(counts.into_iter().collect())
    }

    /// Find active sessions with Slack tags (for startup recovery).
    pub async fn find_active_slack_sessions(&self) -> Result<Vec<SessionRow>> {
        let sessions = self.sessions.read();
        let result: Vec<_> = sessions
            .values()
            .filter(|s| s.status == "active" && s.tags.iter().any(|t| t.starts_with("slack:app:")))
            .cloned()
            .collect();
        Ok(result)
    }

    /// Find sessions in `waiting_for_tool_results` with updated_at before cutoff.
    pub async fn list_sessions_waiting_tool_results_before(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<(SessionId, i64)>> {
        let sessions = self.sessions.read();
        let result: Vec<_> = sessions
            .values()
            .filter(|s| s.status == "waiting_for_tool_results" && s.updated_at < cutoff)
            .map(|s| (s.id, s.org_id))
            .collect();
        Ok(result)
    }

    /// Find a single session matching ALL given tags within an org.
    pub async fn find_session_by_tags(
        &self,
        org_id: i64,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let sessions = self.sessions.read();
        let mut result: Vec<_> = sessions
            .values()
            .filter(|s| s.org_id == org_id && tags.iter().all(|tag| s.tags.contains(tag)))
            .cloned()
            .collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result.into_iter().next())
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
            input
                .agent_identity_id
                .apply(&mut session.agent_identity_id);
            if let Some(locale) = input.locale {
                session.locale = Some(locale);
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

    #[allow(clippy::too_many_arguments)]
    pub async fn list_events(
        &self,
        session_id: SessionId,
        since_sequence: Option<i32>,
        since_id: Option<EventId>,
        filter_types: &[String],
        exclude_types: &[String],
        before_sequence: Option<i32>,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        let events = self.events.read();
        let mut result: Vec<_> = events
            .values()
            .filter(|e| {
                if e.session_id != session_id {
                    return false;
                }
                // Positive type filter: only include matching types (when non-empty)
                if !filter_types.is_empty() && !filter_types.contains(&e.event_type) {
                    return false;
                }
                // Negative type filter: exclude matching types
                if !exclude_types.is_empty() && exclude_types.contains(&e.event_type) {
                    return false;
                }
                // before_sequence cursor for backward pagination
                if let Some(before_seq) = before_sequence
                    && e.sequence >= before_seq
                {
                    return false;
                }
                // Resolve since_id to its sequence number for reliable ordering.
                // UUID v7 is NOT guaranteed monotonically increasing across
                // concurrent inserts, so always filter by sequence.
                if let Some(id) = since_id {
                    if let Some(ref_event) = events.get(&id) {
                        if e.sequence <= ref_event.sequence {
                            return false;
                        }
                    } else {
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

        // Apply limit: take last N events (backward pagination).
        // Safety cap of 10 000 when no explicit limit to prevent unbounded results.
        let effective_limit = limit.map(|l| l as usize).unwrap_or(10_000);
        if result.len() > effective_limit {
            result = result.split_off(result.len() - effective_limit);
        }

        Ok(result)
    }

    /// Find the nearest turn.started sequence at or before the given sequence.
    pub async fn find_turn_boundary(
        &self,
        session_id: SessionId,
        before_sequence: i32,
    ) -> Result<Option<i32>> {
        let events = self.events.read();
        let seq = events
            .values()
            .filter(|e| {
                e.session_id == session_id
                    && e.sequence <= before_sequence
                    && e.event_type == "turn.started"
            })
            .map(|e| e.sequence)
            .max();
        Ok(seq)
    }

    /// Check if an input.message event with a given slack_ts already exists in a session.
    pub async fn has_event_with_slack_ts(
        &self,
        session_id: SessionId,
        slack_ts: &str,
    ) -> Result<bool> {
        let events = self.events.read();
        let found = events.values().any(|e| {
            e.session_id == session_id
                && e.event_type == "input.message"
                && e.data
                    .pointer("/message/metadata/slack_ts")
                    .and_then(|v| v.as_str())
                    == Some(slack_ts)
        });
        Ok(found)
    }

    /// Count events for a session without materializing rows.
    pub async fn count_events(
        &self,
        session_id: SessionId,
        exclude_types: &[String],
    ) -> Result<i64> {
        let events = self.events.read();
        let count = events
            .values()
            .filter(|e| {
                e.session_id == session_id
                    && (exclude_types.is_empty() || !exclude_types.contains(&e.event_type))
            })
            .count();
        Ok(count as i64)
    }

    pub async fn list_message_events(&self, session_id: SessionId) -> Result<Vec<EventRow>> {
        self.list_message_events_limited(session_id, None).await
    }

    /// List message events with an optional limit.
    /// When `limit` is Some, returns the most recent N messages in sequence order.
    pub async fn list_message_events_limited(
        &self,
        session_id: SessionId,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
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
        if let Some(limit) = limit {
            let len = result.len();
            let limit = limit as usize;
            if len > limit {
                result = result.split_off(len - limit);
            }
        }
        Ok(result)
    }

    /// Count message events for a session — no row materialization.
    pub async fn count_message_events(&self, session_id: SessionId) -> Result<i64> {
        let message_types = [
            "input.message",
            "output.message.completed",
            "tool.completed",
        ];
        let events = self.events.read();
        let count = events
            .values()
            .filter(|e| {
                e.session_id == session_id && message_types.contains(&e.event_type.as_str())
            })
            .count();
        Ok(count as i64)
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
                            // Match PostgreSQL tsvector behavior: search data->>'content'
                            let content =
                                e.data.get("content").and_then(|v| v.as_str()).unwrap_or("");
                            if !content
                                .to_lowercase()
                                .contains(&search_query.to_lowercase())
                            {
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
    /// Create or update LLM provider with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_llm_provider_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmProviderRow,
    ) -> Result<Option<LlmProviderRow>> {
        let id = ProviderId::from_uuid(id);
        let mut providers = self.llm_providers.write();
        let now = Self::now();

        if let Some(existing) = providers.get(&id) {
            if existing.name == input.name && existing.provider_type == input.provider_type {
                return Ok(None); // Unchanged
            }
            let row = LlmProviderRow {
                name: input.name,
                provider_type: input.provider_type,
                updated_at: now,
                ..existing.clone()
            };
            providers.insert(id, row.clone());
            return Ok(Some(row));
        }

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

    pub async fn get_llm_provider(&self, org_id: i64, id: Uuid) -> Result<Option<LlmProviderRow>> {
        Ok(self
            .llm_providers
            .read()
            .get(&ProviderId::from_uuid(id))
            .filter(|p| p.org_id == org_id)
            .cloned())
    }

    pub async fn list_llm_providers(&self, org_id: i64) -> Result<Vec<LlmProviderRow>> {
        let providers = self.llm_providers.read();
        let mut result: Vec<_> = providers
            .values()
            .filter(|p| p.org_id == org_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    pub async fn update_llm_provider(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateLlmProvider,
    ) -> Result<Option<LlmProviderRow>> {
        let id = ProviderId::from_uuid(id);
        let mut providers = self.llm_providers.write();
        if let Some(provider) = providers.get_mut(&id) {
            if provider.org_id != org_id {
                return Ok(None);
            }
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

    pub async fn delete_llm_provider(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let id = ProviderId::from_uuid(id);
        // Check org_id before deletion
        {
            let providers = self.llm_providers.read();
            if let Some(provider) = providers.get(&id) {
                if provider.org_id != org_id {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }
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
        org_id: i64,
        id: Uuid,
        last_synced_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let id = ProviderId::from_uuid(id);
        if let Some(provider) = self.llm_providers.write().get_mut(&id) {
            if provider.org_id != org_id {
                return Ok(());
            }
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
        let org_settings = self.org_settings.read();
        let default_model_id = org_settings.get(&org_id).and_then(|s| s.default_model_id);
        let default_model_id = match default_model_id {
            Some(id) => id,
            None => return Ok(None),
        };
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        if let Some(model) = models.get(&default_model_id)
            && model.org_id == org_id
            && let Some(provider) = providers.get(&model.provider_id)
            && provider.org_id == org_id
            && model.status == "active"
            && provider.status == "active"
        {
            return Ok(Some(LlmModelWithProviderRow {
                id: model.id,
                org_id: model.org_id,
                provider_id: model.provider_id,
                model_id: model.model_id.clone(),
                display_name: model.display_name.clone(),
                capabilities: model.capabilities.clone(),
                is_favorite: model.is_favorite,
                installed: model.installed,
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

    // ============================================
    // Organization Settings (in-memory)
    // ============================================

    pub async fn get_organization_settings(
        &self,
        org_id: i64,
    ) -> Result<Option<OrganizationSettingsRow>> {
        Ok(self.org_settings.read().get(&org_id).cloned())
    }

    pub async fn upsert_organization_settings(
        &self,
        org_id: i64,
        default_model_id: Option<Uuid>,
    ) -> Result<OrganizationSettingsRow> {
        let now = Self::now();
        let mut settings = self.org_settings.write();
        let row = settings
            .entry(org_id)
            .or_insert_with(|| OrganizationSettingsRow {
                org_id,
                default_model_id: None,
                default_harness_id: None,
                base_harness_id: None,
                created_at: now,
                updated_at: now,
            });
        row.default_model_id = default_model_id.map(ModelId::from_uuid);
        row.updated_at = now;
        Ok(row.clone())
    }

    pub async fn patch_organization_settings(
        &self,
        org_id: i64,
        input: UpdateOrganizationSettings,
    ) -> Result<OrganizationSettingsRow> {
        let now = Self::now();
        let mut settings = self.org_settings.write();
        let row = settings
            .entry(org_id)
            .or_insert_with(|| OrganizationSettingsRow {
                org_id,
                default_model_id: None,
                default_harness_id: None,
                base_harness_id: None,
                created_at: now,
                updated_at: now,
            });

        input.default_model_id.apply(&mut row.default_model_id);
        input.default_harness_id.apply(&mut row.default_harness_id);
        input.base_harness_id.apply(&mut row.base_harness_id);

        row.updated_at = now;
        Ok(row.clone())
    }

    // ============================================
    // LLM Models (continued)
    // ============================================

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
            is_favorite: input.is_favorite,
            installed: input.installed,
            status: "active".to_string(),
            source: input.source,
            last_seen_at: None,
            provider_metadata: input.provider_metadata,
            created_at: now,
            updated_at: now,
        };
        self.llm_models.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create or update a model with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_llm_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmModelRow,
    ) -> Result<Option<LlmModelRow>> {
        let id = ModelId::from_uuid(id);
        let mut models = self.llm_models.write();
        let now = Self::now();

        if let Some(existing) = models.get(&id).cloned() {
            // Check if seed-controlled fields differ
            if existing.display_name == input.display_name
                && existing.is_favorite == input.is_favorite
                && existing.installed == input.installed
            {
                return Ok(None); // Unchanged
            }
            let row = LlmModelRow {
                display_name: input.display_name,
                is_favorite: input.is_favorite,
                installed: input.installed,
                updated_at: now,
                ..existing
            };
            models.insert(id, row.clone());
            return Ok(Some(row));
        }

        let row = LlmModelRow {
            id,
            org_id,
            provider_id: input.provider_id,
            model_id: input.model_id,
            display_name: input.display_name,
            capabilities: serde_json::to_value(&input.capabilities)?,
            is_favorite: input.is_favorite,
            installed: input.installed,
            status: "active".to_string(),
            source: input.source,
            last_seen_at: None,
            provider_metadata: input.provider_metadata,
            created_at: now,
            updated_at: now,
        };
        models.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_llm_model(&self, org_id: i64, id: Uuid) -> Result<Option<LlmModelRow>> {
        let id = ModelId::from_uuid(id);
        Ok(self
            .llm_models
            .read()
            .get(&id)
            .filter(|m| m.org_id == org_id)
            .cloned())
    }

    pub async fn get_llm_model_with_provider(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let id = ModelId::from_uuid(id);
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        if let Some(model) = models.get(&id)
            && model.org_id == org_id
            && let Some(provider) = providers.get(&model.provider_id)
            && provider.org_id == org_id
        {
            return Ok(Some(LlmModelWithProviderRow {
                id: model.id,
                org_id: model.org_id,
                provider_id: model.provider_id,
                model_id: model.model_id.clone(),
                display_name: model.display_name.clone(),
                capabilities: model.capabilities.clone(),
                is_favorite: model.is_favorite,
                installed: model.installed,
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
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModelRow>> {
        let provider_id = ProviderId::from_uuid(provider_id);
        let models = self.llm_models.read();
        let mut result: Vec<_> = models
            .values()
            .filter(|m| m.provider_id == provider_id && m.org_id == org_id)
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
            .filter(|model| model.org_id == org_id && model.status == "active")
            .filter_map(|model| {
                providers
                    .get(&model.provider_id)
                    .filter(|provider| provider.org_id == org_id && provider.status == "active")
                    .map(|provider| LlmModelWithProviderRow {
                        id: model.id,
                        org_id: model.org_id,
                        provider_id: model.provider_id,
                        model_id: model.model_id.clone(),
                        display_name: model.display_name.clone(),
                        capabilities: model.capabilities.clone(),
                        is_favorite: model.is_favorite,
                        installed: model.installed,
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
        // Sort by installed first, then favorite, then display_name
        result.sort_by(|a, b| {
            b.installed
                .cmp(&a.installed)
                .then_with(|| b.is_favorite.cmp(&a.is_favorite))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        Ok(result)
    }

    pub async fn update_llm_model(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateLlmModel,
    ) -> Result<Option<LlmModelRow>> {
        let id = ModelId::from_uuid(id);
        let mut models = self.llm_models.write();
        // Check existence and org ownership
        if models.get(&id).is_none_or(|m| m.org_id != org_id) {
            return Ok(None);
        }
        let model = models.get_mut(&id).unwrap();
        if let Some(display_name) = input.display_name {
            model.display_name = display_name;
        }
        if let Some(capabilities) = input.capabilities {
            model.capabilities = serde_json::to_value(&capabilities)?;
        }
        if let Some(is_favorite) = input.is_favorite {
            model.is_favorite = is_favorite;
        }
        if let Some(installed) = input.installed {
            model.installed = installed;
        }
        if let Some(status) = input.status {
            model.status = status;
        }
        model.updated_at = Self::now();
        Ok(Some(model.clone()))
    }

    pub async fn delete_llm_model(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let id = ModelId::from_uuid(id);
        let mut models = self.llm_models.write();
        if let Some(model) = models.get(&id) {
            if model.org_id != org_id {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
        Ok(models.remove(&id).is_some())
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
                && model.status == "active"
                && let Some(provider) = providers.get(&model.provider_id)
                && provider.org_id == org_id
                && provider.status == "active"
            {
                return Ok(Some(LlmModelWithProviderRow {
                    id: model.id,
                    org_id: model.org_id,
                    provider_id: model.provider_id,
                    model_id: model.model_id.clone(),
                    display_name: model.display_name.clone(),
                    capabilities: model.capabilities.clone(),
                    is_favorite: model.is_favorite,
                    installed: model.installed,
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
    // Harness Capabilities
    // ============================================

    pub async fn get_harness_capabilities(
        &self,
        harness_id: Uuid,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let harness_id = HarnessId::from_uuid(harness_id);
        let caps = self.harness_capabilities.read();
        let mut result: Vec<_> = caps
            .iter()
            .filter(|((hid, _), _)| *hid == harness_id)
            .map(|(_, c)| c.clone())
            .collect();
        result.sort_by_key(|c| c.position);
        Ok(result)
    }

    pub async fn set_harness_capabilities(
        &self,
        harness_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let harness_id = HarnessId::from_uuid(harness_id);
        let now = Self::now();
        let mut caps = self.harness_capabilities.write();

        // Remove existing capabilities for this harness
        let to_remove: Vec<_> = caps
            .keys()
            .filter(|(hid, _)| *hid == harness_id)
            .cloned()
            .collect();
        for key in to_remove {
            caps.remove(&key);
        }

        // Add new capabilities
        let mut result = Vec::new();
        for (capability_id, position, config) in capabilities.into_iter() {
            let row = HarnessCapabilityRow {
                id: Uuid::now_v7(),
                harness_id,
                capability_id: capability_id.clone(),
                position,
                config,
                created_at: now,
            };
            caps.insert((harness_id, capability_id), row.clone());
            result.push(row);
        }

        Ok(result)
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

    pub async fn update_session_file_if_content_matches(
        &self,
        session_id: Uuid,
        path: &str,
        expected_content: Vec<u8>,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        let mut files = self.session_files.write();
        if let Some(file) = files
            .values_mut()
            .find(|f| f.session_id == session_id && f.path == path)
        {
            if file.is_directory || file.is_readonly {
                return Ok(None);
            }

            let current_content = file.content.clone().unwrap_or_default();
            if current_content != expected_content {
                return Ok(None);
            }

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

    pub async fn has_readonly_session_files(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        let files = self.session_files.read();

        if path == "/" {
            return Ok(files
                .values()
                .any(|f| f.session_id == session_id && f.is_readonly));
        }

        let prefix = format!("{}/", path.trim_end_matches('/'));
        Ok(files.values().any(|f| {
            f.session_id == session_id
                && (f.path == path || f.path.starts_with(&prefix))
                && f.is_readonly
        }))
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
            archived_at: None,
            deleted_at: None,
        };

        self.mcp_servers.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create MCP server with a specific ID (for seeding)
    /// Returns None if server already exists with this ID
    /// Create or update MCP server with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_mcp_server_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateMcpServerRow,
    ) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        let mut servers = self.mcp_servers.write();
        let now = Self::now();

        if let Some(existing) = servers.get(&id) {
            if existing.name == input.name
                && existing.description == input.description
                && existing.url == input.url
                && existing.transport_type == input.transport_type
            {
                return Ok(None); // Unchanged
            }
            let row = McpServerRow {
                name: input.name,
                description: input.description,
                url: input.url,
                transport_type: input.transport_type,
                updated_at: now,
                ..existing.clone()
            };
            servers.insert(id, row.clone());
            return Ok(Some(row));
        }

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
            archived_at: None,
            deleted_at: None,
        };

        servers.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_mcp_server(&self, org_id: i64, id: Uuid) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        Ok(self
            .mcp_servers
            .read()
            .get(&id)
            .filter(|s| s.org_id == org_id)
            .cloned())
    }

    /// Batch fetch multiple MCP servers by IDs.
    pub async fn get_mcp_servers_batch(
        &self,
        org_id: i64,
        ids: &[Uuid],
    ) -> Result<Vec<McpServerRow>> {
        let servers = self.mcp_servers.read();
        Ok(ids
            .iter()
            .filter_map(|id| {
                servers
                    .get(&McpServerId::from_uuid(*id))
                    .filter(|s| s.org_id == org_id)
                    .cloned()
            })
            .collect())
    }

    pub async fn get_mcp_server_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<McpServerRow>> {
        Ok(self
            .mcp_servers
            .read()
            .values()
            .find(|s| s.name == name && s.org_id == org_id)
            .cloned())
    }

    pub async fn list_mcp_servers(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<McpServerRow>> {
        let mut servers: Vec<_> = self
            .mcp_servers
            .read()
            .values()
            .filter(|s| s.org_id == org_id)
            .filter(|s| {
                if include_archived {
                    s.status != "deleted"
                } else {
                    s.status != "archived" && s.status != "deleted"
                }
            })
            .filter(|s| {
                matches_search_tokens(search, &[&s.name, s.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        servers.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(servers)
    }

    pub async fn list_active_mcp_servers(&self, org_id: i64) -> Result<Vec<McpServerRow>> {
        let mut servers: Vec<_> = self
            .mcp_servers
            .read()
            .values()
            .filter(|s| s.status == "active" && s.org_id == org_id)
            .cloned()
            .collect();
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(servers)
    }

    pub async fn update_mcp_server(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id {
                return Ok(None);
            }
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

    pub async fn delete_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id || !matches!(server.status.as_str(), "active" | "disabled") {
                return Ok(false);
            }
            server.status = "archived".to_string();
            server.archived_at = Some(Self::now());
            server.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn destroy_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let id = McpServerId::from_uuid(id);
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id || server.status != "archived" {
                return Ok(false);
            }
            server.status = "deleted".to_string();
            server.deleted_at = Some(Self::now());
            server.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
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

    pub async fn create_image(&self, org_id: i64, input: CreateImageRow) -> Result<ImageRow> {
        let now = Self::now();
        let id = ImageId::new();
        let row = ImageRow {
            id,
            org_id,
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

    pub async fn get_image(&self, org_id: i64, id: Uuid) -> Result<Option<ImageRow>> {
        let id = ImageId::from_uuid(id);
        Ok(self
            .images
            .read()
            .get(&id)
            .filter(|img| img.org_id == org_id)
            .cloned())
    }

    pub async fn get_image_info(&self, org_id: i64, id: Uuid) -> Result<Option<ImageInfoRow>> {
        let id = ImageId::from_uuid(id);
        Ok(self
            .images
            .read()
            .get(&id)
            .filter(|img| img.org_id == org_id)
            .map(|img| ImageInfoRow {
                id: img.id,
                org_id: img.org_id,
                filename: img.filename.clone(),
                content_type: img.content_type.clone(),
                size_bytes: img.size_bytes,
                metadata: img.metadata.clone(),
                created_at: img.created_at,
            }))
    }

    pub async fn delete_image(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let id = ImageId::from_uuid(id);
        let mut images = self.images.write();
        if let Some(img) = images.get(&id) {
            if img.org_id != org_id {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
        Ok(images.remove(&id).is_some())
    }

    pub async fn list_images(
        &self,
        org_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ImageInfoRow>> {
        let images = self.images.read();
        let mut result: Vec<_> = images
            .values()
            .filter(|img| img.org_id == org_id)
            .map(|img| ImageInfoRow {
                id: img.id,
                org_id: img.org_id,
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
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id {
                return Ok(None);
            }
            server.cached_tools = input.cached_tools;
            server.tools_cached_at = Some(Self::now());
            server.updated_at = Self::now();
            return Ok(Some(server.clone()));
        }
        Ok(None)
    }

    // ============================================
    // Skills
    // ============================================

    pub async fn create_skill(&self, org_id: i64, input: CreateSkillRow) -> Result<SkillRow> {
        if self
            .skills
            .read()
            .values()
            .any(|s| s.name == input.name && s.org_id == org_id)
        {
            return Err(anyhow!("Skill with name '{}' already exists", input.name));
        }

        let now = Self::now();
        let id = SkillId::new();

        let row = SkillRow {
            id,
            public_id: input.public_id,
            org_id,
            name: input.name,
            description: input.description,
            license: input.license,
            compatibility: input.compatibility,
            metadata: input.metadata,
            allowed_tools: input.allowed_tools,
            instructions: input.instructions,
            source_type: input.source_type,
            archive_data: input.archive_data,
            status: "active".to_string(),
            version: input.version,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };

        self.skills.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_skill(&self, org_id: i64, id: Uuid) -> Result<Option<SkillRow>> {
        let id = SkillId::from_uuid(id);
        Ok(self
            .skills
            .read()
            .get(&id)
            .filter(|s| s.org_id == org_id)
            .cloned())
    }

    pub async fn get_skill_by_name(&self, org_id: i64, name: &str) -> Result<Option<SkillRow>> {
        Ok(self
            .skills
            .read()
            .values()
            .find(|s| s.org_id == org_id && s.name == name)
            .cloned())
    }

    pub async fn list_skills(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<SkillRow>> {
        let mut skills: Vec<_> = self
            .skills
            .read()
            .values()
            .filter(|s| s.org_id == org_id)
            .filter(|s| {
                if include_archived {
                    s.status != "deleted"
                } else {
                    s.status != "archived" && s.status != "deleted"
                }
            })
            .filter(|s| matches_search_tokens(search, &[&s.name, &s.description]))
            .cloned()
            .collect();
        skills.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(skills)
    }

    pub async fn update_skill(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateSkill,
    ) -> Result<Option<SkillRow>> {
        let id = SkillId::from_uuid(id);
        let mut skills = self.skills.write();
        if let Some(skill) = skills.get_mut(&id) {
            if skill.org_id != org_id {
                return Ok(None);
            }
            if let Some(name) = input.name {
                skill.name = name;
            }
            if let Some(description) = input.description {
                skill.description = description;
            }
            if let Some(license) = input.license {
                skill.license = Some(license);
            }
            if let Some(compatibility) = input.compatibility {
                skill.compatibility = Some(compatibility);
            }
            if let Some(metadata) = input.metadata {
                skill.metadata = metadata;
            }
            if let Some(allowed_tools) = input.allowed_tools {
                skill.allowed_tools = Some(allowed_tools);
            }
            if let Some(instructions) = input.instructions {
                skill.instructions = instructions;
            }
            if let Some(status) = input.status {
                skill.status = status;
            }
            if let Some(version) = input.version {
                skill.version = version;
            }
            if let Some(archive_data) = input.archive_data {
                skill.archive_data = Some(archive_data);
            }
            if let Some(source_type) = input.source_type {
                skill.source_type = source_type;
            }
            skill.updated_at = Self::now();
            return Ok(Some(skill.clone()));
        }
        Ok(None)
    }

    pub async fn delete_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let skill_id = SkillId::from_uuid(id);
        let mut skills = self.skills.write();
        if let Some(skill) = skills.get_mut(&skill_id) {
            if skill.org_id != org_id || !matches!(skill.status.as_str(), "active" | "disabled") {
                return Ok(false);
            }
            skill.status = "archived".to_string();
            skill.archived_at = Some(Self::now());
            skill.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn destroy_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let skill_id = SkillId::from_uuid(id);
        let mut skills = self.skills.write();
        if let Some(skill) = skills.get_mut(&skill_id) {
            if skill.org_id != org_id || skill.status != "archived" {
                return Ok(false);
            }
            skill.status = "deleted".to_string();
            skill.deleted_at = Some(Self::now());
            skill.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    // ============================================
    // Skill Files
    // ============================================

    pub async fn create_skill_file(&self, input: CreateSkillFileRow) -> Result<SkillFileRow> {
        let now = Self::now();
        let row = SkillFileRow {
            id: Uuid::now_v7(),
            skill_id: input.skill_id,
            path: input.path,
            content: input.content,
            content_binary: input.content_binary,
            is_binary: input.is_binary,
            size_bytes: input.size_bytes,
            created_at: now,
        };

        self.skill_files.write().push(row.clone());
        Ok(row)
    }

    pub async fn list_skill_files(&self, skill_id: Uuid) -> Result<Vec<SkillFileRow>> {
        let mut files: Vec<_> = self
            .skill_files
            .read()
            .iter()
            .filter(|f| f.skill_id == skill_id)
            .cloned()
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    pub async fn delete_skill_files(&self, skill_id: Uuid) -> Result<u64> {
        let mut files = self.skill_files.write();
        let before = files.len();
        files.retain(|f| f.skill_id != skill_id);
        Ok((before - files.len()) as u64)
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
            external_id: None,
            created_by: input.created_by,
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
            external_id: None,
            created_by: input.created_by,
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
        role: &str,
    ) -> Result<OrganizationMemberRow> {
        let now = Self::now();
        let row = OrganizationMemberRow {
            org_id,
            user_id,
            role: role.to_string(),
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

    pub async fn list_organization_members_with_users(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberWithUserRow>> {
        let members = self.organization_members.read();
        let users = self.users.read();
        let mut result: Vec<_> = members
            .values()
            .filter(|m| m.org_id == org_id)
            .filter_map(|m| {
                users
                    .get(&m.user_id)
                    .map(|u| OrganizationMemberWithUserRow {
                        user_id: u.id,
                        email: u.email.clone(),
                        name: u.name.clone(),
                        avatar_url: u.avatar_url.clone(),
                        role: m.role.clone(),
                        joined_at: m.created_at,
                    })
            })
            .collect();
        result.sort_by(|a, b| a.joined_at.cmp(&b.joined_at));
        Ok(result)
    }

    pub async fn get_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Option<OrganizationMemberWithUserRow>> {
        let members = self.organization_members.read();
        let users = self.users.read();
        Ok(members.get(&(org_id, user_id)).and_then(|m| {
            users
                .get(&m.user_id)
                .map(|u| OrganizationMemberWithUserRow {
                    user_id: u.id,
                    email: u.email.clone(),
                    name: u.name.clone(),
                    avatar_url: u.avatar_url.clone(),
                    role: m.role.clone(),
                    joined_at: m.created_at,
                })
        }))
    }

    pub async fn update_organization_member_role(
        &self,
        org_id: i64,
        user_id: Uuid,
        role: &str,
    ) -> Result<Option<OrganizationMemberRow>> {
        let mut members = self.organization_members.write();
        if let Some(member) = members.get_mut(&(org_id, user_id)) {
            member.role = role.to_string();
            return Ok(Some(member.clone()));
        }
        Ok(None)
    }

    pub async fn count_organization_owners(&self, org_id: i64) -> Result<i64> {
        let members = self.organization_members.read();
        let count = members
            .values()
            .filter(|m| m.org_id == org_id && m.role == "owner")
            .count();
        Ok(count as i64)
    }

    pub async fn list_user_organizations(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OrganizationWithRoleRow>> {
        let members = self.organization_members.read();
        let orgs = self.organizations.read();
        let mut result: Vec<_> = members
            .values()
            .filter(|m| m.user_id == user_id)
            .filter_map(|m| {
                orgs.get(&m.org_id).map(|o| OrganizationWithRoleRow {
                    org_id: o.org_id,
                    public_id: o.public_id.clone(),
                    name: o.name.clone(),
                    role: m.role.clone(),
                })
            })
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

    /// Get user by external identity provider ID
    pub async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<UserRow>> {
        Ok(self
            .users
            .read()
            .values()
            .find(|u| u.external_id.as_deref() == Some(external_id))
            .cloned())
    }

    /// Get organization by external identity provider ID
    pub async fn get_organization_by_external_id(
        &self,
        external_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        Ok(self
            .organizations
            .read()
            .values()
            .find(|o| o.external_id.as_deref() == Some(external_id))
            .cloned())
    }

    /// Upsert organization by external ID (for external auth provider sync)
    pub async fn upsert_org_by_external_id(
        &self,
        external_id: &str,
        public_id: &str,
        name: &str,
    ) -> Result<OrganizationRow> {
        let mut orgs = self.organizations.write();

        // Check if org with this external_id already exists
        if let Some(existing) = orgs
            .values_mut()
            .find(|o| o.external_id.as_deref() == Some(external_id))
        {
            existing.name = name.to_string();
            existing.updated_at = Self::now();
            return Ok(existing.clone());
        }

        // Create new org
        let org_id = orgs.keys().max().unwrap_or(&0) + 1;
        let now = Self::now();
        let row = OrganizationRow {
            org_id,
            public_id: public_id.to_string(),
            name: name.to_string(),
            created_at: now,
            updated_at: now,
            external_id: Some(external_id.to_string()),
            created_by: None,
        };
        orgs.insert(org_id, row.clone());
        Ok(row)
    }

    /// Ensure user is a member of organization with the given role (upsert).
    /// Updates the role if the membership already exists.
    pub async fn ensure_membership(&self, user_id: Uuid, org_id: i64, role: &str) -> Result<()> {
        let key = (org_id, user_id);
        let mut members = self.organization_members.write();
        members
            .entry(key)
            .and_modify(|m| m.role = role.to_string())
            .or_insert_with(|| OrganizationMemberRow {
                org_id,
                user_id,
                role: role.to_string(),
                created_at: Self::now(),
            });
        Ok(())
    }

    /// Reconcile org memberships to match an authoritative list.
    /// Operates under a single write lock for atomicity.
    /// Duplicate user_ids in the authoritative list are deduplicated (last wins).
    /// Returns `(added, updated, removed)` counts.
    pub async fn reconcile_memberships(
        &self,
        org_id: i64,
        authoritative: &[(Uuid, String)],
    ) -> Result<(usize, usize, usize)> {
        // Deduplicate authoritative input (last occurrence wins)
        let auth_map: std::collections::HashMap<Uuid, &str> = authoritative
            .iter()
            .map(|(uid, role)| (*uid, role.as_str()))
            .collect();

        let mut members = self.organization_members.write();
        let now = Self::now();

        // Build current state under the write lock
        let current_map: std::collections::HashMap<Uuid, String> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), row)| (*uid, row.role.clone()))
            .collect();

        let mut added = 0usize;
        let mut updated = 0usize;
        let mut removed = 0usize;

        // Add or update from the deduped map
        for (user_id, role) in &auth_map {
            match current_map.get(user_id) {
                None => {
                    members.insert(
                        (org_id, *user_id),
                        OrganizationMemberRow {
                            org_id,
                            user_id: *user_id,
                            role: role.to_string(),
                            created_at: now,
                        },
                    );
                    added += 1;
                }
                Some(existing_role) if existing_role != role => {
                    if let Some(row) = members.get_mut(&(org_id, *user_id)) {
                        row.role = role.to_string();
                    }
                    updated += 1;
                }
                _ => {} // unchanged
            }
        }

        // Remove members not in authoritative list
        for user_id in current_map.keys() {
            if !auth_map.contains_key(user_id) {
                members.remove(&(org_id, *user_id));
                removed += 1;
            }
        }

        Ok((added, updated, removed))
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

    pub async fn upsert_session_secret(
        &self,
        input: UpsertSessionSecret,
    ) -> Result<SessionSecretRow> {
        let now = Self::now();
        let map_key = (input.session_id, input.name.clone());
        let mut storage = self.session_secrets.write();

        let row = if let Some(existing) = storage.get_mut(&map_key) {
            existing.value_encrypted = input.value_encrypted;
            existing.updated_at = now;
            existing.clone()
        } else {
            let row = SessionSecretRow {
                id: Uuid::now_v7(),
                session_id: input.session_id,
                name: input.name,
                value_encrypted: input.value_encrypted,
                created_at: now,
                updated_at: now,
            };
            storage.insert(map_key, row.clone());
            row
        };

        Ok(row)
    }
}

// ============================================================================
// SessionStorageStore implementation for in-memory backend
// ============================================================================

#[async_trait::async_trait]
impl everruns_core::traits::SessionStorageStore for InMemoryDatabase {
    async fn set_value(
        &self,
        session_id: SessionId,
        key: &str,
        value: &str,
    ) -> everruns_core::Result<()> {
        let now = Self::now();
        let mut storage = self.session_key_values.write();
        let map_key = (session_id, key.to_string());
        storage
            .entry(map_key)
            .and_modify(|row| {
                row.value = value.to_string();
                row.updated_at = now;
            })
            .or_insert_with(|| SessionKeyValueRow {
                id: Uuid::now_v7(),
                session_id,
                key: key.to_string(),
                value: value.to_string(),
                created_at: now,
                updated_at: now,
            });
        Ok(())
    }

    async fn get_value(
        &self,
        session_id: SessionId,
        key: &str,
    ) -> everruns_core::Result<Option<String>> {
        let storage = self.session_key_values.read();
        Ok(storage
            .get(&(session_id, key.to_string()))
            .map(|r| r.value.clone()))
    }

    async fn delete_value(&self, session_id: SessionId, key: &str) -> everruns_core::Result<bool> {
        let mut storage = self.session_key_values.write();
        Ok(storage.remove(&(session_id, key.to_string())).is_some())
    }

    async fn list_keys(
        &self,
        session_id: SessionId,
    ) -> everruns_core::Result<Vec<everruns_core::KeyInfo>> {
        let storage = self.session_key_values.read();
        let mut keys: Vec<_> = storage
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|(_, row)| everruns_core::KeyInfo {
                key: row.key.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();
        keys.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(keys)
    }

    async fn set_secret(
        &self,
        session_id: SessionId,
        name: &str,
        value: &str,
    ) -> everruns_core::Result<()> {
        let now = Self::now();
        let mut storage = self.session_secrets.write();
        let map_key = (session_id, name.to_string());
        // In-memory: store plain text as bytes (no encryption in dev mode)
        storage
            .entry(map_key)
            .and_modify(|row| {
                row.value_encrypted = value.as_bytes().to_vec();
                row.updated_at = now;
            })
            .or_insert_with(|| SessionSecretRow {
                id: Uuid::now_v7(),
                session_id,
                name: name.to_string(),
                value_encrypted: value.as_bytes().to_vec(),
                created_at: now,
                updated_at: now,
            });
        Ok(())
    }

    async fn get_secret(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> everruns_core::Result<Option<String>> {
        let storage = self.session_secrets.read();
        Ok(storage
            .get(&(session_id, name.to_string()))
            .map(|r| String::from_utf8_lossy(&r.value_encrypted).to_string()))
    }

    async fn delete_secret(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> everruns_core::Result<bool> {
        let mut storage = self.session_secrets.write();
        Ok(storage.remove(&(session_id, name.to_string())).is_some())
    }

    async fn list_secrets(
        &self,
        session_id: SessionId,
    ) -> everruns_core::Result<Vec<everruns_core::SecretInfo>> {
        let storage = self.session_secrets.read();
        let mut secrets: Vec<_> = storage
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|(_, row)| everruns_core::SecretInfo {
                name: row.name.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();
        secrets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(secrets)
    }
}

impl InMemoryDatabase {
    // ============================================
    // User Connections
    // ============================================

    pub async fn upsert_user_connection(
        &self,
        input: CreateUserConnectionRow,
    ) -> Result<UserConnectionRow> {
        let now = Self::now();
        let id = Uuid::now_v7();

        // Remove existing connection for same user+provider (app-level uniqueness)
        let mut connections = self.user_connections.write();
        connections.retain(|_, c| !(c.user_id == input.user_id && c.provider == input.provider));

        let row = UserConnectionRow {
            id,
            user_id: input.user_id,
            provider: input.provider,
            connection_type: input.connection_type,
            provider_user_id: input.provider_user_id,
            provider_username: input.provider_username,
            access_token_encrypted: input.access_token_encrypted,
            refresh_token_encrypted: input.refresh_token_encrypted,
            scopes: input.scopes,
            expires_at: input.expires_at,
            installation_id: input.installation_id,
            created_at: now,
            updated_at: now,
        };
        connections.insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_user_connection(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<UserConnectionRow>> {
        Ok(self
            .user_connections
            .read()
            .values()
            .find(|c| c.user_id == user_id && c.provider == provider)
            .cloned())
    }

    pub async fn list_user_connections(&self, user_id: Uuid) -> Result<Vec<UserConnectionRow>> {
        let mut connections: Vec<_> = self
            .user_connections
            .read()
            .values()
            .filter(|c| c.user_id == user_id)
            .cloned()
            .collect();
        connections.sort_by(|a, b| a.provider.cmp(&b.provider));
        Ok(connections)
    }

    /// Get encrypted connection token for a session's org member (in-memory equivalent).
    pub async fn get_connection_token_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        // Find the session to get org_id
        let sessions = self.sessions.read();
        let session = match sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let org_id = session.org_id;
        drop(sessions);

        // Find org members
        let members = self.organization_members.read();
        let user_ids: Vec<Uuid> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), _)| *uid)
            .collect();
        drop(members);

        // Find first matching connection with an encrypted token
        let connections = self.user_connections.read();
        for uid in user_ids.clone() {
            for conn in connections.values() {
                if conn.user_id == uid
                    && conn.provider == provider
                    && conn.access_token_encrypted.is_some()
                {
                    return Ok(conn.access_token_encrypted.clone());
                }
            }
        }

        Ok(None)
    }

    pub async fn get_connection_user_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Uuid>> {
        let sessions = self.sessions.read();
        let session = match sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let org_id = session.org_id;
        drop(sessions);

        let members = self.organization_members.read();
        let user_ids: Vec<Uuid> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), _)| *uid)
            .collect();
        drop(members);

        let connections = self.user_connections.read();
        for uid in user_ids {
            if connections
                .values()
                .any(|conn| conn.user_id == uid && conn.provider == provider)
            {
                return Ok(Some(uid));
            }
        }

        Ok(None)
    }

    pub async fn get_connection_token_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .user_connections
            .read()
            .values()
            .find(|c| {
                c.user_id == user_id && c.provider == provider && c.access_token_encrypted.is_some()
            })
            .and_then(|c| c.access_token_encrypted.clone()))
    }

    pub async fn get_installation_id_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<i64>> {
        // Look up session → org → members → connections (same join as token lookup)
        let sessions = self.sessions.read();
        let session = match sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let org_id = session.org_id;
        drop(sessions);

        let members = self.organization_members.read();
        let user_ids: Vec<Uuid> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), _)| *uid)
            .collect();
        drop(members);

        let connections = self.user_connections.read();
        for uid in user_ids {
            for conn in connections.values() {
                if conn.user_id == uid
                    && conn.provider == provider
                    && conn.installation_id.is_some()
                {
                    return Ok(conn.installation_id);
                }
            }
        }

        Ok(None)
    }

    pub async fn get_installation_id_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<i64>> {
        Ok(self
            .user_connections
            .read()
            .values()
            .find(|c| c.user_id == user_id && c.provider == provider && c.installation_id.is_some())
            .and_then(|c| c.installation_id))
    }

    pub async fn delete_user_connection(&self, user_id: Uuid, provider: &str) -> Result<bool> {
        let mut connections = self.user_connections.write();
        let before = connections.len();
        connections.retain(|_, c| !(c.user_id == user_id && c.provider == provider));
        Ok(connections.len() < before)
    }

    // ============================================
    // Pinned Sessions
    // ============================================

    pub async fn pin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<()> {
        let mut pins = self.pinned_sessions.write();
        pins.entry((user_id, session_id))
            .or_insert((org_id, Self::now()));
        Ok(())
    }

    pub async fn unpin_session(&self, user_id: Uuid, session_id: SessionId) -> Result<bool> {
        let mut pins = self.pinned_sessions.write();
        Ok(pins.remove(&(user_id, session_id)).is_some())
    }

    pub async fn list_pinned_session_ids(
        &self,
        user_id: Uuid,
        org_id: i64,
    ) -> Result<Vec<SessionId>> {
        let pins = self.pinned_sessions.read();
        let mut entries: Vec<_> = pins
            .iter()
            .filter(|((uid, _), (oid, _))| *uid == user_id && *oid == org_id)
            .map(|((_, sid), (_, pinned_at))| (*sid, *pinned_at))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1)); // Most recently pinned first
        Ok(entries.into_iter().map(|(sid, _)| sid).collect())
    }

    // ============================================
    // Notifications
    // ============================================

    pub async fn create_notification_turn_request(
        &self,
        input: CreateNotificationTurnRequestRow,
    ) -> Result<()> {
        let row = NotificationTurnRequestRow {
            input_message_id: input.input_message_id,
            org_id: input.org_id,
            user_id: input.user_id,
            session_id: input.session_id,
            created_at: Self::now(),
        };
        self.notification_turn_requests
            .write()
            .insert(input.input_message_id, row);
        Ok(())
    }

    pub async fn get_notification_turn_request(
        &self,
        input_message_id: MessageId,
    ) -> Result<Option<NotificationTurnRequestRow>> {
        Ok(self
            .notification_turn_requests
            .read()
            .get(&input_message_id)
            .cloned())
    }

    pub async fn create_notification(
        &self,
        input: CreateNotificationRow,
    ) -> Result<NotificationRow> {
        let now = Self::now();
        let mut notifications = self.notifications.write();

        if let Some(existing_id) = notifications
            .values()
            .find(|row| {
                row.org_id == input.org_id
                    && row.user_id == input.user_id
                    && row.viewed_at.is_none()
                    && row.dedupe_key.is_some()
                    && row.dedupe_key == input.dedupe_key
            })
            .map(|row| row.id)
            && let Some(existing) = notifications.get_mut(&existing_id)
        {
            existing.title = input.title;
            existing.body = input.body;
            existing.target_type = input.target_type;
            existing.target_id = input.target_id;
            existing.href = input.href;
            existing.payload = input.payload;
            existing.occurrence_count += 1;
            existing.updated_at = now;
            return Ok(existing.clone());
        }

        let row = NotificationRow {
            id: NotificationId::new(),
            org_id: input.org_id,
            user_id: input.user_id,
            kind: input.kind,
            title: input.title,
            body: input.body,
            target_type: input.target_type,
            target_id: input.target_id,
            href: input.href,
            payload: input.payload,
            dedupe_key: input.dedupe_key,
            occurrence_count: 1,
            viewed_at: None,
            created_at: now,
            updated_at: now,
        };
        notifications.insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_notification(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        Ok(self.notifications.read().get(&id).and_then(|row| {
            if row.org_id == org_id && row.user_id == user_id {
                Some(row.clone())
            } else {
                None
            }
        }))
    }

    pub async fn list_notifications(
        &self,
        org_id: i64,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        let mut rows: Vec<_> = self
            .notifications
            .read()
            .values()
            .filter(|row| row.org_id == org_id && row.user_id == user_id)
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.uuid().cmp(&a.id.uuid()))
        });
        rows.truncate(limit as usize);
        Ok(rows)
    }

    pub async fn list_notifications_updated_since(
        &self,
        org_id: i64,
        user_id: Uuid,
        updated_since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        let mut rows: Vec<_> = self
            .notifications
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && row.user_id == user_id
                    && updated_since.is_none_or(|ts| row.updated_at >= ts)
            })
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            a.updated_at
                .cmp(&b.updated_at)
                .then_with(|| a.id.uuid().cmp(&b.id.uuid()))
        });
        rows.truncate(limit as usize);
        Ok(rows)
    }

    pub async fn count_unviewed_notifications(&self, org_id: i64, user_id: Uuid) -> Result<u32> {
        Ok(self
            .notifications
            .read()
            .values()
            .filter(|row| row.org_id == org_id && row.user_id == user_id && row.viewed_at.is_none())
            .count() as u32)
    }

    pub async fn count_unviewed_notifications_by_kind(
        &self,
        org_id: i64,
        user_id: Uuid,
        kind: &str,
    ) -> Result<u32> {
        Ok(self
            .notifications
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && row.user_id == user_id
                    && row.kind == kind
                    && row.viewed_at.is_none()
            })
            .count() as u32)
    }

    pub async fn mark_notification_viewed(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        let now = Self::now();
        let mut notifications = self.notifications.write();
        let Some(row) = notifications.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id || row.user_id != user_id {
            return Ok(None);
        }
        if row.viewed_at.is_none() {
            row.viewed_at = Some(now);
            row.updated_at = now;
        }
        Ok(Some(row.clone()))
    }

    // ============================================
    // Session Schedules
    // ============================================

    pub async fn create_session_schedule(
        &self,
        input: CreateSessionScheduleRow,
    ) -> Result<SessionScheduleRow> {
        let now = Self::now();
        let id = ScheduleId::new();
        let public_id = id.to_string();

        let row = SessionScheduleRow {
            id,
            public_id,
            org_id: input.org_id,
            session_id: input.session_id,
            description: input.description,
            cron_expression: input.cron_expression,
            scheduled_at: input.scheduled_at,
            timezone: input.timezone,
            enabled: true,
            next_trigger_at: input.next_trigger_at,
            last_triggered_at: None,
            trigger_count: 0,
            created_at: now,
            updated_at: now,
        };
        self.session_schedules.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<Option<SessionScheduleRow>> {
        Ok(self
            .session_schedules
            .read()
            .get(&schedule_id)
            .filter(|r| r.org_id == org_id)
            .cloned())
    }

    pub async fn list_session_schedules(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionScheduleRow>> {
        let schedules = self.session_schedules.read();
        let mut result: Vec<_> = schedules
            .values()
            .filter(|r| r.org_id == org_id && r.session_id == session_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn update_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
        input: UpdateSessionScheduleRow,
    ) -> Result<Option<SessionScheduleRow>> {
        let mut schedules = self.session_schedules.write();
        let Some(row) = schedules.get_mut(&schedule_id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(enabled) = input.enabled {
            row.enabled = enabled;
        }
        input.next_trigger_at.apply(&mut row.next_trigger_at);
        if let Some(last) = input.last_triggered_at {
            row.last_triggered_at = Some(last);
        }
        if input.trigger_count_increment {
            row.trigger_count += 1;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<bool> {
        let mut schedules = self.session_schedules.write();
        if let Some(row) = schedules.get(&schedule_id) {
            if row.org_id != org_id {
                return Ok(false);
            }
            schedules.remove(&schedule_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn count_active_session_schedules(&self, session_id: SessionId) -> Result<u32> {
        let schedules = self.session_schedules.read();
        let count = schedules
            .values()
            .filter(|r| r.session_id == session_id && r.enabled)
            .count();
        Ok(count as u32)
    }

    pub async fn claim_due_session_schedules(&self, limit: i32) -> Result<Vec<SessionScheduleRow>> {
        let now = Self::now();
        let schedules = self.session_schedules.read();
        let mut due: Vec<_> = schedules
            .values()
            .filter(|r| r.enabled && r.next_trigger_at.is_some_and(|t| t <= now))
            .cloned()
            .collect();
        due.sort_by(|a, b| a.next_trigger_at.cmp(&b.next_trigger_at));
        due.truncate(limit as usize);
        Ok(due)
    }

    // ============================================
    // Leased Resources
    // ============================================

    pub async fn get_session_organization_id(&self, session_id: SessionId) -> Result<Option<i64>> {
        Ok(self.sessions.read().get(&session_id).map(|row| row.org_id))
    }

    pub async fn upsert_leased_resource(
        &self,
        input: UpsertLeasedResourceRow,
    ) -> Result<LeasedResourceRow> {
        let now = Self::now();
        let mut resources = self.leased_resources.write();

        if let Some(existing) = resources.values_mut().find(|row| {
            row.org_id == input.org_id
                && row.provider == input.provider
                && row.resource_type == input.resource_type
                && row.external_id == input.external_id
        }) {
            existing.session_id = Some(input.session_id);
            existing.display_name = input.display_name.or_else(|| existing.display_name.clone());
            existing.status = "active".to_string();
            existing.owner_user_id = input.owner_user_id.or(existing.owner_user_id);
            existing.lease_duration_seconds = input.lease_duration_seconds;
            existing.last_touched_at = now;
            existing.lease_expires_at = input.lease_expires_at;
            existing.cleanup_started_at = None;
            existing.cleanup_completed_at = None;
            existing.last_cleanup_error = None;
            existing.metadata = input.metadata;
            existing.updated_at = now;
            return Ok(existing.clone());
        }

        let id = LeasedResourceId::new();
        let row = LeasedResourceRow {
            id,
            public_id: id.to_string(),
            org_id: input.org_id,
            session_id: Some(input.session_id),
            provider: input.provider,
            resource_type: input.resource_type,
            external_id: input.external_id,
            display_name: input.display_name,
            status: "active".to_string(),
            owner_user_id: input.owner_user_id,
            lease_duration_seconds: input.lease_duration_seconds,
            last_touched_at: now,
            lease_expires_at: input.lease_expires_at,
            cleanup_started_at: None,
            cleanup_completed_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
        };
        resources.insert(id, row.clone());
        Ok(row)
    }

    pub async fn release_leased_resource(
        &self,
        input: ReleaseLeasedResourceRow,
    ) -> Result<Option<LeasedResourceRow>> {
        let now = Self::now();
        let mut resources = self.leased_resources.write();
        let row = resources.values_mut().find(|row| {
            row.org_id == input.org_id
                && row.session_id == Some(input.session_id)
                && row.provider == input.provider
                && row.resource_type == input.resource_type
                && row.external_id == input.external_id
        });
        let Some(row) = row else {
            return Ok(None);
        };

        row.status = "released".to_string();
        row.cleanup_started_at = None;
        row.cleanup_completed_at = Some(now);
        row.lease_expires_at = now;
        row.last_cleanup_error = None;
        row.updated_at = now;
        Ok(Some(row.clone()))
    }

    pub async fn list_session_leased_resources(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<LeasedResourceRow>> {
        let resources = self.leased_resources.read();
        let mut result: Vec<_> = resources
            .values()
            .filter(|row| row.session_id == Some(session_id))
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn claim_due_leased_resources(
        &self,
        limit: i32,
        stale_after_seconds: i32,
    ) -> Result<Vec<LeasedResourceRow>> {
        let now = Self::now();
        let stale_before = now - chrono::TimeDelta::seconds(stale_after_seconds as i64);
        let mut resources = self.leased_resources.write();
        let mut due_ids: Vec<_> = resources
            .values()
            .filter(|row| {
                (matches!(row.status.as_str(), "active" | "cleanup_failed")
                    && row.lease_expires_at <= now)
                    || (row.status == "cleaning"
                        && row.cleanup_started_at.is_some_and(|ts| ts <= stale_before))
            })
            .map(|row| row.id)
            .collect();
        due_ids.sort_by(|a, b| {
            let left = resources.get(a).expect("leased resource must exist");
            let right = resources.get(b).expect("leased resource must exist");
            (
                &left.lease_expires_at,
                &left.cleanup_started_at,
                &left.created_at,
            )
                .cmp(&(
                    &right.lease_expires_at,
                    &right.cleanup_started_at,
                    &right.created_at,
                ))
        });
        due_ids.truncate(limit as usize);

        let mut claimed = Vec::with_capacity(due_ids.len());
        for id in due_ids {
            if let Some(row) = resources.get_mut(&id) {
                row.status = "cleaning".to_string();
                row.cleanup_started_at = Some(now);
                row.cleanup_attempts += 1;
                row.last_cleanup_error = None;
                row.updated_at = now;
                claimed.push(row.clone());
            }
        }

        Ok(claimed)
    }

    pub async fn mark_leased_resource_released(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
    ) -> Result<Option<LeasedResourceRow>> {
        let now = Self::now();
        let mut resources = self.leased_resources.write();
        let Some(row) = resources.get_mut(&resource_id) else {
            return Ok(None);
        };
        if row.status != "cleaning" || row.cleanup_started_at != Some(expected_cleanup_started_at) {
            return Ok(None);
        }

        row.status = "released".to_string();
        row.cleanup_started_at = None;
        row.cleanup_completed_at = Some(now);
        row.lease_expires_at = now;
        row.last_cleanup_error = None;
        row.updated_at = now;
        Ok(Some(row.clone()))
    }

    pub async fn mark_leased_resource_cleanup_failed(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
        retry_after_seconds: i32,
        error: &str,
    ) -> Result<Option<LeasedResourceRow>> {
        let now = Self::now();
        let mut resources = self.leased_resources.write();
        let Some(row) = resources.get_mut(&resource_id) else {
            return Ok(None);
        };
        if row.status != "cleaning" || row.cleanup_started_at != Some(expected_cleanup_started_at) {
            return Ok(None);
        }

        row.status = "cleanup_failed".to_string();
        row.cleanup_started_at = None;
        row.lease_expires_at = now + chrono::TimeDelta::seconds(retry_after_seconds as i64);
        row.last_cleanup_error = Some(error.to_string());
        row.updated_at = now;
        Ok(Some(row.clone()))
    }

    // Audit Logs (TM-OBS-007)

    pub async fn create_audit_log(&self, input: CreateAuditLogRow) -> Result<AuditLogRow> {
        let row = AuditLogRow {
            id: Uuid::now_v7(),
            org_id: input.org_id,
            actor_id: input.actor_id,
            event_type: input.event_type,
            ip_address: input.ip_address,
            metadata: input.metadata,
            created_at: Self::now(),
        };
        self.audit_logs.write().push(row.clone());
        Ok(row)
    }

    pub async fn list_audit_logs(
        &self,
        org_id: i64,
        limit: i64,
        before: Option<DateTime<Utc>>,
        event_type_prefix: Option<&str>,
        actor_id: Option<Uuid>,
    ) -> Result<Vec<AuditLogRow>> {
        let logs = self.audit_logs.read();
        let before_ts = before.unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(1));
        let mut filtered: Vec<_> = logs
            .iter()
            .filter(|r| {
                r.org_id == org_id
                    && r.created_at < before_ts
                    && event_type_prefix.is_none_or(|p| r.event_type.starts_with(p))
                    && actor_id.is_none_or(|a| r.actor_id == Some(a))
            })
            .cloned()
            .collect();
        filtered.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        filtered.truncate(limit as usize);
        Ok(filtered)
    }

    pub async fn delete_audit_logs_before(&self, before: DateTime<Utc>) -> Result<u64> {
        let mut logs = self.audit_logs.write();
        let initial = logs.len();
        logs.retain(|r| r.created_at >= before);
        Ok((initial - logs.len()) as u64)
    }

    // ============================================
    // App CRUD
    // ============================================

    pub async fn create_app(&self, org_id: i64, input: CreateAppRow) -> Result<AppRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = AppRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            harness_id: input.harness_id,
            agent_id: input.agent_id,
            agent_identity_id: input.agent_identity_id,
            channel_type: input.channel_type,
            channel_config: input.channel_config,
            channel_config_encrypted: input.channel_config_encrypted,
            status: "draft".to_string(),
            published_at: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        self.apps.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_app_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AppRow>> {
        let apps = self.apps.read();
        Ok(apps
            .values()
            .find(|a| a.org_id == org_id && a.public_id == public_id)
            .cloned())
    }

    /// Lookup app by public_id without org scoping (for unauthenticated webhooks).
    pub async fn get_app_by_public_id_unscoped(&self, public_id: &str) -> Result<Option<AppRow>> {
        let apps = self.apps.read();
        Ok(apps.values().find(|a| a.public_id == public_id).cloned())
    }

    pub async fn list_apps(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AppRow>> {
        let apps = self.apps.read();
        let mut result: Vec<AppRow> = apps
            .values()
            .filter(|a| {
                a.org_id == org_id
                    && if include_archived {
                        a.status != "deleted"
                    } else {
                        a.status != "archived" && a.status != "deleted"
                    }
            })
            .filter(|a| {
                matches_search_tokens(search, &[&a.name, a.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn update_app(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateApp,
    ) -> Result<Option<AppRow>> {
        let mut apps = self.apps.write();
        let Some(app) = apps.get_mut(&id) else {
            return Ok(None);
        };
        if app.org_id != org_id {
            return Ok(None);
        }
        if let Some(name) = input.name {
            app.name = name;
        }
        if let Some(description) = input.description {
            app.description = Some(description);
        }
        if let Some(harness_id) = input.harness_id {
            app.harness_id = harness_id;
        }
        if let Some(agent_id) = input.agent_id {
            app.agent_id = agent_id;
        }
        input.agent_identity_id.apply(&mut app.agent_identity_id);
        if let Some(channel_type) = input.channel_type {
            app.channel_type = channel_type;
        }
        if let Some(channel_config) = input.channel_config {
            app.channel_config = channel_config;
        }
        if let Some(encrypted) = input.channel_config_encrypted {
            app.channel_config_encrypted = Some(encrypted);
        }
        if let Some(status) = input.status {
            app.status = status;
        }
        input.published_at.apply(&mut app.published_at);
        app.updated_at = Self::now();
        Ok(Some(app.clone()))
    }

    pub async fn delete_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut apps = self.apps.write();
        let Some(app) = apps.get_mut(&id) else {
            return Ok(false);
        };
        if app.org_id != org_id || !matches!(app.status.as_str(), "draft" | "published") {
            return Ok(false);
        }
        app.status = "archived".to_string();
        app.archived_at = Some(Self::now());
        app.updated_at = Self::now();
        Ok(true)
    }

    pub async fn destroy_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut apps = self.apps.write();
        let Some(app) = apps.get_mut(&id) else {
            return Ok(false);
        };
        if app.org_id != org_id || app.status != "archived" {
            return Ok(false);
        }
        app.status = "deleted".to_string();
        app.deleted_at = Some(Self::now());
        app.updated_at = Self::now();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::common::Pagination;
    use everruns_core::DEFAULT_ORG_ID;

    /// Default pagination for tests (large enough to not truncate).
    fn default_pagination() -> Pagination {
        Pagination::new(0, 1000)
    }

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
                    initial_files: serde_json::json!([]),
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
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: Some(agent.id),
                agent_identity_id: None,
                title: Some("Test Session".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();

        let pagination = crate::api::common::Pagination::new(0, 20);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
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
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        // Create session - updated_at should equal created_at
        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: Some(agent.id),
                agent_identity_id: None,
                title: Some("Test Session".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                hints: None,
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
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: Some(agent.id),
                agent_identity_id: None,
                title: None,
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                hints: None,
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

        let events = db
            .list_events(session.id, None, None, &[], &[], None, None)
            .await
            .unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);
        assert_eq!(events[2].sequence, 3);
    }

    /// Helper: create an agent + session for event filter tests.
    async fn create_session_with_events(db: &InMemoryDatabase) -> SessionId {
        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: AgentId::new().to_string(),
                    name: "Filter Test Agent".to_string(),
                    description: None,
                    system_prompt: String::new(),
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: Some(agent.id),
                agent_identity_id: None,
                title: None,
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();

        // Create events of different types
        for event_type in [
            "input.message",
            "output.message.delta",
            "output.message.completed",
            "turn.started",
            "turn.completed",
            "reason.thinking.delta",
        ] {
            db.create_event(CreateEventRow {
                session_id: session.id,
                event_type: event_type.to_string(),
                ts: Utc::now(),
                context: serde_json::json!({}),
                data: serde_json::json!({}),
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
        }

        session.id
    }

    #[tokio::test]
    async fn test_count_events_no_materialization() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // Total count (6 events created by helper)
        let count = db.count_events(session_id, &[]).await.unwrap();
        assert_eq!(count, 6);

        // Count excluding delta types
        let count = db
            .count_events(
                session_id,
                &[
                    "output.message.delta".to_string(),
                    "reason.thinking.delta".to_string(),
                ],
            )
            .await
            .unwrap();
        assert_eq!(count, 4); // 6 - 2 delta types

        // Count for non-existent session
        let other_session = SessionId::new();
        let count = db.count_events(other_session, &[]).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_list_events_filter_types_positive() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // Positive filter: only turn events
        let events = db
            .list_events(
                session_id,
                None,
                None,
                &["turn.started".to_string(), "turn.completed".to_string()],
                &[],
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.event_type.starts_with("turn.")));
    }

    #[tokio::test]
    async fn test_list_events_filter_types_single() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // Positive filter: single type
        let events = db
            .list_events(
                session_id,
                None,
                None,
                &["input.message".to_string()],
                &[],
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "input.message");
    }

    #[tokio::test]
    async fn test_list_events_filter_types_empty_returns_all() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // Empty types = return all (6 events created)
        let events = db
            .list_events(session_id, None, None, &[], &[], None, None)
            .await
            .unwrap();

        assert_eq!(events.len(), 6);
    }

    #[tokio::test]
    async fn test_list_events_filter_types_no_match() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // Types that don't exist — empty result
        let events = db
            .list_events(
                session_id,
                None,
                None,
                &["nonexistent.type".to_string()],
                &[],
                None,
                None,
            )
            .await
            .unwrap();

        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_list_events_exclude_only() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // Exclude delta events (2 of 6)
        let events = db
            .list_events(
                session_id,
                None,
                None,
                &[],
                &[
                    "output.message.delta".to_string(),
                    "reason.thinking.delta".to_string(),
                ],
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(events.len(), 4);
        assert!(events.iter().all(|e| !e.event_type.contains("delta")));
    }

    #[tokio::test]
    async fn test_list_events_types_and_exclude_combined() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // types narrows to turn.* + input.message (3 events),
        // then exclude removes turn.completed (1 event) → 2 events remain
        let events = db
            .list_events(
                session_id,
                None,
                None,
                &[
                    "turn.started".to_string(),
                    "turn.completed".to_string(),
                    "input.message".to_string(),
                ],
                &["turn.completed".to_string()],
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(events.len(), 2);
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(types.contains(&"turn.started"));
        assert!(types.contains(&"input.message"));
        assert!(!types.contains(&"turn.completed"));
    }

    #[tokio::test]
    async fn test_list_events_types_fully_excluded() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // types selects one event, exclude removes that same type → empty
        let events = db
            .list_events(
                session_id,
                None,
                None,
                &["input.message".to_string()],
                &["input.message".to_string()],
                None,
                None,
            )
            .await
            .unwrap();

        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_list_events_since_id_uses_sequence_ordering() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // Get all events to find the ID of the second event
        let all_events = db
            .list_events(session_id, None, None, &[], &[], None, None)
            .await
            .unwrap();
        assert_eq!(all_events.len(), 6);

        let second_event_id = all_events[1].id;
        let second_event_seq = all_events[1].sequence;

        // Using since_id should return events after that event's sequence
        let events_after_id = db
            .list_events(
                session_id,
                None,
                Some(second_event_id),
                &[],
                &[],
                None,
                None,
            )
            .await
            .unwrap();

        // Using since_sequence with the same sequence should return the same events
        let events_after_seq = db
            .list_events(
                session_id,
                Some(second_event_seq),
                None,
                &[],
                &[],
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(events_after_id.len(), events_after_seq.len());
        assert_eq!(events_after_id.len(), 4); // 6 total - 2 skipped = 4
        for (a, b) in events_after_id.iter().zip(events_after_seq.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.sequence, b.sequence);
        }

        // Results should be ordered by sequence
        for window in events_after_id.windows(2) {
            assert!(
                window[0].sequence < window[1].sequence,
                "events must be ordered by sequence"
            );
        }
    }

    #[tokio::test]
    async fn test_list_events_since_id_unknown_returns_empty() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // Using a since_id that doesn't exist should return no events
        let unknown_id = EventId::new();
        let events = db
            .list_events(session_id, None, Some(unknown_id), &[], &[], None, None)
            .await
            .unwrap();

        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_list_events_since_id_takes_precedence_over_since_sequence() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        let all_events = db
            .list_events(session_id, None, None, &[], &[], None, None)
            .await
            .unwrap();

        // Provide since_id of 4th event but since_sequence of 1st event.
        // since_id should take precedence (return events after 4th).
        let fourth_event_id = all_events[3].id;
        let events = db
            .list_events(
                session_id,
                Some(all_events[0].sequence),
                Some(fourth_event_id),
                &[],
                &[],
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(events.len(), 2); // events 5 and 6
        assert_eq!(events[0].sequence, all_events[4].sequence);
        assert_eq!(events[1].sequence, all_events[5].sequence);
    }

    #[tokio::test]
    async fn test_list_events_with_limit() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // 6 events total. limit=3 should return last 3.
        let events = db
            .list_events(session_id, None, None, &[], &[], None, Some(3))
            .await
            .unwrap();
        assert_eq!(events.len(), 3);
        // Should be the last 3 events in sequence order
        assert!(events[0].sequence < events[1].sequence);
        assert!(events[1].sequence < events[2].sequence);
    }

    #[tokio::test]
    async fn test_list_events_with_limit_and_before_sequence() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        let all = db
            .list_events(session_id, None, None, &[], &[], None, None)
            .await
            .unwrap();
        assert_eq!(all.len(), 6);

        // Get last 2 events before the 5th event's sequence
        let fifth_seq = all[4].sequence;
        let events = db
            .list_events(session_id, None, None, &[], &[], Some(fifth_seq), Some(2))
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        // Should be events 3 and 4 (0-indexed)
        assert_eq!(events[0].id, all[2].id);
        assert_eq!(events[1].id, all[3].id);
    }

    #[tokio::test]
    async fn test_list_events_limit_greater_than_total() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // limit=1000 but only 6 events exist — returns all 6
        let events = db
            .list_events(session_id, None, None, &[], &[], None, Some(1000))
            .await
            .unwrap();
        assert_eq!(events.len(), 6);
    }

    #[tokio::test]
    async fn test_list_events_limit_with_exclude() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // 6 events, 2 are deltas. Exclude deltas + limit=2 → last 2 non-delta events
        let events = db
            .list_events(
                session_id,
                None,
                None,
                &[],
                &[
                    "output.message.delta".to_string(),
                    "reason.thinking.delta".to_string(),
                ],
                None,
                Some(2),
            )
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| !e.event_type.contains("delta")));
    }

    #[tokio::test]
    async fn test_count_non_delta_events() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        // 6 events total, 2 are deltas → 4 non-delta
        let delta_types = vec![
            "output.message.delta".to_string(),
            "reason.thinking.delta".to_string(),
        ];
        let count = db.count_events(session_id, &delta_types).await.unwrap();
        assert_eq!(count, 4);
    }

    #[tokio::test]
    async fn test_find_turn_boundary() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_events(&db).await;

        let all = db
            .list_events(session_id, None, None, &[], &[], None, None)
            .await
            .unwrap();

        // Find the turn.started event
        let turn_started = all.iter().find(|e| e.event_type == "turn.started").unwrap();

        // Searching at or after the turn.started sequence should find it
        let boundary = db
            .find_turn_boundary(session_id, turn_started.sequence + 1)
            .await
            .unwrap();
        assert_eq!(boundary, Some(turn_started.sequence));

        // Searching before the turn.started sequence should find nothing
        // (if turn.started is the first turn event)
        let boundary = db
            .find_turn_boundary(session_id, turn_started.sequence - 1)
            .await
            .unwrap();
        assert!(boundary.is_none());
    }

    #[tokio::test]
    async fn test_list_events_empty_session_with_limit() {
        let db = InMemoryDatabase::new();
        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: AgentId::new().to_string(),
                    name: "Empty Agent".to_string(),
                    description: None,
                    system_prompt: String::new(),
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: Some(agent.id),
                agent_identity_id: None,
                title: None,
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();

        // Empty session with limit should return empty
        let events = db
            .list_events(session.id, None, None, &[], &[], None, Some(200))
            .await
            .unwrap();
        assert!(events.is_empty());
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
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        // Create 15 sessions
        for i in 0..15 {
            db.create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: Some(agent.id),
                agent_identity_id: None,
                title: Some(format!("Session {}", i)),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();
        }

        // Test default pagination (all sessions fit within limit)
        let pagination = crate::api::common::Pagination::new(0, 20);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
            .await
            .unwrap();
        assert_eq!(total, 15);
        assert_eq!(sessions.len(), 15);

        // Test with limit=5
        let pagination = crate::api::common::Pagination::new(0, 5);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
            .await
            .unwrap();
        assert_eq!(total, 15);
        assert_eq!(sessions.len(), 5);

        // Test with offset=5, limit=5
        let pagination = crate::api::common::Pagination::new(5, 5);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
            .await
            .unwrap();
        assert_eq!(total, 15);
        assert_eq!(sessions.len(), 5);

        // Test last partial page (offset=10, limit=10 should return 5)
        let pagination = crate::api::common::Pagination::new(10, 10);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
            .await
            .unwrap();
        assert_eq!(total, 15);
        assert_eq!(sessions.len(), 5);

        // Test beyond range (offset=20)
        let pagination = crate::api::common::Pagination::new(20, 10);
        let (sessions, total) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
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
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        // Create sessions with sequential titles
        for i in 1..=5 {
            db.create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: Some(agent.id),
                agent_identity_id: None,
                title: Some(format!("Session {}", i)),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();
            // Small delay to ensure different created_at timestamps
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // Sessions should be ordered by created_at DESC (newest first)
        let pagination = crate::api::common::Pagination::new(0, 10);
        let (sessions, _) = db
            .list_sessions(DEFAULT_ORG_ID, Some(agent.id), None, pagination)
            .await
            .unwrap();

        assert_eq!(sessions.len(), 5);
        // Most recent session should be first
        assert_eq!(sessions[0].title, Some("Session 5".to_string()));
        assert_eq!(sessions[4].title, Some("Session 1".to_string()));
    }

    // TM-TENANT-008: Verify org-scoped user listing prevents cross-tenant enumeration
    #[tokio::test]
    async fn test_list_users_by_org_isolation() {
        let db = InMemoryDatabase::new();

        // Create two orgs
        let org1 = db
            .create_organization(CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000010".to_string(),
                name: "Org 1".to_string(),
                created_by: None,
            })
            .await
            .unwrap();
        let org2 = db
            .create_organization(CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000020".to_string(),
                name: "Org 2".to_string(),
                created_by: None,
            })
            .await
            .unwrap();

        // Create three users
        let user1 = db
            .create_user(CreateUserRow {
                email: "alice@example.com".to_string(),
                name: "Alice".to_string(),
                avatar_url: None,
                roles: vec!["user".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap();
        let user2 = db
            .create_user(CreateUserRow {
                email: "bob@example.com".to_string(),
                name: "Bob".to_string(),
                avatar_url: None,
                roles: vec!["user".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap();
        let _user3 = db
            .create_user(CreateUserRow {
                email: "charlie@example.com".to_string(),
                name: "Charlie".to_string(),
                avatar_url: None,
                roles: vec!["user".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap();

        // Add users to different orgs
        db.add_organization_member(org1.org_id, user1.id, "member")
            .await
            .unwrap();
        db.add_organization_member(org1.org_id, user2.id, "member")
            .await
            .unwrap();
        db.add_organization_member(org2.org_id, user2.id, "member")
            .await
            .unwrap();
        // user3 not in any of these orgs

        // Org1 should see alice + bob
        let org1_users = db.list_users_by_org(org1.org_id, None).await.unwrap();
        assert_eq!(org1_users.len(), 2);
        let org1_emails: Vec<_> = org1_users.iter().map(|u| u.email.as_str()).collect();
        assert!(org1_emails.contains(&"alice@example.com"));
        assert!(org1_emails.contains(&"bob@example.com"));

        // Org2 should see only bob
        let org2_users = db.list_users_by_org(org2.org_id, None).await.unwrap();
        assert_eq!(org2_users.len(), 1);
        assert_eq!(org2_users[0].email, "bob@example.com");

        // Charlie should not appear in either org
        assert!(!org1_emails.contains(&"charlie@example.com"));

        // Search within org should filter
        let search_results = db
            .list_users_by_org(org1.org_id, Some("alice"))
            .await
            .unwrap();
        assert_eq!(search_results.len(), 1);
        assert_eq!(search_results[0].email, "alice@example.com");

        // Search in org2 for alice should return nothing (alice not in org2)
        let cross_org_search = db
            .list_users_by_org(org2.org_id, Some("alice"))
            .await
            .unwrap();
        assert_eq!(cross_org_search.len(), 0);
    }

    #[tokio::test]
    async fn test_audit_log_create_and_list() {
        let db = InMemoryDatabase::new();

        let user_id = Uuid::now_v7();
        db.create_audit_log(CreateAuditLogRow {
            org_id: DEFAULT_ORG_ID,
            actor_id: Some(user_id),
            event_type: "auth.login.success".to_string(),
            ip_address: Some("1.2.3.4".to_string()),
            metadata: serde_json::json!({"method": "password"}),
        })
        .await
        .unwrap();

        db.create_audit_log(CreateAuditLogRow {
            org_id: DEFAULT_ORG_ID,
            actor_id: None,
            event_type: "auth.login.failure".to_string(),
            ip_address: Some("5.6.7.8".to_string()),
            metadata: serde_json::json!({"reason": "invalid_password"}),
        })
        .await
        .unwrap();

        // List all
        let logs = db
            .list_audit_logs(DEFAULT_ORG_ID, 50, None, None, None)
            .await
            .unwrap();
        assert_eq!(logs.len(), 2);
        // Newest first
        assert_eq!(logs[0].event_type, "auth.login.failure");

        // Filter by event type prefix
        let success_only = db
            .list_audit_logs(DEFAULT_ORG_ID, 50, None, Some("auth.login.success"), None)
            .await
            .unwrap();
        assert_eq!(success_only.len(), 1);
        assert_eq!(success_only[0].event_type, "auth.login.success");

        // Filter by actor
        let actor_logs = db
            .list_audit_logs(DEFAULT_ORG_ID, 50, None, None, Some(user_id))
            .await
            .unwrap();
        assert_eq!(actor_logs.len(), 1);
        assert_eq!(actor_logs[0].actor_id, Some(user_id));

        // Limit
        let limited = db
            .list_audit_logs(DEFAULT_ORG_ID, 1, None, None, None)
            .await
            .unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[tokio::test]
    async fn test_audit_log_org_isolation() {
        let db = InMemoryDatabase::new();

        let org2 = db
            .create_organization(CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000099".to_string(),
                name: "Other Org".to_string(),
                created_by: None,
            })
            .await
            .unwrap();

        db.create_audit_log(CreateAuditLogRow {
            org_id: DEFAULT_ORG_ID,
            actor_id: None,
            event_type: "auth.login.success".to_string(),
            ip_address: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

        db.create_audit_log(CreateAuditLogRow {
            org_id: org2.org_id,
            actor_id: None,
            event_type: "auth.login.failure".to_string(),
            ip_address: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

        let org1_logs = db
            .list_audit_logs(DEFAULT_ORG_ID, 50, None, None, None)
            .await
            .unwrap();
        assert_eq!(org1_logs.len(), 1);
        assert_eq!(org1_logs[0].event_type, "auth.login.success");

        let org2_logs = db
            .list_audit_logs(org2.org_id, 50, None, None, None)
            .await
            .unwrap();
        assert_eq!(org2_logs.len(), 1);
        assert_eq!(org2_logs[0].event_type, "auth.login.failure");
    }

    #[tokio::test]
    async fn test_audit_log_retention_delete() {
        let db = InMemoryDatabase::new();

        db.create_audit_log(CreateAuditLogRow {
            org_id: DEFAULT_ORG_ID,
            actor_id: None,
            event_type: "auth.login.success".to_string(),
            ip_address: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

        // Delete logs before future timestamp → removes all
        let deleted = db
            .delete_audit_logs_before(Utc::now() + chrono::Duration::hours(1))
            .await
            .unwrap();
        assert_eq!(deleted, 1);

        let logs = db
            .list_audit_logs(DEFAULT_ORG_ID, 50, None, None, None)
            .await
            .unwrap();
        assert!(logs.is_empty());
    }

    // ─── Search / command-palette tests ───

    /// Helper: create an agent with given name + description
    async fn create_test_agent(
        db: &InMemoryDatabase,
        name: &str,
        description: Option<&str>,
    ) -> AgentRow {
        db.create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: name.to_string(),
                description: description.map(|d| d.to_string()),
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_search_agents_no_filter_returns_all() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Alpha", None).await;
        create_test_agent(&db, "Beta", None).await;

        let (results, _total) = db
            .list_agents(DEFAULT_ORG_ID, None, false, default_pagination())
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_search_agents_empty_string_returns_all() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Alpha", None).await;

        let (results, _total) = db
            .list_agents(DEFAULT_ORG_ID, Some(""), false, default_pagination())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_agents_single_word_match() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Customer Support Bot", None).await;
        create_test_agent(&db, "Code Reviewer", None).await;

        let (results, _total) = db
            .list_agents(
                DEFAULT_ORG_ID,
                Some("customer"),
                false,
                default_pagination(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Customer Support Bot");
    }

    #[tokio::test]
    async fn test_search_agents_case_insensitive() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Customer Support Bot", None).await;

        let (results, _total) = db
            .list_agents(
                DEFAULT_ORG_ID,
                Some("CUSTOMER"),
                false,
                default_pagination(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_agents_multi_word_all_must_match() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Customer Support Bot", None).await;
        create_test_agent(&db, "Customer Feedback Analyzer", None).await;

        let (results, _total) = db
            .list_agents(
                DEFAULT_ORG_ID,
                Some("customer bot"),
                false,
                default_pagination(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Customer Support Bot");
    }

    #[tokio::test]
    async fn test_search_agents_matches_description() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Helper", Some("Handles billing inquiries")).await;

        let (results, _total) = db
            .list_agents(DEFAULT_ORG_ID, Some("billing"), false, default_pagination())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Helper");
    }

    #[tokio::test]
    async fn test_search_agents_cross_field_match() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Daytona Coder", Some("cloud sandbox agent")).await;

        // "daytona" in name, "sandbox" in description → both must match
        let (results, _total) = db
            .list_agents(
                DEFAULT_ORG_ID,
                Some("daytona sandbox"),
                false,
                default_pagination(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_agents_no_match() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Customer Support Bot", None).await;

        let (results, _total) = db
            .list_agents(
                DEFAULT_ORG_ID,
                Some("zzz_nonexistent"),
                false,
                default_pagination(),
            )
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_agents_poem_does_not_crash() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Alpha", None).await;

        // A full poem pasted into search — should not crash or hang
        let poem = "Roses are red, violets are blue, \
                    sugar is sweet, and so are you. \
                    The sky is wide, the ocean deep, \
                    these memories I shall forever keep. \
                    Through winding roads and starlit nights, \
                    we chase our dreams to greater heights.";
        let (results, _total) = db
            .list_agents(DEFAULT_ORG_ID, Some(poem), false, default_pagination())
            .await
            .unwrap();
        // No agent should match a poem
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_agents_poem_token_cap() {
        let db = InMemoryDatabase::new();
        // Agent whose name contains many words from the poem
        create_test_agent(&db, "roses are red violets are blue sugar is sweet", None).await;

        // Query with >MAX_SEARCH_TOKENS words — only first 8 tokens used
        let long_query = "roses are red violets are blue sugar is sweet and so are you forever";
        let (results, _total) = db
            .list_agents(
                DEFAULT_ORG_ID,
                Some(long_query),
                false,
                default_pagination(),
            )
            .await
            .unwrap();
        // First 8 tokens: "roses are red violets are blue sugar is"
        // All present in agent name → should match
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_agents_special_characters() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Agent v2.0 (beta)", None).await;
        create_test_agent(&db, "my-agent_v1", None).await;

        let (results, _total) = db
            .list_agents(DEFAULT_ORG_ID, Some("v2.0"), false, default_pagination())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Agent v2.0 (beta)");
    }

    #[tokio::test]
    async fn test_search_agents_unicode() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "日本語エージェント", Some("テスト用")).await;
        create_test_agent(&db, "English Agent", None).await;

        let (results, _total) = db
            .list_agents(DEFAULT_ORG_ID, Some("日本語"), false, default_pagination())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "日本語エージェント");
    }

    #[tokio::test]
    async fn test_search_agents_emoji() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "🤖 Robot Helper", None).await;
        create_test_agent(&db, "Normal Agent", None).await;

        let (results, _total) = db
            .list_agents(DEFAULT_ORG_ID, Some("🤖"), false, default_pagination())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_agents_whitespace_normalization() {
        let db = InMemoryDatabase::new();
        create_test_agent(&db, "Customer Support Bot", None).await;

        // Extra spaces, tabs, etc.
        let (results, _total) = db
            .list_agents(
                DEFAULT_ORG_ID,
                Some("  customer   bot  "),
                false,
                default_pagination(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_sessions_by_title() {
        let db = InMemoryDatabase::new();
        let agent = create_test_agent(&db, "Agent", None).await;

        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: Some("Debug production memory leak".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            hints: None,
        })
        .await
        .unwrap();

        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_identity_id: None,
            title: Some("Refactor auth module".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            hints: None,
        })
        .await
        .unwrap();

        let pagination = crate::api::common::Pagination::new(0, 20);
        let (results, total) = db
            .list_sessions(DEFAULT_ORG_ID, None, Some("memory leak"), pagination)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(results.len(), 1);
        assert!(results[0].title.as_ref().unwrap().contains("memory leak"));
    }

    #[tokio::test]
    async fn test_search_sessions_with_agent_filter() {
        let db = InMemoryDatabase::new();
        let agent1 = create_test_agent(&db, "Agent1", None).await;
        let agent2 = create_test_agent(&db, "Agent2", None).await;

        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent1.id),
            agent_identity_id: None,
            title: Some("Shared keyword session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            hints: None,
        })
        .await
        .unwrap();

        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: Some(agent2.id),
            agent_identity_id: None,
            title: Some("Shared keyword session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            hints: None,
        })
        .await
        .unwrap();

        let pagination = crate::api::common::Pagination::new(0, 20);
        // Search + agent filter combined
        let (results, total) = db
            .list_sessions(
                DEFAULT_ORG_ID,
                Some(agent1.id),
                Some("shared keyword"),
                pagination,
            )
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_sessions_poem_input() {
        let db = InMemoryDatabase::new();

        let pagination = crate::api::common::Pagination::new(0, 20);
        let poem = "Shall I compare thee to a summer's day? \
                    Thou art more lovely and more temperate. \
                    Rough winds do shake the darling buds of May, \
                    And summer's lease hath all too short a date.";
        let (results, total) = db
            .list_sessions(DEFAULT_ORG_ID, None, Some(poem), pagination)
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_matches_search_tokens_unit() {
        // Direct unit tests on the helper function
        assert!(matches_search_tokens(None, &["anything"]));
        assert!(matches_search_tokens(Some(""), &["anything"]));
        assert!(matches_search_tokens(Some("  "), &["anything"]));
        assert!(matches_search_tokens(Some("hello"), &["hello world"]));
        assert!(matches_search_tokens(
            Some("hello world"),
            &["hello", "world"]
        ));
        assert!(!matches_search_tokens(Some("missing"), &["hello world"]));
        // Multi-word: all tokens must match
        assert!(!matches_search_tokens(
            Some("hello missing"),
            &["hello world"]
        ));
        // Case insensitive
        assert!(matches_search_tokens(Some("HELLO"), &["hello"]));
    }

    #[tokio::test]
    async fn test_search_skills() {
        let db = InMemoryDatabase::new();

        db.create_skill(
            DEFAULT_ORG_ID,
            CreateSkillRow {
                public_id: SkillId::new().to_string(),
                name: "Web Scraper".to_string(),
                description: "Scrapes web pages for data".to_string(),
                license: Some("MIT".to_string()),
                compatibility: Some("*".to_string()),
                metadata: serde_json::json!({}),
                allowed_tools: None,
                instructions: "scrape it".to_string(),
                source_type: "inline".to_string(),
                archive_data: None,
                version: "1.0.0".to_string(),
            },
        )
        .await
        .unwrap();

        db.create_skill(
            DEFAULT_ORG_ID,
            CreateSkillRow {
                public_id: SkillId::new().to_string(),
                name: "Code Formatter".to_string(),
                description: "Formats code using prettier".to_string(),
                license: Some("MIT".to_string()),
                compatibility: Some("*".to_string()),
                metadata: serde_json::json!({}),
                allowed_tools: None,
                instructions: "format it".to_string(),
                source_type: "inline".to_string(),
                archive_data: None,
                version: "1.0.0".to_string(),
            },
        )
        .await
        .unwrap();

        let results = db
            .list_skills(DEFAULT_ORG_ID, Some("scraper"), false)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Web Scraper");

        // Cross-field: "code prettier"
        let results = db
            .list_skills(DEFAULT_ORG_ID, Some("code prettier"), false)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Code Formatter");
    }

    #[tokio::test]
    async fn test_search_apps() {
        let db = InMemoryDatabase::new();
        let agent = create_test_agent(&db, "Agent", None).await;
        let harness_id = Uuid::now_v7();

        db.create_app(
            DEFAULT_ORG_ID,
            CreateAppRow {
                public_id: "app_test1".to_string(),
                name: "Slack Bot".to_string(),
                description: Some("Slack integration for support".to_string()),
                harness_id,
                agent_id: agent.id.into(),
                agent_identity_id: None,
                channel_type: "slack".to_string(),
                channel_config: serde_json::json!({}),
                channel_config_encrypted: None,
            },
        )
        .await
        .unwrap();

        db.create_app(
            DEFAULT_ORG_ID,
            CreateAppRow {
                public_id: "app_test2".to_string(),
                name: "Web Widget".to_string(),
                description: Some("Embeddable chat widget".to_string()),
                harness_id,
                agent_id: agent.id.into(),
                agent_identity_id: None,
                channel_type: "web".to_string(),
                channel_config: serde_json::json!({}),
                channel_config_encrypted: None,
            },
        )
        .await
        .unwrap();

        let results = db
            .list_apps(DEFAULT_ORG_ID, Some("slack"), false)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Slack Bot");

        // Multi-word across name + description
        let results = db
            .list_apps(DEFAULT_ORG_ID, Some("widget chat"), false)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Web Widget");
    }

    // ─── MessageFilter::Search tests (EVE-87) ───

    /// Helper: create a session with events containing specific content.
    async fn create_session_with_content_events(db: &InMemoryDatabase) -> SessionId {
        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: AgentId::new().to_string(),
                    name: "Search Test Agent".to_string(),
                    description: None,
                    system_prompt: String::new(),
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                },
            )
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: Some(agent.id),
                agent_identity_id: None,
                title: None,
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();

        // Create events with different content
        let events_data = vec![
            (
                "input.message",
                serde_json::json!({"content": "Hello, how are you?"}),
            ),
            (
                "output.message.completed",
                serde_json::json!({"content": "I am doing great, thank you!"}),
            ),
            (
                "input.message",
                serde_json::json!({"content": "Tell me about Rust programming"}),
            ),
            (
                "tool.completed",
                serde_json::json!({"tool_name": "search", "content": "Rust is a systems language"}),
            ),
            (
                "output.message.completed",
                serde_json::json!({"content": "Here is information about Rust"}),
            ),
            // Event with no content field
            ("turn.started", serde_json::json!({"turn_id": "abc123"})),
        ];

        for (event_type, data) in events_data {
            db.create_event(CreateEventRow {
                session_id: session.id,
                event_type: event_type.to_string(),
                ts: Utc::now(),
                context: serde_json::json!({}),
                data,
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
        }

        session.id
    }

    #[tokio::test]
    async fn test_search_filter_matches_content_field() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_content_events(&db).await;

        let query =
            MessageQuery::new(session_id).with_filter(MessageFilter::Search("Rust".to_string()));

        let events = db.list_message_events_filtered(&query).await.unwrap();
        // Should match: "Tell me about Rust programming", "Rust is a systems language",
        // "Here is information about Rust"
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn test_search_filter_case_insensitive() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_content_events(&db).await;

        let query =
            MessageQuery::new(session_id).with_filter(MessageFilter::Search("rust".to_string()));

        let events = db.list_message_events_filtered(&query).await.unwrap();
        assert_eq!(events.len(), 3);

        // Also uppercase
        let query =
            MessageQuery::new(session_id).with_filter(MessageFilter::Search("RUST".to_string()));

        let events = db.list_message_events_filtered(&query).await.unwrap();
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn test_search_filter_no_match() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_content_events(&db).await;

        let query = MessageQuery::new(session_id)
            .with_filter(MessageFilter::Search("nonexistent_xyz".to_string()));

        let events = db.list_message_events_filtered(&query).await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_search_filter_skips_events_without_content() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_content_events(&db).await;

        // "abc123" is in turn_id, not content — should not match
        let query =
            MessageQuery::new(session_id).with_filter(MessageFilter::Search("abc123".to_string()));

        let events = db.list_message_events_filtered(&query).await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_search_filter_partial_match() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_content_events(&db).await;

        let query =
            MessageQuery::new(session_id).with_filter(MessageFilter::Search("great".to_string()));

        let events = db.list_message_events_filtered(&query).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "output.message.completed");
    }

    #[tokio::test]
    async fn test_search_filter_combined_with_event_type() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_content_events(&db).await;

        // Search for "Rust" but only in input.message events
        let query = MessageQuery::new(session_id)
            .with_filter(MessageFilter::EventTypes(vec!["input.message".to_string()]))
            .with_filter(MessageFilter::Search("Rust".to_string()));

        let events = db.list_message_events_filtered(&query).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "input.message");
    }

    #[tokio::test]
    async fn test_list_sessions_waiting_tool_results_before() {
        let db = InMemoryDatabase::new();
        let now = Utc::now();
        let cutoff = now - chrono::Duration::minutes(5);

        // Create 3 sessions: one waiting+old, one waiting+recent, one active+old
        let s1 = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: None,
                agent_identity_id: None,
                title: None,
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!({}),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();
        let s2 = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: None,
                agent_identity_id: None,
                title: None,
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!({}),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();
        let s3 = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: None,
                agent_id: None,
                agent_identity_id: None,
                title: None,
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!({}),
                tools: serde_json::json!([]),
                hints: None,
            })
            .await
            .unwrap();

        // Manually set statuses and updated_at
        {
            let mut sessions = db.sessions.write();
            if let Some(s) = sessions.get_mut(&s1.id) {
                s.status = "waiting_for_tool_results".to_string();
                s.updated_at = now - chrono::Duration::minutes(10); // old, should match
            }
            if let Some(s) = sessions.get_mut(&s2.id) {
                s.status = "waiting_for_tool_results".to_string();
                s.updated_at = now - chrono::Duration::minutes(1); // recent, should NOT match
            }
            if let Some(s) = sessions.get_mut(&s3.id) {
                s.status = "active".to_string();
                s.updated_at = now - chrono::Duration::minutes(10); // old but active, should NOT match
            }
        }

        let result = db
            .list_sessions_waiting_tool_results_before(cutoff)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, s1.id);
        assert_eq!(result[0].1, DEFAULT_ORG_ID);
    }

    #[tokio::test]
    async fn test_search_filter_empty_string_matches_all() {
        let db = InMemoryDatabase::new();
        let session_id = create_session_with_content_events(&db).await;

        // Empty search string should match events that have content field
        // (empty string is contained in any string)
        let query = MessageQuery::new(session_id).with_filter(MessageFilter::Search(String::new()));

        let events = db.list_message_events_filtered(&query).await.unwrap();
        // All default message-type events have content, so all should match
        // Default types: input.message, output.message.completed, tool.completed
        assert_eq!(events.len(), 5);
    }
}
