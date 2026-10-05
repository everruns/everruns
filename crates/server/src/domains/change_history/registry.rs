// What each mutating command changes.
//
// Decision: one table, keyed by wire name, instead of a `change()` override in
// each of ~170 command impls. The question "which entity does this mutation
// touch, and how" is answered for every command in one reviewable place, the
// guard test below fails on any mutating command missing from it, and a new
// command author meets the table the first time `cargo test` runs.
// `Command::change()` reads it, so a command can still override when the table
// cannot express it.
//
// `Exempt` is for mutations that are not entity changes: conversation traffic
// (messages, tool results, tasks, session files), runs and their scores,
// syncs, previews, one-off triggers. The reason string says why, so the
// exemption is reviewed rather than assumed. See the Coverage table in
// knowledge/execution/change-reasons-and-manager-context.md.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::kernel_imports::Policy;

/// The kinds of entity whose changes are recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Agent,
    Harness,
    Skill,
    Capability,
    CheckRule,
    /// An agent channel: how an agent is exposed (an endpoint).
    AgentChannel,
    AgentTrigger,
    Schedule,
    KnowledgeBase,
    KnowledgeEntry,
    KnowledgeIndex,
    Memory,
    Provider,
    Model,
    McpServer,
    Plugin,
    PluginMarketplace,
    Workspace,
    VirtualUser,
    Observer,
    Budget,
    PaymentAccount,
    PaymentPolicy,
    Eval,
    EvalCase,
    SavedReport,
    Session,
}

impl EntityKind {
    pub const ALL: &'static [EntityKind] = &[
        Self::Agent,
        Self::Harness,
        Self::Skill,
        Self::Capability,
        Self::CheckRule,
        Self::AgentChannel,
        Self::AgentTrigger,
        Self::Schedule,
        Self::KnowledgeBase,
        Self::KnowledgeEntry,
        Self::KnowledgeIndex,
        Self::Memory,
        Self::Provider,
        Self::Model,
        Self::McpServer,
        Self::Plugin,
        Self::PluginMarketplace,
        Self::Workspace,
        Self::VirtualUser,
        Self::Observer,
        Self::Budget,
        Self::PaymentAccount,
        Self::PaymentPolicy,
        Self::Eval,
        Self::EvalCase,
        Self::SavedReport,
        Self::Session,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Harness => "harness",
            Self::Skill => "skill",
            Self::Capability => "capability",
            Self::CheckRule => "check_rule",
            Self::AgentChannel => "agent_channel",
            Self::AgentTrigger => "agent_trigger",
            Self::Schedule => "schedule",
            Self::KnowledgeBase => "knowledge_base",
            Self::KnowledgeEntry => "knowledge_entry",
            Self::KnowledgeIndex => "knowledge_index",
            Self::Memory => "memory",
            Self::Provider => "provider",
            Self::Model => "model",
            Self::McpServer => "mcp_server",
            Self::Plugin => "plugin",
            Self::PluginMarketplace => "plugin_marketplace",
            Self::Workspace => "workspace",
            Self::VirtualUser => "virtual_user",
            Self::Observer => "observer",
            Self::Budget => "budget",
            Self::PaymentAccount => "payment_account",
            Self::PaymentPolicy => "payment_policy",
            Self::Eval => "eval",
            Self::EvalCase => "eval_case",
            Self::SavedReport => "saved_report",
            Self::Session => "session",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == value)
    }

    /// The public id prefix of this kind, when it has one. Kinds addressed by
    /// a bare UUID (schedules, saved reports, check rules) have none and need
    /// an explicit `--kind`.
    pub fn id_prefix(self) -> Option<&'static str> {
        Some(match self {
            Self::Agent => "agent",
            Self::Harness => "harness",
            Self::Skill => "skill",
            Self::Capability => "cap",
            Self::AgentChannel => "appchan",
            Self::AgentTrigger => "trg",
            Self::KnowledgeBase => "kb",
            Self::KnowledgeEntry => "kbe",
            Self::KnowledgeIndex => "kidx",
            Self::Memory => "mem",
            Self::Provider => "provider",
            Self::Model => "model",
            Self::McpServer => "mcp",
            Self::Plugin => "plugin",
            Self::PluginMarketplace => "plgmkt",
            Self::Workspace => "wsp",
            Self::VirtualUser => "identity",
            Self::Observer => "observer",
            Self::Budget => "bdgt",
            Self::PaymentAccount => "payacct",
            Self::PaymentPolicy => "paypol",
            Self::Eval => "eval",
            Self::EvalCase => "evalcase",
            Self::Session => "session",
            Self::CheckRule | Self::Schedule | Self::SavedReport => return None,
        })
    }

    /// The kind a prefixed public id names, if any.
    pub fn from_ref(entity_ref: &str) -> Option<Self> {
        let (prefix, _) = entity_ref.split_once('_')?;
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.id_prefix() == Some(prefix))
    }

    /// Policy that may read this kind's history: the kind's own view policy,
    /// so history is visible exactly where the entity is.
    pub fn view_policy(self) -> &'static Policy {
        use crate::domains::*;
        match self {
            Self::Agent | Self::CheckRule | Self::AgentChannel | Self::AgentTrigger => {
                &agents::AGENT_VIEW
            }
            Self::Harness => &harnesses::HARNESS_VIEW,
            Self::Skill => &skills::SKILL_VIEW,
            Self::Capability => &capabilities::CAPABILITY_VIEW,
            Self::Schedule => &schedules::SCHEDULE_VIEW,
            Self::KnowledgeBase | Self::KnowledgeEntry => &knowledge_bases::KNOWLEDGE_BASE_VIEW,
            Self::KnowledgeIndex => &knowledge_indexes::KNOWLEDGE_INDEX_VIEW,
            Self::Memory => &memory::MEMORY_VIEW,
            Self::Provider => &providers::LLM_PROVIDER_VIEW,
            Self::Model => &models::LLM_MODEL_VIEW,
            Self::McpServer => &mcp_servers::MCP_SERVER_VIEW,
            Self::Plugin | Self::PluginMarketplace => &plugins::PLUGIN_VIEW,
            Self::Workspace => &workspaces::WORKSPACE_VIEW,
            Self::VirtualUser => &virtual_users::VIRTUAL_USER_VIEW,
            Self::Observer => &observers::service::OBSERVER_VIEW,
            Self::Budget => &budgets::BUDGET_VIEW,
            Self::PaymentAccount | Self::PaymentPolicy => &payments::PAYMENT_VIEW,
            Self::Eval | Self::EvalCase => &evals::service::EVAL_VIEW,
            Self::SavedReport => &reporting::REPORT_VIEW,
            // Session metadata is private to its participants, which a role
            // policy cannot express; `history list` also checks the session.
            Self::Session => &sessions::service::SESSION_VIEW,
        }
    }

    /// Policy of the people who manage this kind: who may read and write its
    /// manager context. The kind's own manage policy, so notes addressed to
    /// managers reach exactly the people who can act on them.
    pub fn manage_policy(self) -> &'static Policy {
        use crate::domains::*;
        match self {
            Self::Agent | Self::AgentChannel | Self::AgentTrigger => &agents::AGENT_MANAGE,
            Self::CheckRule => &agents::check_rules::AGENT_CHECKS_MANAGE,
            Self::Harness => &harnesses::HARNESS_MANAGE,
            Self::Skill => &skills::SKILL_MANAGE,
            Self::Capability => &capabilities::CAPABILITY_MANAGE,
            Self::Schedule => &schedules::SCHEDULE_MANAGE,
            Self::KnowledgeBase | Self::KnowledgeEntry => &knowledge_bases::KNOWLEDGE_BASE_MANAGE,
            Self::KnowledgeIndex => &knowledge_indexes::KNOWLEDGE_INDEX_MANAGE,
            Self::Memory => &memory::MEMORY_MANAGE,
            Self::Provider => &providers::LLM_PROVIDER_MANAGE,
            Self::Model => &models::LLM_MODEL_MANAGE,
            Self::McpServer => &mcp_servers::MCP_SERVER_MANAGE,
            Self::Plugin | Self::PluginMarketplace => &plugins::PLUGIN_MANAGE,
            Self::Workspace => &workspaces::WORKSPACE_MANAGE,
            Self::VirtualUser => &virtual_users::VIRTUAL_USER_MANAGE,
            Self::Observer => &observers::service::OBSERVER_MANAGE,
            Self::Budget => &budgets::BUDGET_MANAGE,
            Self::PaymentAccount | Self::PaymentPolicy => &payments::PAYMENT_MANAGE,
            Self::Eval | Self::EvalCase => &evals::service::EVAL_MANAGE,
            Self::SavedReport => &reporting::REPORT_MANAGE,
            Self::Session => &sessions::service::SESSION_MANAGE,
        }
    }

    /// Whether this kind carries manager context. Knowledge base entries and
    /// eval cases are content, not things a manager configures: notes about
    /// them belong on the parent. Sessions have their own lifecycle and are not
    /// managed in this sense.
    pub fn has_manager_context(self) -> bool {
        !matches!(self, Self::KnowledgeEntry | Self::EvalCase | Self::Session)
    }

    /// The read command that fetches one entity of this kind by its ref alone,
    /// and that command's id param. Used to confirm an entity exists, and is
    /// visible to the caller, before anything is written about it. Kinds whose
    /// read needs a parent id (channels, triggers, check rules) have none.
    pub fn lookup(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Self::Agent => ("get_agent", "id"),
            Self::Harness => ("get_harness", "id"),
            Self::Skill => ("get_skill", "id"),
            Self::Capability => ("get_declarative_capability", "id"),
            Self::Schedule => ("get_schedule", "schedule_id"),
            Self::KnowledgeBase => ("get_knowledge_base", "kb_id"),
            Self::KnowledgeIndex => ("get_knowledge_index", "index_id"),
            Self::Memory => ("get_memory", "memory_id"),
            Self::Provider => ("get_provider", "id"),
            Self::Model => ("get_model", "id"),
            Self::McpServer => ("get_mcp_server", "id"),
            Self::Plugin => ("get_plugin", "id"),
            Self::PluginMarketplace => ("get_plugin_marketplace", "id"),
            Self::Workspace => ("get_workspace", "workspace_id"),
            Self::VirtualUser => ("get_virtual_user", "id"),
            Self::Observer => ("get_observer", "observer_id"),
            Self::Budget => ("get_budget", "budget_id"),
            Self::PaymentAccount => ("get_payment_account", "payment_account_id"),
            Self::PaymentPolicy => ("get_payment_policy", "payment_policy_id"),
            Self::Eval => ("get_eval", "eval_id"),
            Self::SavedReport => ("get_saved_report", "report_id"),
            Self::Session => ("get_session", "session_id"),
            Self::CheckRule
            | Self::AgentChannel
            | Self::AgentTrigger
            | Self::KnowledgeEntry
            | Self::EvalCase => return None,
        })
    }
}

/// What a recorded change did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Created,
    Updated,
    Deleted,
    Restored,
    Forked,
    Imported,
    ContextUpdated,
    Attached,
    Detached,
    Archived,
    Unarchived,
    Published,
    Unpublished,
    Suspended,
    Resumed,
    Paused,
    Disabled,
}

impl ChangeAction {
    pub const ALL: &'static [ChangeAction] = &[
        Self::Created,
        Self::Updated,
        Self::Deleted,
        Self::Restored,
        Self::Forked,
        Self::Imported,
        Self::ContextUpdated,
        Self::Attached,
        Self::Detached,
        Self::Archived,
        Self::Unarchived,
        Self::Published,
        Self::Unpublished,
        Self::Suspended,
        Self::Resumed,
        Self::Paused,
        Self::Disabled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Deleted => "deleted",
            Self::Restored => "restored",
            Self::Forked => "forked",
            Self::Imported => "imported",
            Self::ContextUpdated => "context_updated",
            Self::Attached => "attached",
            Self::Detached => "detached",
            Self::Archived => "archived",
            Self::Unarchived => "unarchived",
            Self::Published => "published",
            Self::Unpublished => "unpublished",
            Self::Suspended => "suspended",
            Self::Resumed => "resumed",
            Self::Paused => "paused",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|action| action.as_str() == value)
    }
}

/// Where the changed entity's id is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectId {
    /// A top-level field of the params (update, delete: the id is an input).
    Param(&'static str),
    /// A top-level field of the output (create: the id exists only after).
    Output(&'static str),
}

/// What a command changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Read-only.
    None,
    /// A mutating command nobody has classified yet. The guard rejects it.
    Undeclared,
    /// A mutation that is not an entity change, and why.
    Exempt(&'static str),
    /// A change to one entity.
    Subject {
        kind: EntityKind,
        action: ChangeAction,
        id: SubjectId,
    },
}

/// `Command::change()`'s default: reads change nothing, writes are looked up.
pub fn default_for(read_only: bool, name: &str) -> Change {
    if read_only {
        Change::None
    } else {
        declared(name)
    }
}

/// The declaration for a mutating command, by wire name.
pub fn declared(name: &str) -> Change {
    use ChangeAction::*;
    use EntityKind as K;
    use SubjectId::{Output, Param};

    const fn on(kind: EntityKind, action: ChangeAction, id: SubjectId) -> Change {
        Change::Subject { kind, action, id }
    }
    const ID: SubjectId = Output("id");

    match name {
        // Agents and what hangs off them.
        "create_agent" | "copy_agent" => on(K::Agent, Created, ID),
        "import_agent" => on(K::Agent, Imported, ID),
        "update_agent" | "upsert_agent" | "set_default_agent_version" => on(K::Agent, Updated, ID),
        "delete_agent" | "destroy_agent" => on(K::Agent, Deleted, Param("id")),
        "fork_agent_version" => on(K::Agent, Forked, ID),
        "rollback_agent_version" => on(K::Agent, Restored, ID),
        "suspend_agent_exposures" => on(K::Agent, Suspended, ID),
        "resume_agent_exposures" => on(K::Agent, Resumed, ID),
        "create_agent_credential_binding" => on(K::Agent, Attached, Param("agent_id")),
        "create_agent_version" => {
            Change::Exempt("a saved agent version is a copy of the agent, not a change to it")
        }
        "upsert_agent_check_rule" => on(K::CheckRule, Updated, Param("rule_id")),
        "delete_agent_check_rule" => on(K::CheckRule, Deleted, Param("rule_id")),
        "analyze_agent" | "preview_agent" | "diff_agent_package" | "validate_agent_package" => {
            Change::Exempt("advisory: computes a result without changing anything")
        }
        "trigger_agent_health_check" => Change::Exempt("a health-check run, not a change"),
        "export_agent" => Change::Exempt("an export reads the agent"),

        "create_agent_channel" => on(K::AgentChannel, Created, ID),
        "update_agent_channel" => on(K::AgentChannel, Updated, ID),
        "publish_agent_channel" => on(K::AgentChannel, Published, ID),
        "unpublish_agent_channel" => on(K::AgentChannel, Unpublished, ID),
        "delete_agent_channel" => on(K::AgentChannel, Deleted, Param("channel_id")),
        "trigger_agent_channel" => Change::Exempt("a manual fire, not a change"),

        "create_agent_trigger" => on(K::AgentTrigger, Created, ID),
        "update_agent_trigger" => on(K::AgentTrigger, Updated, ID),
        "delete_agent_trigger" => on(K::AgentTrigger, Deleted, Param("trigger_id")),
        "trigger_agent_trigger" => Change::Exempt("a manual fire, not a change"),

        "create_schedule" => on(K::Schedule, Created, ID),
        "update_schedule" => on(K::Schedule, Updated, Param("schedule_id")),
        "pause_schedule" => on(K::Schedule, Paused, Param("schedule_id")),
        "resume_schedule" => on(K::Schedule, Resumed, Param("schedule_id")),
        "delete_schedule" => on(K::Schedule, Deleted, Param("schedule_id")),
        "trigger_schedule" => Change::Exempt("a manual fire, not a change"),

        // Agent definition building blocks.
        "create_harness" | "copy_harness" => on(K::Harness, Created, ID),
        "update_harness" => on(K::Harness, Updated, ID),
        "delete_harness" | "destroy_harness" => on(K::Harness, Deleted, Param("id")),
        "preview_harness" => {
            Change::Exempt("advisory: computes a result without changing anything")
        }

        "create_skill" => on(K::Skill, Created, ID),
        "update_skill" => on(K::Skill, Updated, ID),
        "delete_skill" | "destroy_skill" => on(K::Skill, Deleted, Param("id")),

        "create_declarative_capability" => on(K::Capability, Created, ID),
        "update_declarative_capability" => on(K::Capability, Updated, ID),
        "delete_declarative_capability" | "destroy_declarative_capability" => {
            on(K::Capability, Deleted, Param("id"))
        }
        "dry_run_guardrails" => {
            Change::Exempt("advisory: computes a result without changing anything")
        }

        // Knowledge.
        "create_knowledge_base" => on(K::KnowledgeBase, Created, ID),
        "update_knowledge_base" => on(K::KnowledgeBase, Updated, ID),
        "delete_knowledge_base" => on(K::KnowledgeBase, Deleted, Param("kb_id")),
        "import_okf_bundle" => on(K::KnowledgeBase, Imported, Param("kb_id")),
        "create_knowledge_entry" => on(K::KnowledgeEntry, Created, ID),
        "update_knowledge_entry" => on(K::KnowledgeEntry, Updated, ID),
        "delete_knowledge_entry" => on(K::KnowledgeEntry, Deleted, Param("entry_id")),

        "create_knowledge_index" => on(K::KnowledgeIndex, Created, ID),
        "update_knowledge_index" => on(K::KnowledgeIndex, Updated, ID),
        "delete_knowledge_index" => on(K::KnowledgeIndex, Deleted, Param("index_id")),
        "sync_knowledge_index" => Change::Exempt("a sync run refreshes content, not configuration"),

        "create_memory" => on(K::Memory, Created, ID),
        "update_memory" => on(K::Memory, Updated, ID),
        "delete_memory" => on(K::Memory, Deleted, Param("memory_id")),
        "sync_memory_now" => Change::Exempt("a sync run refreshes content, not configuration"),

        // Connections.
        "create_provider" => on(K::Provider, Created, ID),
        "update_provider" => on(K::Provider, Updated, ID),
        "delete_provider" => on(K::Provider, Deleted, Param("id")),
        "check_provider_credentials" => Change::Exempt("a credential check, not a change"),
        "sync_provider_models" => Change::Exempt("a sync run refreshes the model list"),

        "create_model" => on(K::Model, Created, ID),
        "update_model" => on(K::Model, Updated, ID),
        "delete_model" => on(K::Model, Deleted, Param("id")),
        "set_default_decision_model" => {
            Change::Exempt("an organization setting, not a change to one model")
        }

        "create_mcp_server" => on(K::McpServer, Created, ID),
        "update_mcp_server" => on(K::McpServer, Updated, ID),
        "delete_mcp_server" | "destroy_mcp_server" => on(K::McpServer, Deleted, Param("id")),

        "install_plugin" => on(K::Plugin, Created, ID),
        "patch_installed_plugin" | "update_plugin" => on(K::Plugin, Updated, ID),
        "uninstall_plugin" => on(K::Plugin, Deleted, Param("id")),
        "create_plugin_marketplace" => on(K::PluginMarketplace, Created, ID),
        "update_plugin_marketplace" => on(K::PluginMarketplace, Updated, ID),
        "delete_plugin_marketplace" => on(K::PluginMarketplace, Deleted, Param("id")),
        "sync_plugin_marketplace" => Change::Exempt("a sync run refreshes the catalog"),

        // Organization.
        "create_workspace" => on(K::Workspace, Created, ID),
        "update_workspace" => on(K::Workspace, Updated, ID),
        "delete_workspace" => on(K::Workspace, Deleted, Param("workspace_id")),

        "create_virtual_user" => on(K::VirtualUser, Created, ID),
        "update_virtual_user" => on(K::VirtualUser, Updated, ID),
        "delete_virtual_user" | "destroy_virtual_user" => on(K::VirtualUser, Deleted, Param("id")),

        "create_observer" => on(K::Observer, Created, ID),
        "update_observer" => on(K::Observer, Updated, ID),
        "delete_observer" => on(K::Observer, Deleted, Param("observer_id")),

        "create_budget" => on(K::Budget, Created, ID),
        "update_budget" => on(K::Budget, Updated, ID),
        "delete_budget" => on(K::Budget, Deleted, Param("budget_id")),
        "top_up_budget" => Change::Exempt("a balance movement, recorded in the budget ledger"),
        "resume_session_budgets" => Change::Exempt("resumes sessions a budget paused"),

        "create_payment_account" => on(K::PaymentAccount, Created, ID),
        "update_payment_account" => on(K::PaymentAccount, Updated, ID),
        "disable_payment_account" => on(K::PaymentAccount, Disabled, Param("payment_account_id")),
        "create_payment_policy" => on(K::PaymentPolicy, Created, ID),
        "update_payment_policy" => on(K::PaymentPolicy, Updated, ID),
        "disable_payment_policy" => on(K::PaymentPolicy, Disabled, Param("payment_policy_id")),

        // Evaluation and reporting.
        "create_eval" => on(K::Eval, Created, ID),
        "update_eval" => on(K::Eval, Updated, ID),
        "delete_eval" => on(K::Eval, Deleted, Param("eval_id")),
        "import_atif_trajectories" => on(K::Eval, Imported, Param("eval_id")),
        "create_eval_case" => on(K::EvalCase, Created, ID),
        "update_eval_case" => on(K::EvalCase, Updated, ID),
        "delete_eval_case" => on(K::EvalCase, Deleted, Param("case_id")),
        "create_eval_run" | "cancel_eval_run" | "import_eval_run" => {
            Change::Exempt("eval runs are results, not configuration")
        }
        "bulk_update_eval_run_scores" | "update_eval_result_scores" => {
            Change::Exempt("scores are results, not configuration")
        }
        "create_eval_run_share" | "revoke_eval_run_share" | "export_eval_run_dataset" => {
            Change::Exempt("sharing or exporting a run's results")
        }

        "create_saved_report" => on(K::SavedReport, Created, ID),
        "update_saved_report" => on(K::SavedReport, Updated, ID),
        "delete_saved_report" => on(K::SavedReport, Deleted, Param("report_id")),
        "run_saved_report" | "run_report_query" | "export_report_query" | "export_saved_report" => {
            Change::Exempt("running or exporting a report")
        }
        "run_reporting_projector" | "backfill_reporting" => {
            Change::Exempt("reporting maintenance rebuilds derived data")
        }

        // Sessions: their metadata is recorded, their contents are not.
        "create_session" => on(K::Session, Created, ID),
        "fork_session" => on(K::Session, Forked, ID),
        "update_session" | "pin_session" | "unpin_session" => {
            on(K::Session, Updated, Param("session_id"))
        }
        "archive_session" => on(K::Session, Archived, Param("session_id")),
        "unarchive_session" => on(K::Session, Unarchived, Param("session_id")),
        "delete_session" => on(K::Session, Deleted, Param("session_id")),
        "add_session_participant" => on(K::Session, Attached, Param("session_id")),
        "leave_session_participant" => on(K::Session, Detached, Param("session_id")),
        "ensure_platform_chat" => Change::Exempt("finds or opens the caller's own Platform Chat"),
        "cancel_session" => Change::Exempt("turn control, not a change to the session"),
        "create_message" | "submit_tool_results" => {
            Change::Exempt("conversation traffic, kept in the session's events")
        }
        "cancel_session_task"
        | "post_session_task_message"
        | "create_task_push_config"
        | "delete_task_push_config" => Change::Exempt("task traffic inside a session"),
        "create_workspace_file"
        | "update_workspace_file"
        | "delete_workspace_file"
        | "copy_workspace_file"
        | "move_workspace_file"
        | "grep_workspace_files"
        | "search_workspace_files"
        | "stat_workspace_file" => Change::Exempt("session files are the session's working state"),
        "create_session_database"
        | "delete_session_database"
        | "manage_session_sandbox"
        | "batch_set_session_secrets"
        | "delete_session_secret" => {
            Change::Exempt("session resources are the session's working state")
        }

        "mark_notification_viewed" => Change::Exempt("read state of the caller's own inbox"),
        "check_health_issue" | "snooze_health_issue" => {
            Change::Exempt("triage state of a health issue, not configuration")
        }

        _ => Change::Undeclared,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_actions_round_trip_through_their_names() {
        for kind in EntityKind::ALL {
            assert_eq!(EntityKind::parse(kind.as_str()), Some(*kind));
            let json = serde_json::to_value(kind).unwrap();
            assert_eq!(json, kind.as_str());
        }
        for action in ChangeAction::ALL {
            assert_eq!(ChangeAction::parse(action.as_str()), Some(*action));
            assert_eq!(serde_json::to_value(action).unwrap(), action.as_str());
        }
    }

    #[test]
    fn a_prefixed_ref_names_its_kind() {
        assert_eq!(
            EntityKind::from_ref("agent_01933b5a000070008000000000000001"),
            Some(EntityKind::Agent)
        );
        assert_eq!(
            EntityKind::from_ref("kbe_01933b5a000070008000000000000001"),
            Some(EntityKind::KnowledgeEntry)
        );
        assert_eq!(
            EntityKind::from_ref("01933b5a-0000-7000-8000-000000000001"),
            None
        );
        assert_eq!(EntityKind::from_ref("support-agent"), None);
    }

    #[test]
    fn id_prefixes_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for kind in EntityKind::ALL {
            if let Some(prefix) = kind.id_prefix() {
                assert!(seen.insert(prefix), "{prefix} is claimed twice");
            }
        }
    }
}
