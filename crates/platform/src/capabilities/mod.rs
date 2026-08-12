//! Hosted product capabilities.
//!
//! These implementations require product services, persisted state, or hosted
//! orchestration. Server and worker presets register them explicitly; the
//! standalone Framework registry does not advertise them.

#[cfg(feature = "a2a")]
pub mod a2a_delegation;
pub mod agent_handoff;
pub mod background_execution;
pub mod citation_retrieval;
pub mod citation_verification;
pub mod data_knowledge;
pub mod delegation_result;
pub mod knowledge_base;
pub mod knowledge_index;
pub mod memory;
pub mod monitors;
pub mod platform;
pub mod platform_management;
pub mod research;
pub mod session;
pub mod session_sandbox;
pub mod session_schedule;
pub mod session_sql_database;
pub mod session_storage;
pub mod session_tasks;
pub mod subagents;
pub mod user_hooks;
pub mod util;

pub use everruns_core::capabilities::{
    A2A_AGENT_DELEGATION_CAPABILITY_ID, AGENT_RUN_KEY_PREFIX, DelegationTargetProvider,
    SPAWN_AGENT_CONCURRENCY_CLASS,
};
pub(crate) use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, MountDirectoryBuilder, MountPoint,
    RiskLevel, SpawnMode, SystemPromptContext,
};

#[cfg(feature = "a2a")]
pub use a2a_delegation::{A2aAgentDelegationCapability, SpawnAgentTool};
pub use agent_handoff::{
    AGENT_HANDOFF_CAPABILITY_ID, AgentHandoffCapability, SpawnAgentHandoffTool,
};
pub use background_execution::{BACKGROUND_EXECUTION_CAPABILITY_ID, BackgroundExecutionCapability};
pub use citation_retrieval::{
    CITATION_RETRIEVAL_CAPABILITY_ID, CitationRetrievalCapability, CitationRetrievalConfig,
};
pub use citation_verification::{
    CITATION_VERIFICATION_CAPABILITY_ID, CitationVerificationCapability,
    CitationVerificationConfig, VerificationMode,
};
pub use data_knowledge::{DATA_KNOWLEDGE_CAPABILITY_ID, DataKnowledgeCapability};
pub use delegation_result::{
    ReportResultTool, ReportTaskProgressTool, report_result_tool_for_child_session,
    report_task_progress_tool_for_child_session,
};
pub use knowledge_base::{
    KNOWLEDGE_BASE_CAPABILITY_ID, KnowledgeBaseCapability, KnowledgeBaseConfig,
    validate_knowledge_base_config,
};
pub use knowledge_index::{
    KNOWLEDGE_INDEX_CAPABILITY_ID, KnowledgeIndexCapability, KnowledgeIndexConfig,
    validate_knowledge_index_config,
};
pub use memory::{MEMORY_CAPABILITY_ID, MemoryCapability};
pub use platform::{
    DISCOVER_DESCRIPTION as PLATFORM_DISCOVER_DESCRIPTION,
    EXECUTE_DESCRIPTION as PLATFORM_EXECUTE_DESCRIPTION, PLATFORM_CAPABILITY_ID,
    PlatformCapability, QUERY_DESCRIPTION as PLATFORM_QUERY_DESCRIPTION, discover_input_schema,
    execute_input_schema, query_input_schema,
};
pub use platform_management::{
    ManageAgentsTool, ManageHarnessesTool, ManageSessionsTool, PLATFORM_MANAGEMENT_CAPABILITY_ID,
    PlatformManagementCapability, ReadAgentsTool, ReadCapabilitiesTool, ReadHarnessesTool,
    ReadSessionsTool, SessionReadMessagesTool, SessionReadResponseTool, SessionSendMessageTool,
};
pub use research::{RESEARCH_CAPABILITY_ID, ResearchCapability};
pub use session::{
    GetSessionInfoTool, SESSION_CAPABILITY_ID, SessionCapability, SessionCapabilityConfig,
    SessionTitleMutation, WriteSessionTitleTool, session_title_updated_event,
    update_session_title_with_event,
};
pub use session_sandbox::{
    SESSION_SANDBOX_CAPABILITY_ID, SandboxExecTool, SandboxManageTool, SandboxReadFileTool,
    SandboxStatusTool, SandboxWriteFileTool, SessionSandboxCapability,
};
pub use session_schedule::{
    CancelScheduleTool, CreateScheduleTool, ListSchedulesTool, SESSION_SCHEDULE_CAPABILITY_ID,
    SessionScheduleCapability,
};
pub use session_sql_database::{
    SESSION_SQL_DATABASE_CAPABILITY_ID, SessionSqlDatabaseCapability, SqlExecuteTool, SqlQueryTool,
    SqlSchemaTool,
};
pub use session_storage::{
    KvStoreTool, SESSION_STORAGE_CAPABILITY_ID, SecretStoreTool, SessionStorageCapability,
    is_internal_session_kv_key, is_internal_session_secret_name,
};
pub use session_tasks::{SESSION_TASKS_CAPABILITY_ID, SessionTasksCapability};
pub use subagents::{
    SUBAGENTS_CAPABILITY_ID, SpawnLifetime, SpawnSubagentAsAgentTool, SubagentCapability,
};
pub use user_hooks::{USER_HOOKS_CAPABILITY_ID, UserHooksCapability};

/// Register the hosted platform-management capabilities on a registry.
pub fn register_platform_capabilities(
    registry: &mut everruns_core::capabilities::CapabilityRegistry,
) {
    registry.register(PlatformCapability);
    registry.register(PlatformManagementCapability);
}

/// Register environment-backed capabilities compiled into the hosted product.
#[cfg(feature = "environment-capabilities")]
pub fn register_environment_capabilities(
    registry: &mut everruns_core::capabilities::CapabilityRegistry,
) {
    registry.register(everruns_integrations_filesystem::FileSystemCapability);
    registry.register(everruns_integrations_bashkit::BashkitShellCapability);
    registry.register(everruns_integrations_web_fetch::WebFetchCapability::from_env());
    registry.register(everruns_integrations_openrouter_workspace::ModelScoutCapability);
    registry.register(everruns_integrations_openrouter_workspace::OpenRouterWorkspaceCapability);

    #[cfg(feature = "lua")]
    if everruns_core::InternalFeatureFlags::from_env().lua {
        registry.register(everruns_integrations_lua::LuaCapability);
        registry.register(everruns_integrations_lua::LuaCodeModeCapability);
    }
}

#[cfg(not(feature = "environment-capabilities"))]
fn register_environment_capabilities(
    _registry: &mut everruns_core::capabilities::CapabilityRegistry,
) {
}

/// Register every hosted product capability while preserving stable IDs,
/// configuration schemas, feature gates, and task-executor registrations.
pub fn register_hosted_capabilities(
    registry: &mut everruns_core::capabilities::CapabilityRegistry,
    grade: everruns_core::DeploymentGrade,
) {
    // Session-service capabilities (EVE-886): every one needs a host service —
    // a session store, session key/value + secret storage, a SQL database, or a
    // sandbox provider — so they are composed here rather than shipped by the
    // kernel.
    registry.register(SessionCapability);
    registry.register(SessionStorageCapability);
    registry.register(SessionSqlDatabaseCapability);
    if everruns_core::InternalFeatureFlags::from_env().session_sandbox {
        registry.register(SessionSandboxCapability);
    }
    registry.register(ResearchCapability);
    registry.register(MemoryCapability);
    registry.register(BackgroundExecutionCapability);
    registry.register(SessionScheduleCapability);
    registry.register(SubagentCapability);
    registry.register(SessionTasksCapability);
    if everruns_core::ExecutionFeatureDecisions::from_env(grade).agent_delegation {
        registry.register(AgentHandoffCapability);
        #[cfg(feature = "a2a")]
        registry.register(A2aAgentDelegationCapability);
    }
    registry.register(UserHooksCapability);
    registry.register(DataKnowledgeCapability);
    registry.register(KnowledgeBaseCapability);
    registry.register(KnowledgeIndexCapability);
    registry.register(CitationRetrievalCapability);
    registry.register(CitationVerificationCapability);
    register_environment_capabilities(registry);
    register_platform_capabilities(registry);
}

/// Portable builtins plus the hosted product catalog for a deployment grade.
pub fn hosted_capability_registry_for_grade(
    grade: everruns_core::DeploymentGrade,
) -> everruns_core::capabilities::CapabilityRegistry {
    let mut registry =
        everruns_core::capabilities::CapabilityRegistry::with_builtins_for_grade(grade);
    #[cfg(feature = "portable-builtins")]
    everruns_builtins::register_portable_capabilities(&mut registry)
        .expect("core and portable built-in catalogs must not collide");
    register_hosted_capabilities(&mut registry, grade);
    registry
}

/// [`hosted_capability_registry_for_grade`] using the environment grade.
pub fn hosted_capability_registry() -> everruns_core::capabilities::CapabilityRegistry {
    hosted_capability_registry_for_grade(everruns_core::DeploymentGrade::from_env())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_registry_always_contains_platform_capabilities() {
        let registry = hosted_capability_registry_for_grade(everruns_core::DeploymentGrade::Prod);
        assert!(registry.has(PLATFORM_CAPABILITY_ID));
        assert!(registry.has(PLATFORM_MANAGEMENT_CAPABILITY_ID));
    }

    #[cfg(feature = "portable-builtins")]
    #[test]
    fn hosted_registry_contains_full_portable_policy_catalog() {
        let registry = hosted_capability_registry_for_grade(everruns_core::DeploymentGrade::Prod);
        for capability_id in [
            "current_time",
            "compaction",
            "tool_call_repair",
            "usage_limit_auto_continue",
        ] {
            assert!(
                registry.has(capability_id),
                "hosted product registry is missing `{capability_id}`"
            );
        }
    }

    #[cfg(feature = "environment-capabilities")]
    #[test]
    fn hosted_registry_composes_product_environment_capabilities() {
        let registry = hosted_capability_registry_for_grade(everruns_core::DeploymentGrade::Prod);
        for capability_id in [
            "session_file_system",
            "bashkit_shell",
            "web_fetch",
            "model_scout",
            "openrouter_workspace",
        ] {
            assert!(
                registry.has(capability_id),
                "hosted product registry is missing `{capability_id}`"
            );
        }
        assert_eq!(registry.canonical_id("virtual_bash"), Some("bashkit_shell"));
    }

    #[cfg(not(feature = "environment-capabilities"))]
    #[test]
    fn platform_default_does_not_link_environment_capabilities() {
        let registry = hosted_capability_registry_for_grade(everruns_core::DeploymentGrade::Prod);
        for capability_id in [
            "session_file_system",
            "bashkit_shell",
            "web_fetch",
            "model_scout",
            "openrouter_workspace",
        ] {
            assert!(!registry.has(capability_id));
        }
    }
}
