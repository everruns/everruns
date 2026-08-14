// In-memory storage implementation for dev mode
// Decision: Use parking_lot for thread-safe access
// Decision: UUIDs generated via uuid v7 (time-ordered)
//
// This implementation provides a PostgreSQL-compatible API backed by in-memory
// HashMaps, allowing the server to run without a database for development.
//
// Split into per-entity modules for maintainability (EVE-99).

mod agent_check_rules;
mod agent_health_checks;
mod agent_identities;
mod agent_identity_connections;
mod agent_mcp_secret_bindings;
mod agent_triggers;
mod agents;
mod app_channels;
mod apps;
mod audit_logs;
mod auth;
mod budgets;
mod compaction_checkpoints;
mod declarative_capabilities;
mod evals;
mod events;
mod harnesses;
mod knowledge_bases;
mod knowledge_indexes;
mod mcp_servers;
mod memories;
mod notifications;
mod observers;
mod org_feature_flags;
mod organizations;
mod payments;
mod plugins;
mod principals;
mod providers;
mod reporting;
mod schedules;
mod session_files;
mod session_git;
mod session_participants;
mod session_resources;
mod session_storage;
mod session_tasks;
mod sessions;
mod skills;
pub mod subagent_spawn_handles;
mod user_connections;
mod user_preferences;
mod users;
mod workspaces;

#[cfg(test)]
mod tests;

use crate::kernel_imports::{
    DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, everruns_provider::typed_id::AgentId,
    everruns_provider::typed_id::AgentIdentityId, everruns_provider::typed_id::AgentVersionId,
    everruns_provider::typed_id::EventId, everruns_provider::typed_id::HarnessId,
    everruns_provider::typed_id::ImageId, everruns_provider::typed_id::LeasedResourceId,
    everruns_provider::typed_id::McpServerId, everruns_provider::typed_id::MessageId,
    everruns_provider::typed_id::ModelId, everruns_provider::typed_id::NotificationId,
    everruns_provider::typed_id::PluginMarketplaceId, everruns_provider::typed_id::PrincipalId,
    everruns_provider::typed_id::ProviderId, everruns_provider::typed_id::ScheduleId,
    everruns_provider::typed_id::SessionId, everruns_provider::typed_id::SessionParticipantId,
    everruns_provider::typed_id::SkillId, everruns_provider::typed_id::TriggerId,
};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
type ReportingOutboxKey = (i64, String, String, String, String);

/// All data is stored in memory and lost on restart
pub struct InMemoryDatabase {
    // TODO: Used in Phase 3 when org APIs are implemented
    #[allow(dead_code)]
    organizations: RwLock<HashMap<i64, OrganizationRow>>,
    #[allow(dead_code)]
    organization_members: RwLock<HashMap<(i64, Uuid), OrganizationMemberRow>>,
    users: RwLock<HashMap<Uuid, UserRow>>,
    user_oauth_identities: RwLock<HashMap<(String, String), Uuid>>,
    personal_access_tokens: RwLock<HashMap<Uuid, PersonalAccessTokenRow>>,
    cli_auth_sessions: RwLock<HashMap<Uuid, CliAuthSessionRow>>,
    refresh_tokens: RwLock<HashMap<Uuid, RefreshTokenRow>>,
    agents: RwLock<HashMap<AgentId, AgentRow>>,
    agent_mcp_secret_bindings: RwLock<HashMap<Uuid, AgentMcpSecretBindingRow>>,
    agent_versions: RwLock<HashMap<AgentVersionId, AgentVersionRow>>,
    sessions: RwLock<HashMap<SessionId, SessionRow>>,
    session_participants: RwLock<HashMap<SessionParticipantId, SessionParticipantRow>>,
    events: RwLock<HashMap<EventId, EventRow>>,
    compaction_checkpoints:
        RwLock<HashMap<(SessionId, String, String, i32), CompactionCheckpointRow>>,
    providers: RwLock<HashMap<ProviderId, ProviderRow>>,
    models: RwLock<HashMap<ModelId, ModelRow>>,
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
    declarative_capabilities: RwLock<HashMap<Uuid, DeclarativeCapabilityRow>>,
    // Event sequence counter per session
    event_sequences: RwLock<HashMap<SessionId, i32>>,
    // Session storage
    session_key_values: RwLock<HashMap<(SessionId, String), SessionKeyValueRow>>,
    session_secrets: RwLock<HashMap<(SessionId, String), SessionSecretRow>>,
    // User connections (external service accounts)
    user_connections: RwLock<HashMap<Uuid, UserConnectionRow>>,
    // User preferences: (user_id, key) -> row
    user_preferences: RwLock<HashMap<(Uuid, String), UserPreferenceRow>>,
    // Pinned sessions: (user_id, session_id) -> (org_id, pinned_at)
    pinned_sessions: RwLock<HashMap<(Uuid, SessionId), PinnedSessionData>>,
    // Durable UI notifications
    notifications: RwLock<HashMap<NotificationId, NotificationRow>>,
    notification_turn_requests: RwLock<HashMap<MessageId, NotificationTurnRequestRow>>,
    // Session schedules
    session_schedules: RwLock<HashMap<ScheduleId, SessionScheduleRow>>,
    // Generic leased resources that require eventual cleanup.
    leased_resources: RwLock<HashMap<LeasedResourceId, LeasedResourceRow>>,
    // Session resource registry (generic active-resource tracking).
    session_resources: RwLock<HashMap<(SessionId, String), SessionResourceRow>>,
    // Session tasks (background work owned by a session), keyed by task id.
    session_tasks: RwLock<HashMap<String, SessionTaskRow>>,
    session_task_messages: RwLock<Vec<SessionTaskMessageRow>>,
    // Audit logs (TM-OBS-007)
    audit_logs: RwLock<Vec<AuditLogRow>>,
    // Apps (deployable agent+harness bundles)
    apps: RwLock<HashMap<Uuid, AppRow>>,
    // App channels (distribution channels per app)
    app_channels: RwLock<HashMap<Uuid, AppChannelRow>>,
    // Agent identities (virtual principals)
    agent_identities: RwLock<HashMap<AgentIdentityId, AgentIdentityRow>>,
    // Agent triggers (agent-owned invocation triggers)
    agent_triggers: RwLock<HashMap<TriggerId, AgentTriggerRow>>,
    principals: RwLock<HashMap<PrincipalId, PrincipalRow>>,
    // Agent identity connections (identity-scoped external accounts)
    agent_identity_connections: RwLock<HashMap<Uuid, AgentIdentityConnectionRow>>,
    // Organization settings (default model, etc.)
    org_settings: RwLock<HashMap<i64, OrganizationSettingsRow>>,
    org_feature_flags: RwLock<HashMap<i64, HashMap<String, bool>>>,
    // Evals (user-facing behavioral tests)
    evals: RwLock<HashMap<Uuid, EvalRow>>,
    eval_cases: RwLock<HashMap<Uuid, EvalCaseRow>>,
    eval_runs: RwLock<HashMap<Uuid, EvalRunRow>>,
    eval_case_results: RwLock<HashMap<Uuid, EvalCaseResultRow>>,
    eval_run_datasets: RwLock<HashMap<Uuid, EvalRunDatasetRow>>,
    eval_run_share_tokens: RwLock<HashMap<Uuid, EvalRunShareTokenRow>>,
    agent_health_check_runs: RwLock<HashMap<Uuid, AgentHealthCheckRunRow>>,
    agent_check_rules: RwLock<HashMap<Uuid, AgentCheckRuleRow>>,
    observers: RwLock<HashMap<Uuid, ObserverRow>>,
    trace_scores: RwLock<HashMap<Uuid, TraceScoreRow>>,
    // Budgets
    budgets: RwLock<HashMap<Uuid, BudgetRow>>,
    usage_journal: RwLock<HashMap<Uuid, UsageJournalRow>>,
    usage_ledger: RwLock<Vec<UsageLedgerRow>>,
    reporting_outbox:
        RwLock<HashMap<ReportingOutboxKey, crate::storage::reporting::models::ReportingOutboxRow>>,
    payment_accounts: RwLock<HashMap<Uuid, PaymentAccountRow>>,
    payment_policies: RwLock<HashMap<Uuid, PaymentPolicyRow>>,
    payment_attempts: RwLock<HashMap<Uuid, PaymentAttemptRow>>,
    // Memories (org-scoped named Memories — see knowledge/runtime-resources/memory.md)
    memories: RwLock<HashMap<Uuid, MemoryRow>>,
    memory_files: RwLock<HashMap<Uuid, MemoryFileRow>>,
    // Workspaces (org-scoped named working areas — see knowledge/runtime-resources/workspace.md)
    workspaces: RwLock<HashMap<Uuid, WorkspaceRow>>,
    // Knowledge bases (curated org knowledge)
    knowledge_bases: RwLock<HashMap<Uuid, KnowledgeBaseRow>>,
    knowledge_entries: RwLock<HashMap<Uuid, KnowledgeEntryRow>>,
    // Knowledge indexes (source-backed embedded collections — see knowledge/runtime-resources/knowledge-indexes.md)
    knowledge_indexes: RwLock<HashMap<Uuid, KnowledgeIndexRow>>,
    knowledge_index_documents: RwLock<HashMap<Uuid, KnowledgeIndexDocumentRow>>,
    knowledge_index_chunks: RwLock<HashMap<Uuid, KnowledgeIndexChunkRow>>,
    // OAuth clients (MCP OAuth 2.1)
    oauth_clients: RwLock<HashMap<Uuid, OAuthClientRow>>,
    oauth_authorization_codes: RwLock<HashMap<Uuid, OAuthAuthorizationCodeRow>>,
    oauth_refresh_tokens: RwLock<HashMap<Uuid, OAuthRefreshTokenRow>>,
    // Plugin marketplaces and installs (see knowledge/integrations/plugins.md)
    plugin_marketplaces: RwLock<HashMap<Uuid, PluginMarketplaceRow>>,
    plugin_installs: RwLock<HashMap<Uuid, PluginInstallRow>>,
    // Organization task webhooks (outbound HTTP on terminal task transitions)
    org_task_webhooks: RwLock<Vec<OrgTaskWebhookRow>>,
    // Per-task push-notification configs (EVE-682): session/task-scoped webhooks
    session_task_push_configs: RwLock<Vec<SessionTaskPushConfigRow>>,
    // OSS-owned organization invitations (EVE-602)
    org_invitations: RwLock<Vec<OrgInvitationRow>>,
    // Native-auth account-recovery tokens (hashed, single-use, short-TTL).
    password_reset_tokens: RwLock<Vec<auth::AccountRecoveryToken>>,
    email_verification_tokens: RwLock<Vec<auth::AccountRecoveryToken>>,
    #[cfg(test)]
    session_list_lookup_count: AtomicUsize,
    #[cfg(test)]
    session_list_lookup_delay_ms: AtomicU64,
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
                // Default org is pre-provisioned and already onboarded.
                onboarding_completed_at: Some(now),
            },
        );

        // Pre-seed default plugin marketplace for the default org.
        // Mirrors the 058_backfill_default_marketplace.sql migration and
        // org-creation seeding so in-memory dev mode starts in the same
        // state as a freshly migrated Postgres instance.
        let default_marketplace_id = Uuid::now_v7();
        let mut plugin_marketplaces = HashMap::new();
        plugin_marketplaces.insert(
            default_marketplace_id,
            PluginMarketplaceRow {
                id: default_marketplace_id,
                org_id: DEFAULT_ORG_ID,
                public_id: PluginMarketplaceId::new().to_string(),
                name: "everruns".to_string(),
                source_type: "github".to_string(),
                source: serde_json::json!({"repo": "everruns/everruns"}),
                status: "active".to_string(),
                catalog: None,
                last_synced_at: None,
                last_synced_sha: None,
                created_at: now,
                updated_at: now,
            },
        );

        Self {
            organizations: RwLock::new(organizations),
            organization_members: RwLock::new(HashMap::new()),
            users: RwLock::new(HashMap::new()),
            user_oauth_identities: RwLock::new(HashMap::new()),
            personal_access_tokens: RwLock::new(HashMap::new()),
            cli_auth_sessions: RwLock::new(HashMap::new()),
            refresh_tokens: RwLock::new(HashMap::new()),
            agents: RwLock::new(HashMap::new()),
            agent_mcp_secret_bindings: RwLock::new(HashMap::new()),
            agent_versions: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
            session_participants: RwLock::new(HashMap::new()),
            events: RwLock::new(HashMap::new()),
            compaction_checkpoints: RwLock::new(HashMap::new()),
            providers: RwLock::new(HashMap::new()),
            models: RwLock::new(HashMap::new()),
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
            declarative_capabilities: RwLock::new(HashMap::new()),
            event_sequences: RwLock::new(HashMap::new()),
            session_key_values: RwLock::new(HashMap::new()),
            session_secrets: RwLock::new(HashMap::new()),
            user_connections: RwLock::new(HashMap::new()),
            user_preferences: RwLock::new(HashMap::new()),
            pinned_sessions: RwLock::new(HashMap::new()),
            notifications: RwLock::new(HashMap::new()),
            notification_turn_requests: RwLock::new(HashMap::new()),
            session_schedules: RwLock::new(HashMap::new()),
            leased_resources: RwLock::new(HashMap::new()),
            session_resources: RwLock::new(HashMap::new()),
            session_tasks: RwLock::new(HashMap::new()),
            session_task_messages: RwLock::new(Vec::new()),
            audit_logs: RwLock::new(Vec::new()),
            apps: RwLock::new(HashMap::new()),
            app_channels: RwLock::new(HashMap::new()),
            agent_identities: RwLock::new(HashMap::new()),
            agent_triggers: RwLock::new(HashMap::new()),
            principals: RwLock::new(HashMap::new()),
            agent_identity_connections: RwLock::new(HashMap::new()),
            org_settings: RwLock::new(HashMap::new()),
            org_feature_flags: RwLock::new(HashMap::new()),
            evals: RwLock::new(HashMap::new()),
            eval_cases: RwLock::new(HashMap::new()),
            eval_runs: RwLock::new(HashMap::new()),
            eval_case_results: RwLock::new(HashMap::new()),
            eval_run_datasets: RwLock::new(HashMap::new()),
            eval_run_share_tokens: RwLock::new(HashMap::new()),
            agent_health_check_runs: RwLock::new(HashMap::new()),
            agent_check_rules: RwLock::new(HashMap::new()),
            observers: RwLock::new(HashMap::new()),
            trace_scores: RwLock::new(HashMap::new()),
            budgets: RwLock::new(HashMap::new()),
            usage_journal: RwLock::new(HashMap::new()),
            usage_ledger: RwLock::new(Vec::new()),
            reporting_outbox: RwLock::new(HashMap::new()),
            payment_accounts: RwLock::new(HashMap::new()),
            payment_policies: RwLock::new(HashMap::new()),
            payment_attempts: RwLock::new(HashMap::new()),
            memories: RwLock::new(HashMap::new()),
            memory_files: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(HashMap::new()),
            knowledge_bases: RwLock::new(HashMap::new()),
            knowledge_entries: RwLock::new(HashMap::new()),
            knowledge_indexes: RwLock::new(HashMap::new()),
            knowledge_index_documents: RwLock::new(HashMap::new()),
            knowledge_index_chunks: RwLock::new(HashMap::new()),
            oauth_clients: RwLock::new(HashMap::new()),
            oauth_authorization_codes: RwLock::new(HashMap::new()),
            oauth_refresh_tokens: RwLock::new(HashMap::new()),
            plugin_marketplaces: RwLock::new(plugin_marketplaces),
            plugin_installs: RwLock::new(HashMap::new()),
            org_task_webhooks: RwLock::new(Vec::new()),
            session_task_push_configs: RwLock::new(Vec::new()),
            org_invitations: RwLock::new(Vec::new()),
            password_reset_tokens: RwLock::new(Vec::new()),
            email_verification_tokens: RwLock::new(Vec::new()),
            #[cfg(test)]
            session_list_lookup_count: AtomicUsize::new(0),
            #[cfg(test)]
            session_list_lookup_delay_ms: AtomicU64::new(0),
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

    #[cfg(test)]
    pub(crate) fn record_session_list_lookup(&self) {
        self.session_list_lookup_count
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn reset_session_list_lookup_count(&self) {
        self.session_list_lookup_count.store(0, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn session_list_lookup_count(&self) -> usize {
        self.session_list_lookup_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn set_session_list_lookup_delay_ms(&self, delay_ms: u64) {
        self.session_list_lookup_delay_ms
            .store(delay_ms, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn session_list_lookup_delay_ms(&self) -> u64 {
        self.session_list_lookup_delay_ms.load(Ordering::Relaxed)
    }
}
