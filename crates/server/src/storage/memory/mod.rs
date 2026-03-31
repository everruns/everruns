// In-memory storage implementation for dev mode
// Decision: Use parking_lot for thread-safe access
// Decision: UUIDs generated via uuid v7 (time-ordered)
//
// This implementation provides a PostgreSQL-compatible API backed by in-memory
// HashMaps, allowing the server to run without a database for development.
//
// Split into per-entity modules for maintainability (EVE-99).

mod agent_identities;
mod agent_identity_connections;
mod agents;
mod app_channels;
mod apps;
mod audit_logs;
mod auth;
mod budgets;
mod evals;
mod events;
mod harnesses;
mod llm;
mod mcp_servers;
mod notifications;
mod organizations;
mod schedules;
mod session_files;
mod session_git;
mod session_storage;
mod sessions;
mod skills;
mod user_connections;
mod users;

#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};
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
pub(crate) fn matches_search_tokens(search: Option<&str>, texts: &[&str]) -> bool {
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
    // Session git objects: (session_id, oid) -> object row
    git_objects: RwLock<HashMap<(SessionId, Vec<u8>), SessionGitObjectRow>>,
    // Session git refs: (session_id, name) -> ref row
    git_refs: RwLock<HashMap<(SessionId, String), SessionGitRefRow>>,
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
    // App channels (distribution channels per app)
    app_channels: RwLock<HashMap<Uuid, AppChannelRow>>,
    // Agent identities (virtual principals)
    agent_identities: RwLock<HashMap<AgentIdentityId, AgentIdentityRow>>,
    // Agent identity connections (identity-scoped external accounts)
    agent_identity_connections: RwLock<HashMap<Uuid, AgentIdentityConnectionRow>>,
    // Organization settings (default model, etc.)
    org_settings: RwLock<HashMap<i64, OrganizationSettingsRow>>,
    // Evals (user-facing behavioral tests)
    evals: RwLock<HashMap<Uuid, EvalRow>>,
    eval_cases: RwLock<HashMap<Uuid, EvalCaseRow>>,
    eval_runs: RwLock<HashMap<Uuid, EvalRunRow>>,
    eval_case_results: RwLock<HashMap<Uuid, EvalCaseResultRow>>,
    // Budgets
    budgets: RwLock<HashMap<Uuid, BudgetRow>>,
    budget_ledger: RwLock<Vec<BudgetLedgerRow>>,
    // OAuth clients (MCP OAuth 2.1)
    oauth_clients: RwLock<HashMap<Uuid, OAuthClientRow>>,
    oauth_authorization_codes: RwLock<HashMap<Uuid, OAuthAuthorizationCodeRow>>,
    oauth_refresh_tokens: RwLock<HashMap<Uuid, OAuthRefreshTokenRow>>,
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
            git_objects: RwLock::new(HashMap::new()),
            git_refs: RwLock::new(HashMap::new()),
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
            app_channels: RwLock::new(HashMap::new()),
            agent_identities: RwLock::new(HashMap::new()),
            agent_identity_connections: RwLock::new(HashMap::new()),
            org_settings: RwLock::new(HashMap::new()),
            evals: RwLock::new(HashMap::new()),
            eval_cases: RwLock::new(HashMap::new()),
            eval_runs: RwLock::new(HashMap::new()),
            eval_case_results: RwLock::new(HashMap::new()),
            budgets: RwLock::new(HashMap::new()),
            budget_ledger: RwLock::new(Vec::new()),
            oauth_clients: RwLock::new(HashMap::new()),
            oauth_authorization_codes: RwLock::new(HashMap::new()),
            oauth_refresh_tokens: RwLock::new(HashMap::new()),
        }
    }
}

impl InMemoryDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn now() -> DateTime<Utc> {
        Utc::now()
    }
}
