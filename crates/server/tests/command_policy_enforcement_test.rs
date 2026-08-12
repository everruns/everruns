//! Security integration tests for command policy enforcement.
//!
//! Ensures that `Command::run()` — the single entry point for every caller
//! kind (HTTP, MCP, gRPC, platform capability) — evaluates `Command::policy()`
//! against the `PermissionResolver` carried in `Ctx`. Without enforcement at
//! this layer, HTTP requests would bypass role-based restrictions and SaaS
//! custom resolvers would never be consulted during write operations.
//!
//! Covers:
//!  1. Role-based denial (Member cannot manage harnesses; default resolver).
//!  2. Role-based success (Member can view; Owner can manage).
//!  3. A custom `PermissionResolver` plugged into `Ctx` overrides the default
//!     role map (including denying an Owner) — the SaaS enforcement contract.
//!  4. `dispatch()` (MCP + gRPC `ExecuteCommand`) enforces identically.
//!  5. Inventory coverage: every mutating command declares `policy()`.
//!  6. MCP `query` exposes only reviewed non-GET read-only helpers.
//!
//! Run with: cargo test -p everruns-server --test command_policy_enforcement_test

use std::sync::Arc;

use everruns_core::{
    AgentId, Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DefaultPermissionResolver, OrgRole,
    Permission, PermissionResolver, SessionSeedMode,
};
use everruns_platform::FeatureFlags;
use everruns_server::api::evals::CreateEvalRunRequest;
use everruns_server::api::sessions::CreateSessionRequest;
use everruns_server::domains::agents::health_check::commands::TriggerAgentHealthCheck;
use everruns_server::domains::common::{
    Command, CommandError, CommandErrorKind, Ctx, catalog_entries, dispatch,
};
use everruns_server::domains::evals::{CreateEvalRun, ListEvals};
use everruns_server::domains::harnesses::types::CreateHarnessRequest;
use everruns_server::domains::harnesses::{CreateHarness, ListHarnesses};
use everruns_server::domains::messages::{ExportSessionMessages, SessionExportFormat};
use everruns_server::domains::session_files::{GetWorkspaceFile, ListWorkspaceFiles};
use everruns_server::domains::session_tasks::{
    CancelSessionTask, GetSessionTask, ListSessionTasks, PostSessionTaskMessage,
};
use everruns_server::domains::sessions::CreateSession;
use everruns_server::services::CapabilityService;
use everruns_server::storage::StorageBackend;
use everruns_server::storage::models::{CreateAgentRow, CreateHarnessRow};
use uuid::Uuid;

// ============================================================================
// Fixtures
// ============================================================================

fn caller_with_role(role: OrgRole) -> Caller {
    Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
        user_id: Some(Uuid::nil()),
        role,
        is_platform_user: false,
        is_internal: false,
    }
}

fn make_ctx(caller: Caller, resolver: Arc<dyn PermissionResolver>) -> Ctx {
    let db = Arc::new(StorageBackend::in_memory());
    let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
    Ctx::new(caller, db, capability_service, None, resolver)
}

fn minimal_harness(name: &str) -> CreateHarnessRequest {
    CreateHarnessRequest {
        name: name.to_string(),
        display_name: None,
        description: None,
        system_prompt: Some("test prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: Vec::new(),
        capabilities: Vec::new(),
        initial_files: Vec::new(),
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    }
}

/// Denies every permission — models a SaaS tier that blocks all writes.
struct DenyAllResolver;

impl PermissionResolver for DenyAllResolver {
    fn has_permission(&self, _caller: &Caller, _permission: &Permission) -> bool {
        false
    }
    fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
        Vec::new()
    }
}

/// Grants only harness read — models a read-only SaaS tier.
struct HarnessReadOnlyResolver;

impl PermissionResolver for HarnessReadOnlyResolver {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        matches!(permission, Permission::OrgHarnessesView)
    }
    fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
        vec![Permission::OrgHarnessesView]
    }
}

// ============================================================================
// 1. Role-based enforcement via Command::run (the HTTP path)
// ============================================================================

#[tokio::test]
async fn run_blocks_member_from_manage_command() {
    // Members have OrgHarnessesView but not OrgHarnessesManage. Create must fail.
    let ctx = make_ctx(
        caller_with_role(OrgRole::Member),
        Arc::new(DefaultPermissionResolver),
    );
    let err = CreateHarness(minimal_harness("member-denied"))
        .run(&ctx)
        .await
        .expect_err("Member must not be allowed to create harnesses");
    assert!(
        matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Forbidden(_),
                ..
            }
        ),
        "expected Forbidden, got {err:?}"
    );
}

#[tokio::test]
async fn run_allows_member_view_command() {
    let ctx = make_ctx(
        caller_with_role(OrgRole::Member),
        Arc::new(DefaultPermissionResolver),
    );
    ListHarnesses {
        search: None,
        include_archived: false,
    }
    .run(&ctx)
    .await
    .expect("Member should be allowed to list harnesses");
}

#[tokio::test]
async fn run_allows_owner_manage_command() {
    let ctx = make_ctx(
        caller_with_role(OrgRole::Owner),
        Arc::new(DefaultPermissionResolver),
    );
    CreateHarness(minimal_harness("owner-allowed"))
        .run(&ctx)
        .await
        .expect("Owner should be allowed to create harnesses");
}

// ============================================================================
// 2. Custom PermissionResolver (the SaaS enforcement contract) fires during
//    run() — not only at /config endpoints.
// ============================================================================

#[tokio::test]
async fn run_honors_custom_resolver_denying_owner_write() {
    // Owner role would normally pass — SaaS resolver denies. This is the
    // exact property SaaS enforcement depends on (billing tier, etc.).
    let ctx = make_ctx(caller_with_role(OrgRole::Owner), Arc::new(DenyAllResolver));
    let err = CreateHarness(minimal_harness("saas-denied"))
        .run(&ctx)
        .await
        .expect_err("Custom resolver must block Owner write");
    assert!(
        matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Forbidden(_),
                ..
            }
        ),
        "expected Forbidden, got {err:?}"
    );
}

#[tokio::test]
async fn session_task_commands_honor_session_permissions_before_access() {
    // Regression coverage for session-task REST/MCP commands: a custom resolver
    // denying OrgSessionsManage must block before any session/task lookup or mutation.
    let ctx = make_ctx(caller_with_role(OrgRole::Owner), Arc::new(DenyAllResolver));

    let list_err = ListSessionTasks {
        session_id: "not-a-session-id".to_string(),
        state: None,
        kind: None,
    }
    .run(&ctx)
    .await
    .expect_err("list must require the session view policy");
    assert_forbidden(list_err);

    let get_err = GetSessionTask {
        session_id: "not-a-session-id".to_string(),
        task_id: "task_test".to_string(),
        after_id: None,
        limit: None,
    }
    .run(&ctx)
    .await
    .expect_err("get must require the session view policy");
    assert_forbidden(get_err);

    let post_err = PostSessionTaskMessage {
        session_id: "not-a-session-id".to_string(),
        task_id: "task_test".to_string(),
        text: Some("answer".to_string()),
        content: None,
        in_reply_to: None,
    }
    .run(&ctx)
    .await
    .expect_err("post message must require the session manage policy");
    assert_forbidden(post_err);

    let cancel_err = CancelSessionTask {
        session_id: "not-a-session-id".to_string(),
        task_id: "task_test".to_string(),
    }
    .run(&ctx)
    .await
    .expect_err("cancel must require the session manage policy");
    assert_forbidden(cancel_err);
}

#[tokio::test]
async fn session_export_requires_session_view_policy_before_lookup() {
    // Regression for ATIF export RBAC: both export formats expose session data,
    // so a resolver denying OrgSessionsManage must block before ID parsing or lookup.
    let ctx = make_ctx(caller_with_role(OrgRole::Owner), Arc::new(DenyAllResolver));

    for format in [SessionExportFormat::Jsonl, SessionExportFormat::Atif] {
        let err = ExportSessionMessages {
            session_id: "not-a-session-id".to_string(),
            format,
        }
        .run(&ctx)
        .await
        .expect_err("session export must require SESSION_VIEW");
        assert_forbidden(err);
    }
}

fn assert_forbidden(err: CommandError) {
    assert!(
        matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Forbidden(_),
                ..
            }
        ),
        "expected Forbidden, got {err:?}"
    );
}

#[tokio::test]
async fn run_honors_read_only_resolver() {
    // Owner with read-only resolver: read works, write blocked.
    let ctx = make_ctx(
        caller_with_role(OrgRole::Owner),
        Arc::new(HarnessReadOnlyResolver),
    );

    ListHarnesses {
        search: None,
        include_archived: false,
    }
    .run(&ctx)
    .await
    .expect("Read-only resolver must allow view");

    let err = CreateHarness(minimal_harness("ro-create"))
        .run(&ctx)
        .await
        .expect_err("Read-only resolver must block create");
    assert!(matches!(
        err,
        CommandError {
            kind: CommandErrorKind::Forbidden(_),
            ..
        }
    ));
}

// ============================================================================
// 3. dispatch() path (MCP / gRPC ExecuteCommand) enforces identically.
// ============================================================================

#[tokio::test]
async fn dispatch_blocks_member_from_manage_command() {
    let ctx = make_ctx(
        caller_with_role(OrgRole::Member),
        Arc::new(DefaultPermissionResolver),
    );
    let params = serde_json::json!({
        "name": "dispatch-member",
        "system_prompt": "test prompt",
    });
    let err = dispatch("create_harness", params, &ctx)
        .await
        .expect_err("dispatch must enforce Command::policy()");
    assert!(
        matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Forbidden(_),
                ..
            }
        ),
        "expected Forbidden from dispatch, got {err:?}"
    );
}

#[tokio::test]
async fn dispatch_honors_custom_resolver() {
    let ctx = make_ctx(caller_with_role(OrgRole::Owner), Arc::new(DenyAllResolver));
    let params = serde_json::json!({
        "name": "dispatch-saas",
        "system_prompt": "test prompt",
    });
    let err = dispatch("create_harness", params, &ctx)
        .await
        .expect_err("dispatch must consult custom resolver");
    assert!(matches!(
        err,
        CommandError {
            kind: CommandErrorKind::Forbidden(_),
            ..
        }
    ));
}

// ============================================================================
// 4. Inventory coverage — every non-GET command declares policy(). Catches
//    new write commands that forget enforcement. Keep ALLOWED_PUBLIC tiny
//    and justify each entry.
// ============================================================================

#[test]
fn every_non_readonly_command_declares_policy() {
    use everruns_server::domains::common::CommandDescriptor;

    let mut missing: Vec<&'static str> = Vec::new();
    for desc in inventory::iter::<CommandDescriptor> {
        let meta = (desc.meta)();
        let has_policy = (desc.policy)().is_some();
        let is_get = meta.method == "GET";
        if !is_get && !has_policy && !ALLOWED_PUBLIC.contains(&meta.name) {
            missing.push(meta.name);
        }
    }
    assert!(
        missing.is_empty(),
        "These mutating commands are missing a policy() declaration. \
         Every non-GET command must enforce authorization. Missing: {missing:?}"
    );
    assert!(
        !catalog_entries().is_empty(),
        "command catalog is empty — is inventory registration broken?"
    );
}

#[test]
fn mcp_query_read_only_overrides_are_allowlisted() {
    use everruns_server::domains::common::CommandDescriptor;

    let mut non_get_read_only: Vec<&'static str> = inventory::iter::<CommandDescriptor>
        .into_iter()
        .filter_map(|desc| {
            let meta = (desc.meta)();
            ((desc.read_only)() && meta.method != "GET").then_some(meta.name)
        })
        .collect();
    non_get_read_only.sort_unstable();

    let mut allowed = ALLOWED_NON_GET_READ_ONLY.to_vec();
    allowed.sort_unstable();

    assert_eq!(
        non_get_read_only, allowed,
        "MCP query exposes every read_only() command. Non-GET read-only overrides \
         must stay intentionally reviewed so mutating commands do not drift into query."
    );
}

/// Grants only session management — models custom deployments that let users
/// create sessions without allowing them to view or manage agents.
struct SessionsOnlyResolver;

impl PermissionResolver for SessionsOnlyResolver {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        matches!(permission, Permission::OrgSessionsManage)
    }
    fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
        vec![Permission::OrgSessionsManage]
    }
}

fn create_session_request() -> CreateSessionRequest {
    CreateSessionRequest {
        source: None,
        workspace_id: None,
        harness_id: None,
        harness_name: None,
        agent_id: None,
        agent_name: None,
        agent_identity_id: None,
        title: Some("Policy Test Session".to_string()),
        goal: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        parent_session_id: None,
        forked_from_session_id: None,
        budget_root_session_id: None,
        seed: SessionSeedMode::Fresh,
    }
}

async fn seed_agent(ctx: &Ctx, name: &str) -> AgentId {
    let harness = ctx
        .db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: format!("harness-{name}"),
                display_name: None,
                description: None,
                system_prompt: Some("test prompt".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .expect("seed harness");
    let public_id = AgentId::new();
    ctx.db
        .create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: public_id.to_string(),
                name: name.to_string(),
                display_name: None,
                description: None,
                system_prompt: "test agent".to_string(),
                default_model_id: None,
                harness_id: harness.id,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                is_built_in: false,
            },
        )
        .await
        .expect("seed agent");
    public_id
}

/// Grants OrgAgentsManage but denies OrgSessionsManage — models an eval-only
/// SaaS tier that can manage eval definitions but cannot start runs.
struct AgentsOnlyResolver;

impl PermissionResolver for AgentsOnlyResolver {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        matches!(permission, Permission::OrgAgentsManage)
    }
    fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
        vec![Permission::OrgAgentsManage]
    }
}

// ============================================================================
// EVE-549: CreateEvalRun must require EVAL_RUN (OrgAgentsManage +
//           OrgSessionsManage), not just EVAL_MANAGE (OrgAgentsManage).
// ============================================================================

#[tokio::test]
async fn session_creation_by_agent_name_requires_agent_permission() {
    let ctx = make_ctx(
        caller_with_role(OrgRole::Owner),
        Arc::new(SessionsOnlyResolver),
    );
    seed_agent(&ctx, "hidden-agent-by-name").await;

    let mut req = create_session_request();
    req.agent_name = Some("hidden-agent-by-name".to_string());

    let err = CreateSession(req)
        .run(&ctx)
        .await
        .expect_err("agent-name session creation must require agent visibility");
    assert_forbidden(err);
}

#[tokio::test]
async fn session_creation_by_agent_id_requires_agent_permission() {
    let ctx = make_ctx(
        caller_with_role(OrgRole::Owner),
        Arc::new(SessionsOnlyResolver),
    );
    let agent_id = seed_agent(&ctx, "hidden-agent-by-id").await;

    let mut req = create_session_request();
    req.agent_id = Some(agent_id);

    let err = CreateSession(req)
        .run(&ctx)
        .await
        .expect_err("agent-id session creation must require agent visibility");
    assert_forbidden(err);
}

#[tokio::test]
async fn eval_run_creation_requires_session_permission() {
    // Regression for EVE-549: a caller with eval management but denied
    // OrgSessionsManage must not be able to trigger real session creation.
    let ctx = make_ctx(
        caller_with_role(OrgRole::Owner),
        Arc::new(AgentsOnlyResolver),
    )
    .with_feature_flags(FeatureFlags {
        evals: true,
        ..Default::default()
    });
    let err = CreateEvalRun {
        eval_id: "eval_nonexistent".to_string(),
        req: CreateEvalRunRequest {
            target: None,
            model_override: None,
        },
    }
    .run(&ctx)
    .await
    .expect_err("CreateEvalRun must require OrgSessionsManage");
    assert_forbidden(err);
}

#[tokio::test]
async fn agent_health_check_requires_session_permission() {
    // Health checks launch real sessions/messages in the background, so an
    // agent manager without OrgSessionsManage must be blocked before agent
    // lookup or run creation.
    let ctx = make_ctx(
        caller_with_role(OrgRole::Owner),
        Arc::new(AgentsOnlyResolver),
    );
    let err = TriggerAgentHealthCheck {
        agent_id: "agent_nonexistent".to_string(),
    }
    .run(&ctx)
    .await
    .expect_err("TriggerAgentHealthCheck must require OrgSessionsManage");
    assert_forbidden(err);
}

#[tokio::test]
async fn eval_manage_without_session_permission_still_allows_list() {
    // A caller with only OrgAgentsManage can still list evals — the EVAL_VIEW
    // policy only requires OrgAgentsManage. Only run creation needs more.
    // Evals are feature-gated (require_evals_enabled), so enable the flag here;
    // this test asserts policy behavior, not the gate.
    let ctx = make_ctx(
        caller_with_role(OrgRole::Owner),
        Arc::new(AgentsOnlyResolver),
    )
    .with_feature_flags(FeatureFlags {
        evals: true,
        ..Default::default()
    });
    ListEvals {
        search: None,
        include_archived: false,
    }
    .run(&ctx)
    .await
    .expect("ListEvals must be allowed with only OrgAgentsManage");
}

// ============================================================================
// EVE-551: ListWorkspaceFiles and GetWorkspaceFile must enforce SESSION_VIEW.
// A resolver denying OrgSessionsManage must get 403 before any file lookup.
// (Commands were renamed from ListSessionFiles/GetSessionFile in #2189.)
// ============================================================================

#[tokio::test]
async fn session_file_read_commands_enforce_session_view_policy() {
    let ctx = make_ctx(caller_with_role(OrgRole::Owner), Arc::new(DenyAllResolver));

    let list_err = ListWorkspaceFiles {
        session_id: "session_00000000000000000000000000000001".to_string(),
        recursive: false,
    }
    .run(&ctx)
    .await
    .expect_err("ListWorkspaceFiles must require SESSION_VIEW");
    assert_forbidden(list_err);

    let get_err = GetWorkspaceFile {
        session_id: "session_00000000000000000000000000000001".to_string(),
        path: "/".to_string(),
        recursive: false,
    }
    .run(&ctx)
    .await
    .expect_err("GetWorkspaceFile must require SESSION_VIEW");
    assert_forbidden(get_err);
}

/// Commands intentionally exposed without a policy (public reads, probes).
/// Keep tiny; justify every entry in a comment.
const ALLOWED_PUBLIC: &[&str] = &[
    // (none today)
];

// ============================================================================
// Built-in agent protection coverage (EVE-865)
//
// A guard that must be remembered will be forgotten the next time someone adds
// an agent command. This test walks the inventory instead: every mutating
// command in the `agents` category must be classified as either guarded
// against built-in agents or deliberately exempt. Adding an eleventh agent
// command fails CI here until its author decides which it is.
//
// This is a classification check, not a behavioral one — the behavior is
// asserted per verb in `domains::agents::commands::tests`.
// ============================================================================

/// Agent commands that reject built-in agents via `q::ensure_not_built_in`.
/// Each has a matching `built_in_agent_rejects_*` test.
const BUILT_IN_GUARDED_AGENT_COMMANDS: &[&str] = &[
    "create_agent_version",
    "delete_agent",
    "destroy_agent",
    "rollback_agent_version",
    "set_default_agent_version",
    "update_agent",
    "upsert_agent",
];

/// Agent commands that are deliberately NOT guarded. Justify every entry.
///
/// The rule is definition vs bindings: a command that changes a built-in
/// agent's own definition is guarded; one that only creates something new, or
/// attaches an org's own binding around the agent, is not.
const BUILT_IN_EXEMPT_AGENT_COMMANDS: &[&str] = &[
    // Creates a fresh agent; cannot mint a built-in (is_built_in is false on
    // every API-facing creation path).
    "create_agent",
    // Same — import is create with a definition body.
    "import_agent",
    // The escape hatch. Blocking copy would leave built-in agents unusable as
    // a starting point, and the rejection message tells users to copy first.
    "copy_agent",
    // Forks a *source version* into a brand-new agent. It reads the built-in
    // and writes elsewhere, so it is a copy, not a mutation — same reasoning
    // as copy_agent. (EVE-865 listed this as needing a guard; the code shows
    // it never writes to the source row.)
    "fork_agent_version",
    // --- Bindings, not definition -------------------------------------------
    // These live in adjacent domains and never touch the agents row. An org
    // must be able to wire a built-in agent into its own environment; a
    // built-in agent nobody can attach a rule or credential to is unusable.
    "upsert_agent_check_rule",
    "delete_agent_check_rule",
    "create_agent_credential_binding",
    // Operational probe. Runs the agent to report health; changes no definition.
    "trigger_agent_health_check",
    // Produces an analysis report. Non-read-only only because it spends utility
    // LLM budget and is rate limited; it does not write the agent.
    "analyze_agent",
];

/// The guard lives in `Command::execute`, so it must hold over `dispatch()` —
/// the MCP `execute` tier and gRPC `ExecuteCommand` both route through here.
/// A route-level check would pass the REST test and miss this one entirely,
/// leaving the built-in agent editable by any caller holding agent-management
/// access over MCP — including the agent itself.
#[tokio::test]
async fn dispatch_blocks_built_in_agent_mutation() {
    let ctx = make_ctx(
        caller_with_role(OrgRole::Owner),
        Arc::new(DefaultPermissionResolver),
    );
    let agent_id = seed_agent(&ctx, "platform-chat").await;
    let row = ctx
        .db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &agent_id.to_string())
        .await
        .expect("load seeded agent")
        .expect("seeded agent exists");
    ctx.db
        .mark_agent_built_in(DEFAULT_ORG_ID, row.id)
        .await
        .expect("mark built-in");

    let err = dispatch(
        "update_agent",
        serde_json::json!({ "id": agent_id.to_string(), "system_prompt": "hijacked" }),
        &ctx,
    )
    .await
    .expect_err("MCP execute must not edit a built-in agent");

    assert!(
        err.message().contains("built-in agent"),
        "expected the built-in rejection over dispatch, got {err:?}"
    );

    // An Owner is allowed to update agents in general, so this must be the
    // built-in guard talking, not a permission denial.
    assert!(
        !matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Forbidden(_),
                ..
            }
        ),
        "rejection must come from the built-in guard, not authorization: {err:?}"
    );
}

#[test]
fn every_mutating_agent_command_is_classified_for_built_in_protection() {
    use everruns_server::domains::common::CommandDescriptor;

    let mut unclassified: Vec<&'static str> = Vec::new();
    let mut mutating: Vec<&'static str> = Vec::new();

    for desc in inventory::iter::<CommandDescriptor> {
        let meta = (desc.meta)();
        if meta.category != "agents" || (desc.read_only)() {
            continue;
        }
        mutating.push(meta.name);
        if !BUILT_IN_GUARDED_AGENT_COMMANDS.contains(&meta.name)
            && !BUILT_IN_EXEMPT_AGENT_COMMANDS.contains(&meta.name)
        {
            unclassified.push(meta.name);
        }
    }

    assert!(
        !mutating.is_empty(),
        "no mutating agents commands found — is inventory registration broken?"
    );
    assert!(
        unclassified.is_empty(),
        "New mutating agent command(s) {unclassified:?} are not classified for \
         built-in agent protection. Either call `q::ensure_not_built_in` and add \
         the name to BUILT_IN_GUARDED_AGENT_COMMANDS with a \
         `built_in_agent_rejects_*` test, or add it to \
         BUILT_IN_EXEMPT_AGENT_COMMANDS with a written reason. Guard at the \
         command layer, not the HTTP route — MCP `execute` dispatches the same \
         descriptors."
    );

    // Stale entries are as dangerous as missing ones: a renamed command would
    // leave a guard listed that no longer exists.
    for name in BUILT_IN_GUARDED_AGENT_COMMANDS
        .iter()
        .chain(BUILT_IN_EXEMPT_AGENT_COMMANDS)
    {
        assert!(
            mutating.contains(name),
            "'{name}' is classified for built-in protection but is no longer a \
             mutating agents command. Remove the stale entry."
        );
    }
}

/// POST-style helpers intentionally available through MCP `query`.
/// They do not persist state and each still declares a view policy.
const ALLOWED_NON_GET_READ_ONLY: &[&str] = &[
    "export_report_query",
    "export_saved_report",
    "grep_workspace_files",
    "preview_agent",
    "preview_harness",
    "run_report_query",
    "run_saved_report",
    "stat_workspace_file",
];
