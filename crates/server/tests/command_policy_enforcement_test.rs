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
    Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DefaultPermissionResolver, OrgRole, Permission,
    PermissionResolver,
};
use everruns_server::domains::common::{Command, CommandError, Ctx, catalog_entries, dispatch};
use everruns_server::domains::harnesses::types::CreateHarnessRequest;
use everruns_server::domains::harnesses::{CreateHarness, ListHarnesses};
use everruns_server::services::CapabilityService;
use everruns_server::storage::StorageBackend;
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
        system_prompt: "test prompt".to_string(),
        parent_harness_id: None,
        default_model_id: None,
        tags: Vec::new(),
        capabilities: Vec::new(),
        initial_files: Vec::new(),
        mcp_servers: Default::default(),
        network_access: None,
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
        matches!(err, CommandError::Forbidden(_)),
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
        matches!(err, CommandError::Forbidden(_)),
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
    assert!(matches!(err, CommandError::Forbidden(_)));
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
        matches!(err, CommandError::Forbidden(_)),
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
    assert!(matches!(err, CommandError::Forbidden(_)));
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

/// Commands intentionally exposed without a policy (public reads, probes).
/// Keep tiny; justify every entry in a comment.
const ALLOWED_PUBLIC: &[&str] = &[
    // (none today)
];

/// POST-style helpers intentionally available through MCP `query`.
/// They do not persist state and each still declares a view policy.
const ALLOWED_NON_GET_READ_ONLY: &[&str] = &[
    "grep_session_files",
    "preview_agent",
    "preview_harness",
    "stat_session_file",
];
