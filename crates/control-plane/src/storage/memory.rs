// In-memory storage implementation for dev mode
// Decision: Use parking_lot for thread-safe access
// Decision: UUIDs generated via uuid v7 (time-ordered)
//
// This implementation provides a PostgreSQL-compatible API backed by in-memory
// HashMaps, allowing the control-plane to run without a database for development.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use std::collections::HashMap;
use uuid::Uuid;

use super::models::*;

/// In-memory database for dev mode
/// All data is stored in memory and lost on restart
#[derive(Default)]
pub struct InMemoryDatabase {
    users: RwLock<HashMap<Uuid, UserRow>>,
    api_keys: RwLock<HashMap<Uuid, ApiKeyRow>>,
    refresh_tokens: RwLock<HashMap<Uuid, RefreshTokenRow>>,
    agents: RwLock<HashMap<Uuid, AgentRow>>,
    sessions: RwLock<HashMap<Uuid, SessionRow>>,
    events: RwLock<HashMap<Uuid, EventRow>>,
    llm_providers: RwLock<HashMap<Uuid, LlmProviderRow>>,
    llm_models: RwLock<HashMap<Uuid, LlmModelRow>>,
    agent_capabilities: RwLock<HashMap<(Uuid, String), AgentCapabilityRow>>,
    session_files: RwLock<HashMap<Uuid, SessionFileRow>>,
    // Event sequence counter per session
    event_sequences: RwLock<HashMap<Uuid, i32>>,
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
        if let Some(key) = keys.get(&id) {
            if key.user_id == user_id {
                keys.remove(&id);
                return Ok(true);
            }
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

    pub async fn create_agent(&self, input: CreateAgentRow) -> Result<AgentRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = AgentRow {
            id,
            name: input.name,
            description: input.description,
            system_prompt: input.system_prompt,
            default_model_id: input.default_model_id,
            tags: input.tags,
            status: "active".to_string(), // Default status for new agents
            created_at: now,
            updated_at: now,
        };
        self.agents.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_agent(&self, id: Uuid) -> Result<Option<AgentRow>> {
        Ok(self.agents.read().get(&id).cloned())
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentRow>> {
        let agents = self.agents.read();
        let mut result: Vec<_> = agents.values().cloned().collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn update_agent(&self, id: Uuid, input: UpdateAgent) -> Result<Option<AgentRow>> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&id) {
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

    pub async fn delete_agent(&self, id: Uuid) -> Result<bool> {
        // Delete capabilities first
        {
            let mut caps = self.agent_capabilities.write();
            let to_remove: Vec<_> = caps.keys().filter(|(aid, _)| *aid == id).cloned().collect();
            for key in to_remove {
                caps.remove(&key);
            }
        }
        Ok(self.agents.write().remove(&id).is_some())
    }

    // ============================================
    // Sessions
    // ============================================

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = SessionRow {
            id,
            agent_id: input.agent_id,
            title: input.title,
            tags: input.tags,
            model_id: input.model_id,
            status: "pending".to_string(), // Default status for new sessions
            created_at: now,
            started_at: None,
            finished_at: None,
        };
        self.sessions.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_session(&self, id: Uuid) -> Result<Option<SessionRow>> {
        Ok(self.sessions.read().get(&id).cloned())
    }

    pub async fn list_sessions(&self, agent_id: Uuid) -> Result<Vec<SessionRow>> {
        let sessions = self.sessions.read();
        let mut result: Vec<_> = sessions
            .values()
            .filter(|s| s.agent_id == agent_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn update_session(
        &self,
        id: Uuid,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
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
            return Ok(Some(session.clone()));
        }
        Ok(None)
    }

    pub async fn delete_session(&self, id: Uuid) -> Result<bool> {
        // Delete events first
        {
            let mut events = self.events.write();
            let to_remove: Vec<Uuid> = events
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
        let id = Uuid::now_v7();

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
        session_id: Uuid,
        since_sequence: Option<i32>,
        since_id: Option<Uuid>,
    ) -> Result<Vec<EventRow>> {
        let events = self.events.read();
        let mut result: Vec<_> = events
            .values()
            .filter(|e| {
                if e.session_id != session_id {
                    return false;
                }
                // Prefer since_id (UUID v7 monotonically increasing) over sequence
                if let Some(id) = since_id {
                    if e.id <= id {
                        return false;
                    }
                } else if let Some(seq) = since_sequence {
                    if e.sequence <= seq {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        result.sort_by_key(|e| e.sequence);
        Ok(result)
    }

    pub async fn list_message_events(&self, session_id: Uuid) -> Result<Vec<EventRow>> {
        let message_types = [
            "message.user",
            "message.assistant",
            "message.tool_call",
            "message.tool_result",
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

    // ============================================
    // LLM Providers
    // ============================================

    pub async fn create_llm_provider(&self, input: CreateLlmProviderRow) -> Result<LlmProviderRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let api_key_set = input.api_key_encrypted.is_some();
        let row = LlmProviderRow {
            id,
            name: input.name,
            provider_type: input.provider_type,
            base_url: input.base_url,
            api_key_encrypted: input.api_key_encrypted,
            api_key_set,
            status: "active".to_string(), // Default status for new providers
            settings: input.settings.unwrap_or(serde_json::json!({})),
            created_at: now,
            updated_at: now,
        };
        self.llm_providers.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_llm_provider(&self, id: Uuid) -> Result<Option<LlmProviderRow>> {
        Ok(self.llm_providers.read().get(&id).cloned())
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
        // Delete models first
        {
            let mut models = self.llm_models.write();
            let to_remove: Vec<Uuid> = models
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

    pub async fn get_default_llm_model(&self) -> Result<Option<LlmModelWithProviderRow>> {
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        for model in models.values() {
            if model.is_default {
                if let Some(provider) = providers.get(&model.provider_id) {
                    return Ok(Some(LlmModelWithProviderRow {
                        id: model.id,
                        provider_id: model.provider_id,
                        model_id: model.model_id.clone(),
                        display_name: model.display_name.clone(),
                        capabilities: model.capabilities.clone(),
                        is_default: model.is_default,
                        status: model.status.clone(),
                        created_at: model.created_at,
                        updated_at: model.updated_at,
                        provider_name: provider.name.clone(),
                        provider_type: provider.provider_type.clone(),
                    }));
                }
            }
        }
        Ok(None)
    }

    pub async fn clear_all_model_defaults(&self) -> Result<()> {
        for model in self.llm_models.write().values_mut() {
            model.is_default = false;
        }
        Ok(())
    }

    pub async fn create_llm_model(&self, input: CreateLlmModelRow) -> Result<LlmModelRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = LlmModelRow {
            id,
            provider_id: input.provider_id,
            model_id: input.model_id,
            display_name: input.display_name,
            capabilities: serde_json::to_value(&input.capabilities)?,
            is_default: input.is_default,
            status: "active".to_string(), // Default status for new models
            created_at: now,
            updated_at: now,
        };
        self.llm_models.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_llm_model(&self, id: Uuid) -> Result<Option<LlmModelRow>> {
        Ok(self.llm_models.read().get(&id).cloned())
    }

    pub async fn get_llm_model_with_provider(
        &self,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        if let Some(model) = models.get(&id) {
            if let Some(provider) = providers.get(&model.provider_id) {
                return Ok(Some(LlmModelWithProviderRow {
                    id: model.id,
                    provider_id: model.provider_id,
                    model_id: model.model_id.clone(),
                    display_name: model.display_name.clone(),
                    capabilities: model.capabilities.clone(),
                    is_default: model.is_default,
                    status: model.status.clone(),
                    created_at: model.created_at,
                    updated_at: model.updated_at,
                    provider_name: provider.name.clone(),
                    provider_type: provider.provider_type.clone(),
                }));
            }
        }
        Ok(None)
    }

    pub async fn list_llm_models_for_provider(
        &self,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModelRow>> {
        let models = self.llm_models.read();
        let mut result: Vec<_> = models
            .values()
            .filter(|m| m.provider_id == provider_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        Ok(result)
    }

    pub async fn list_all_llm_models(&self) -> Result<Vec<LlmModelWithProviderRow>> {
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        let mut result: Vec<_> = models
            .values()
            .filter_map(|model| {
                providers
                    .get(&model.provider_id)
                    .map(|provider| LlmModelWithProviderRow {
                        id: model.id,
                        provider_id: model.provider_id,
                        model_id: model.model_id.clone(),
                        display_name: model.display_name.clone(),
                        capabilities: model.capabilities.clone(),
                        is_default: model.is_default,
                        status: model.status.clone(),
                        created_at: model.created_at,
                        updated_at: model.updated_at,
                        provider_name: provider.name.clone(),
                        provider_type: provider.provider_type.clone(),
                    })
            })
            .collect();
        result.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        Ok(result)
    }

    pub async fn update_llm_model(
        &self,
        id: Uuid,
        input: UpdateLlmModel,
    ) -> Result<Option<LlmModelRow>> {
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
            if let Some(status) = input.status {
                model.status = status;
            }
            model.updated_at = Self::now();
            return Ok(Some(model.clone()));
        }
        Ok(None)
    }

    pub async fn delete_llm_model(&self, id: Uuid) -> Result<bool> {
        Ok(self.llm_models.write().remove(&id).is_some())
    }

    pub async fn get_llm_model_by_model_id(
        &self,
        model_id: &str,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let models = self.llm_models.read();
        let providers = self.llm_providers.read();

        for model in models.values() {
            if model.model_id == model_id {
                if let Some(provider) = providers.get(&model.provider_id) {
                    return Ok(Some(LlmModelWithProviderRow {
                        id: model.id,
                        provider_id: model.provider_id,
                        model_id: model.model_id.clone(),
                        display_name: model.display_name.clone(),
                        capabilities: model.capabilities.clone(),
                        is_default: model.is_default,
                        status: model.status.clone(),
                        created_at: model.created_at,
                        updated_at: model.updated_at,
                        provider_name: provider.name.clone(),
                        provider_type: provider.provider_type.clone(),
                    }));
                }
            }
        }
        Ok(None)
    }

    // ============================================
    // Agent Capabilities
    // ============================================

    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
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
        capabilities: Vec<(String, i32)>,
    ) -> Result<Vec<AgentCapabilityRow>> {
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
        for (capability_id, position) in capabilities.into_iter() {
            let row = AgentCapabilityRow {
                id: Uuid::now_v7(),
                agent_id,
                capability_id: capability_id.clone(),
                position,
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
        let regex = regex::Regex::new(pattern)?;
        let files = self.session_files.read();

        let result: Vec<_> = files
            .values()
            .filter(|f| {
                if f.session_id != session_id || f.is_directory {
                    return false;
                }
                if let Some(prefix) = path_prefix {
                    if !f.path.starts_with(prefix) {
                        return false;
                    }
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
        let prefix = format!("{}/", path.trim_end_matches('/'));
        Ok(self
            .session_files
            .read()
            .values()
            .any(|f| f.session_id == session_id && f.path.starts_with(&prefix)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_and_get_agent() {
        let db = InMemoryDatabase::new();

        let agent = db
            .create_agent(CreateAgentRow {
                name: "Test Agent".to_string(),
                description: Some("A test agent".to_string()),
                system_prompt: "You are helpful".to_string(),
                default_model_id: None,
                tags: vec!["test".to_string()],
            })
            .await
            .unwrap();

        assert_eq!(agent.name, "Test Agent");

        let fetched = db.get_agent(agent.id).await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().name, "Test Agent");
    }

    #[tokio::test]
    async fn test_create_and_list_sessions() {
        let db = InMemoryDatabase::new();

        let agent = db
            .create_agent(CreateAgentRow {
                name: "Test Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                agent_id: agent.id,
                title: Some("Test Session".to_string()),
                tags: vec![],
                model_id: None,
            })
            .await
            .unwrap();

        let sessions = db.list_sessions(agent.id).await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, session.id);
    }

    #[tokio::test]
    async fn test_events_sequence() {
        use chrono::Utc;

        let db = InMemoryDatabase::new();

        let agent = db
            .create_agent(CreateAgentRow {
                name: "Test Agent".to_string(),
                description: None,
                system_prompt: String::new(),
                default_model_id: None,
                tags: vec![],
            })
            .await
            .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                agent_id: agent.id,
                title: None,
                tags: vec![],
                model_id: None,
            })
            .await
            .unwrap();

        // Create multiple events
        for i in 0..3 {
            db.create_event(CreateEventRow {
                session_id: session.id,
                event_type: "message.user".to_string(),
                ts: Utc::now(),
                context: serde_json::json!({}),
                data: serde_json::json!({"content": format!("Message {}", i)}),
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
        }

        let events = db.list_events(session.id, None, None).await.unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);
        assert_eq!(events[2].sequence, 3);
    }
}
