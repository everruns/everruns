// Permissions model for fine-grained access control
//
// Decision: Default permissions are hardcoded per OrgRole; custom resolvers can override evaluation.
// Decision: Policies are const values evaluated at service method entry via #[policy] macro.
// Decision: Permission format is `org:<resource>:<action>`.
// See specs/permissions.md for full design.

use crate::organization::OrgRole;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

/// A permission identifier representing an action on a resource.
///
/// Format: `org:<resource>:<action>`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// View harnesses (read-only)
    OrgHarnessesView,
    /// CRUD on harnesses
    OrgHarnessesManage,
    /// Delete, reset, and other dangerous harness operations
    OrgHarnessesDangerous,
    /// CRUD on agents
    OrgAgentsManage,
    /// Delete and other dangerous agent operations
    OrgAgentsDangerous,
    /// Dangerous skill operations
    OrgSkillsDangerous,
    /// Dangerous MCP server operations
    OrgMcpServersDangerous,
    /// Dangerous app operations
    OrgAppsDangerous,
    /// CRUD on sessions
    OrgSessionsManage,
    /// View LLM providers (read-only)
    OrgLlmProvidersView,
    /// CRUD on LLM providers
    OrgLlmProvidersManage,
    /// View organization settings (read-only)
    OrgSettingsView,
    /// Organization settings
    OrgSettingsManage,
    /// View members (read-only)
    OrgMembersView,
    /// Invite/remove members
    OrgMembersManage,
    /// CRUD on API keys
    OrgApiKeysManage,
}

impl Permission {
    /// String identifier for this permission.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Permission::OrgHarnessesView => "org:harnesses:view",
            Permission::OrgHarnessesManage => "org:harnesses:manage",
            Permission::OrgHarnessesDangerous => "org:harnesses:dangerous",
            Permission::OrgAgentsManage => "org:agents:manage",
            Permission::OrgAgentsDangerous => "org:agents:dangerous",
            Permission::OrgSkillsDangerous => "org:skills:dangerous",
            Permission::OrgMcpServersDangerous => "org:mcp-servers:dangerous",
            Permission::OrgAppsDangerous => "org:apps:dangerous",
            Permission::OrgSessionsManage => "org:sessions:manage",
            Permission::OrgLlmProvidersView => "org:llm-providers:view",
            Permission::OrgLlmProvidersManage => "org:llm-providers:manage",
            Permission::OrgSettingsView => "org:settings:view",
            Permission::OrgSettingsManage => "org:settings:manage",
            Permission::OrgMembersView => "org:members:view",
            Permission::OrgMembersManage => "org:members:manage",
            Permission::OrgApiKeysManage => "org:api-keys:manage",
        }
    }

    /// All defined permissions.
    pub const ALL: &'static [Permission] = &[
        Permission::OrgHarnessesView,
        Permission::OrgHarnessesManage,
        Permission::OrgHarnessesDangerous,
        Permission::OrgAgentsManage,
        Permission::OrgAgentsDangerous,
        Permission::OrgSkillsDangerous,
        Permission::OrgMcpServersDangerous,
        Permission::OrgAppsDangerous,
        Permission::OrgSessionsManage,
        Permission::OrgLlmProvidersView,
        Permission::OrgLlmProvidersManage,
        Permission::OrgSettingsView,
        Permission::OrgSettingsManage,
        Permission::OrgMembersView,
        Permission::OrgMembersManage,
        Permission::OrgApiKeysManage,
    ];
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// Role → Permission mapping
// ============================================================================

/// Permissions granted to Owner role.
const OWNER_PERMISSIONS: &[Permission] = &[
    Permission::OrgHarnessesView,
    Permission::OrgHarnessesManage,
    Permission::OrgHarnessesDangerous,
    Permission::OrgAgentsManage,
    Permission::OrgAgentsDangerous,
    Permission::OrgSkillsDangerous,
    Permission::OrgMcpServersDangerous,
    Permission::OrgAppsDangerous,
    Permission::OrgSessionsManage,
    Permission::OrgLlmProvidersView,
    Permission::OrgLlmProvidersManage,
    Permission::OrgSettingsView,
    Permission::OrgSettingsManage,
    Permission::OrgMembersView,
    Permission::OrgMembersManage,
    Permission::OrgApiKeysManage,
];

/// Permissions granted to Admin role.
const ADMIN_PERMISSIONS: &[Permission] = &[
    Permission::OrgHarnessesView,
    Permission::OrgHarnessesManage,
    Permission::OrgAgentsManage,
    Permission::OrgSessionsManage,
    Permission::OrgLlmProvidersView,
    Permission::OrgLlmProvidersManage,
    Permission::OrgSettingsView,
    Permission::OrgSettingsManage,
    Permission::OrgMembersView,
    Permission::OrgMembersManage,
    Permission::OrgApiKeysManage,
];

/// Permissions granted to Member role.
/// Members can view all resources but only manage agents and sessions.
const MEMBER_PERMISSIONS: &[Permission] = &[
    Permission::OrgHarnessesView,
    Permission::OrgAgentsManage,
    Permission::OrgSessionsManage,
    Permission::OrgLlmProvidersView,
    Permission::OrgSettingsView,
    Permission::OrgMembersView,
];

/// Check whether a role grants a specific permission.
pub fn role_has_permission(role: OrgRole, permission: &Permission) -> bool {
    let perms = match role {
        OrgRole::Owner => OWNER_PERMISSIONS,
        OrgRole::Admin => ADMIN_PERMISSIONS,
        OrgRole::Member => MEMBER_PERMISSIONS,
    };
    perms.contains(permission)
}

/// List all permissions granted to a role.
pub fn role_permissions(role: OrgRole) -> &'static [Permission] {
    match role {
        OrgRole::Owner => OWNER_PERMISSIONS,
        OrgRole::Admin => ADMIN_PERMISSIONS,
        OrgRole::Member => MEMBER_PERMISSIONS,
    }
}

/// Contract for resolving which permissions a caller has.
///
/// `DefaultPermissionResolver` preserves the OSS role-based mapping, while downstream
/// consumers can implement this trait to inject billing-tier rules, database-backed
/// grants, or external RBAC systems without patching policy evaluation itself.
///
/// # Contract
///
/// Implementations must:
///
/// - Fail closed. Return `true` from `has_permission()` only when the caller is
///   actually authorized.
/// - Be consistent. If `has_permission(caller, permission)` returns `true`, then
///   `caller_permissions(caller)` must include that same permission.
/// - Be safe to call from async request paths (`Send + Sync`).
///
/// # Example
///
/// ```no_run
/// use everruns_core::{Caller, Permission, PermissionResolver};
///
/// struct TierAwareResolver;
///
/// impl PermissionResolver for TierAwareResolver {
///     fn has_permission(&self, caller: &Caller, permission: &Permission) -> bool {
///         caller.role == everruns_core::organization::OrgRole::Owner
///             && permission == &Permission::OrgHarnessesDangerous
///     }
///
///     fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
///         Permission::ALL
///             .iter()
///             .copied()
///             .filter(|permission| self.has_permission(caller, permission))
///             .collect()
///     }
/// }
/// ```
pub trait PermissionResolver: Send + Sync {
    /// Return whether the caller currently has `permission`.
    ///
    /// This method is used by `Policy::evaluate_with()` for
    /// `Rule::UserHasPermission` checks.
    fn has_permission(&self, caller: &Caller, permission: &Permission) -> bool;

    /// Return the full set of permissions currently granted to `caller`.
    ///
    /// Config endpoints and downstream policy-reporting code use this to expose
    /// evaluated capabilities to a UI. Prefer deterministic ordering so callers
    /// can compare results reliably.
    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission>;
}

/// Default `PermissionResolver` backed by the built-in role-to-permission map.
///
/// This preserves the existing OSS behavior by delegating to
/// `role_has_permission()` and `role_permissions()`.
///
/// # Safety
///
/// This resolver is stateless and can be shared freely across threads.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultPermissionResolver;

impl PermissionResolver for DefaultPermissionResolver {
    fn has_permission(&self, caller: &Caller, permission: &Permission) -> bool {
        role_has_permission(caller.role, permission)
    }

    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        role_permissions(caller.role).to_vec()
    }
}

// ============================================================================
// Rule
// ============================================================================

/// A single predicate evaluated against a Caller context.
/// All rules in a Policy must pass (AND logic).
#[derive(Debug, Clone)]
pub enum Rule {
    /// Caller's role must grant this permission.
    UserHasPermission(Permission),
    /// Caller must have at least this OrgRole level.
    UserHasRole(OrgRole),
    /// Caller must be a platform user (allowlisted email).
    IsPlatformUser,
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rule::UserHasPermission(p) => write!(f, "UserHasPermission({})", p),
            Rule::UserHasRole(r) => write!(f, "UserHasRole({})", r),
            Rule::IsPlatformUser => write!(f, "IsPlatformUser"),
        }
    }
}

// ============================================================================
// Policy
// ============================================================================

/// A named set of rules that must all pass for access.
///
/// Policies are defined as `const` values and attached to service methods
/// via the `#[policy]` macro.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Unique identifier for this policy (e.g. "harness.manage").
    pub id: &'static str,
    /// Rules that must all pass. AND logic.
    pub rules: &'static [Rule],
}

impl Policy {
    /// Evaluate all rules against the caller. Returns `Ok(())` if all pass,
    /// or `Err(PolicyError)` on the first failing rule.
    pub fn evaluate(&self, caller: &Caller) -> Result<(), PolicyError> {
        let resolver = DefaultPermissionResolver;
        self.evaluate_with(&resolver, caller)
    }

    /// Evaluate all rules against the caller using a custom permission resolver.
    ///
    /// `Rule::UserHasPermission` checks are delegated to `resolver`, while other
    /// rule types keep their current built-in behavior.
    pub fn evaluate_with(
        &self,
        resolver: &dyn PermissionResolver,
        caller: &Caller,
    ) -> Result<(), PolicyError> {
        for rule in self.rules {
            match rule {
                Rule::UserHasPermission(perm) => {
                    if !resolver.has_permission(caller, perm) {
                        return Err(PolicyError::denied(self.id, perm.as_str()));
                    }
                }
                Rule::UserHasRole(required) => {
                    if !caller.role.has_permission(*required) {
                        return Err(PolicyError::denied(self.id, &format!("role:{}", required)));
                    }
                }
                Rule::IsPlatformUser => {
                    if !caller.is_platform_user {
                        return Err(PolicyError::denied(self.id, "platform_user"));
                    }
                }
            }
        }
        Ok(())
    }
}

// ============================================================================
// Caller
// ============================================================================

/// Auth context passed from API handler to service layer.
///
/// Replaces raw `org_id: i64` parameter on service methods. Carries all
/// information needed for policy evaluation.
#[derive(Debug, Clone)]
pub struct Caller {
    /// Internal organization ID (for database queries).
    pub org_id: i64,
    /// External organization public ID.
    pub org_public_id: String,
    /// Authenticated user ID (`None` for API key auth without user context).
    pub user_id: Option<Uuid>,
    /// User's role in the organization.
    pub role: OrgRole,
    /// Whether the caller is a platform user (email allowlist).
    pub is_platform_user: bool,
}

impl Caller {
    /// Create an internal/platform caller with Owner role.
    ///
    /// Used for gRPC service calls (worker ↔ server) and other internal
    /// operations that should bypass all policy checks.
    // THREAT[TM-AUTHZ-002]: Internal caller bypasses policies; only use for gRPC/internal paths
    pub fn internal(org_id: i64) -> Self {
        Self {
            org_id,
            org_public_id: crate::organization::org_public_id_from_internal(org_id),
            user_id: None,
            role: OrgRole::Owner,
            is_platform_user: true,
        }
    }
}

// ============================================================================
// PolicyError
// ============================================================================

/// Error returned when a policy evaluation fails.
#[derive(Debug, Clone)]
pub struct PolicyError {
    /// Which policy failed.
    pub policy_id: String,
    /// Human-readable message.
    pub message: String,
}

impl PolicyError {
    pub fn denied(policy_id: &str, detail: &str) -> Self {
        Self {
            policy_id: policy_id.to_string(),
            message: format!(
                "Access denied: policy '{}' requires '{}'",
                policy_id, detail
            ),
        }
    }
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PolicyError {}

// ============================================================================
// Policy evaluation helpers (for config endpoints)
// ============================================================================

/// Evaluate multiple policies against a caller and return a map of results.
/// Used by `/config` endpoints to expose policy results to the UI.
pub fn evaluate_policies(caller: &Caller, policies: &[&Policy]) -> HashMap<String, bool> {
    let resolver = DefaultPermissionResolver;
    evaluate_policies_with(&resolver, caller, policies)
}

/// Evaluate multiple policies using a custom permission resolver.
///
/// This is the config-endpoint companion to `Policy::evaluate_with()`.
pub fn evaluate_policies_with(
    resolver: &dyn PermissionResolver,
    caller: &Caller,
    policies: &[&Policy],
) -> HashMap<String, bool> {
    policies
        .iter()
        .map(|policy| {
            (
                policy.id.to_string(),
                policy.evaluate_with(resolver, caller).is_ok(),
            )
        })
        .collect()
}

/// Response type for per-resource config endpoints.
///
/// Every resource exposes `GET /v1/{resource}/config` returning this type.
/// UI uses it to gate controls (create/edit/delete buttons, admin panels).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResourceConfigResponse {
    /// Map of policy ID → whether the caller satisfies it.
    pub policies: HashMap<String, bool>,
}

/// Backwards compat alias — prefer `ResourceConfigResponse`.
pub type PolicyConfigResponse = ResourceConfigResponse;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn owner_caller() -> Caller {
        Caller {
            org_id: 1,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::new_v4()),
            role: OrgRole::Owner,
            is_platform_user: false,
        }
    }

    fn admin_caller() -> Caller {
        Caller {
            org_id: 1,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::new_v4()),
            role: OrgRole::Admin,
            is_platform_user: false,
        }
    }

    fn member_caller() -> Caller {
        Caller {
            org_id: 1,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::new_v4()),
            role: OrgRole::Member,
            is_platform_user: false,
        }
    }

    // -- role_has_permission tests --

    #[test]
    fn owner_has_all_permissions() {
        for perm in Permission::ALL {
            assert!(
                role_has_permission(OrgRole::Owner, perm),
                "Owner should have {:?}",
                perm
            );
        }
    }

    #[test]
    fn admin_has_manage_but_not_dangerous() {
        assert!(role_has_permission(
            OrgRole::Admin,
            &Permission::OrgHarnessesManage
        ));
        assert!(!role_has_permission(
            OrgRole::Admin,
            &Permission::OrgHarnessesDangerous
        ));
        assert!(role_has_permission(
            OrgRole::Admin,
            &Permission::OrgAgentsManage
        ));
        assert!(role_has_permission(
            OrgRole::Admin,
            &Permission::OrgSettingsManage
        ));
    }

    #[test]
    fn member_has_only_basic_permissions() {
        assert!(role_has_permission(
            OrgRole::Member,
            &Permission::OrgAgentsManage
        ));
        assert!(role_has_permission(
            OrgRole::Member,
            &Permission::OrgSessionsManage
        ));
        assert!(!role_has_permission(
            OrgRole::Member,
            &Permission::OrgHarnessesManage
        ));
        assert!(!role_has_permission(
            OrgRole::Member,
            &Permission::OrgSettingsManage
        ));
        assert!(!role_has_permission(
            OrgRole::Member,
            &Permission::OrgApiKeysManage
        ));
    }

    // -- Policy evaluation tests --

    const TEST_MANAGE: Policy = Policy {
        id: "harness.manage",
        rules: &[Rule::UserHasPermission(Permission::OrgHarnessesManage)],
    };

    const TEST_DANGEROUS: Policy = Policy {
        id: "harness.dangerous",
        rules: &[
            Rule::UserHasPermission(Permission::OrgHarnessesManage),
            Rule::UserHasPermission(Permission::OrgHarnessesDangerous),
        ],
    };

    const TEST_ROLE_ADMIN: Policy = Policy {
        id: "require.admin",
        rules: &[Rule::UserHasRole(OrgRole::Admin)],
    };

    #[test]
    fn owner_passes_all_policies() {
        let caller = owner_caller();
        assert!(TEST_MANAGE.evaluate(&caller).is_ok());
        assert!(TEST_DANGEROUS.evaluate(&caller).is_ok());
        assert!(TEST_ROLE_ADMIN.evaluate(&caller).is_ok());
    }

    #[test]
    fn admin_passes_manage_but_not_dangerous() {
        let caller = admin_caller();
        assert!(TEST_MANAGE.evaluate(&caller).is_ok());
        assert!(TEST_DANGEROUS.evaluate(&caller).is_err());
        assert!(TEST_ROLE_ADMIN.evaluate(&caller).is_ok());
    }

    #[test]
    fn member_fails_manage_and_dangerous() {
        let caller = member_caller();
        assert!(TEST_MANAGE.evaluate(&caller).is_err());
        assert!(TEST_DANGEROUS.evaluate(&caller).is_err());
        assert!(TEST_ROLE_ADMIN.evaluate(&caller).is_err());
    }

    #[test]
    fn policy_error_contains_useful_info() {
        let caller = member_caller();
        let err = TEST_MANAGE.evaluate(&caller).unwrap_err();
        assert_eq!(err.policy_id, "harness.manage");
        assert!(err.message.contains("org:harnesses:manage"));
        assert!(err.to_string().contains("Access denied"));
    }

    // -- evaluate_policies (config endpoint helper) tests --

    #[test]
    fn evaluate_policies_returns_correct_map() {
        let caller = admin_caller();
        let result = evaluate_policies(&caller, &[&TEST_MANAGE, &TEST_DANGEROUS]);

        assert_eq!(result.get("harness.manage"), Some(&true));
        assert_eq!(result.get("harness.dangerous"), Some(&false));
    }

    #[test]
    fn evaluate_policies_owner_all_true() {
        let caller = owner_caller();
        let result = evaluate_policies(&caller, &[&TEST_MANAGE, &TEST_DANGEROUS, &TEST_ROLE_ADMIN]);

        assert!(result.values().all(|&v| v));
    }

    #[test]
    fn evaluate_policies_member_all_false_for_admin_policies() {
        let caller = member_caller();
        let result = evaluate_policies(&caller, &[&TEST_MANAGE, &TEST_DANGEROUS, &TEST_ROLE_ADMIN]);

        assert!(result.values().all(|&v| !v));
    }

    #[test]
    fn default_permission_resolver_matches_hardcoded_role_mapping() {
        let resolver = DefaultPermissionResolver;

        for caller in [owner_caller(), admin_caller(), member_caller()] {
            for permission in Permission::ALL {
                assert_eq!(
                    resolver.has_permission(&caller, permission),
                    role_has_permission(caller.role, permission)
                );
            }

            assert_eq!(
                resolver.caller_permissions(&caller),
                role_permissions(caller.role).to_vec()
            );
        }
    }

    struct DenyManageResolver;

    impl PermissionResolver for DenyManageResolver {
        fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
            permission != &Permission::OrgHarnessesManage
        }

        fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
            Permission::ALL
                .iter()
                .copied()
                .filter(|permission| permission != &Permission::OrgHarnessesManage)
                .collect()
        }
    }

    #[test]
    fn evaluate_with_uses_custom_permission_resolver() {
        let caller = owner_caller();
        let resolver = DenyManageResolver;

        assert!(TEST_MANAGE.evaluate(&caller).is_ok());
        assert!(TEST_MANAGE.evaluate_with(&resolver, &caller).is_err());
    }

    #[test]
    fn evaluate_policies_with_uses_custom_permission_resolver() {
        let caller = owner_caller();
        let resolver = DenyManageResolver;

        let result = evaluate_policies_with(&resolver, &caller, &[&TEST_MANAGE, &TEST_DANGEROUS]);

        assert_eq!(result.get("harness.manage"), Some(&false));
        assert_eq!(result.get("harness.dangerous"), Some(&false));
    }

    // -- Permission display --

    #[test]
    fn permission_display() {
        assert_eq!(
            Permission::OrgHarnessesManage.to_string(),
            "org:harnesses:manage"
        );
        assert_eq!(
            Permission::OrgHarnessesDangerous.to_string(),
            "org:harnesses:dangerous"
        );
        assert_eq!(Permission::OrgAgentsManage.to_string(), "org:agents:manage");
    }

    // -- role_permissions --

    #[test]
    fn role_permissions_returns_correct_sets() {
        assert_eq!(role_permissions(OrgRole::Owner).len(), 16);
        assert_eq!(role_permissions(OrgRole::Admin).len(), 11);
        assert_eq!(role_permissions(OrgRole::Member).len(), 6);
    }

    // -- Caller --

    #[test]
    fn caller_without_user_id() {
        let caller = Caller {
            org_id: 1,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: None,
            role: OrgRole::Admin,
            is_platform_user: false,
        };
        // API key callers without user_id should still evaluate policies
        assert!(TEST_MANAGE.evaluate(&caller).is_ok());
    }

    // -- Edge cases --

    #[test]
    fn empty_policy_always_passes() {
        const EMPTY: Policy = Policy {
            id: "empty",
            rules: &[],
        };
        let caller = member_caller();
        assert!(EMPTY.evaluate(&caller).is_ok());
    }

    #[test]
    fn caller_internal_has_owner_role() {
        let caller = Caller::internal(42);
        assert_eq!(caller.org_id, 42);
        assert_eq!(caller.role, OrgRole::Owner);
        assert!(caller.user_id.is_none());
        // Internal callers should pass all policies
        assert!(TEST_MANAGE.evaluate(&caller).is_ok());
        assert!(TEST_DANGEROUS.evaluate(&caller).is_ok());
        assert!(TEST_ROLE_ADMIN.evaluate(&caller).is_ok());
    }

    #[test]
    fn caller_internal_generates_public_id() {
        let caller = Caller::internal(1);
        assert_eq!(caller.org_public_id, "org_00000000000000000000000000000001");

        let caller = Caller::internal(99);
        assert!(caller.org_public_id.starts_with("org_"));
    }

    #[test]
    fn policy_error_is_std_error() {
        let err = PolicyError::denied("test", "detail");
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn policy_error_downcast_from_anyhow() {
        let err = PolicyError::denied("test.policy", "org:harnesses:manage");
        let anyhow_err: anyhow::Error = err.into();
        let downcasted = anyhow_err.downcast_ref::<PolicyError>();
        assert!(downcasted.is_some());
        assert_eq!(downcasted.unwrap().policy_id, "test.policy");
    }

    // -- View permissions --

    #[test]
    fn member_has_view_permissions() {
        assert!(role_has_permission(
            OrgRole::Member,
            &Permission::OrgHarnessesView
        ));
        assert!(role_has_permission(
            OrgRole::Member,
            &Permission::OrgLlmProvidersView
        ));
        assert!(role_has_permission(
            OrgRole::Member,
            &Permission::OrgSettingsView
        ));
        assert!(role_has_permission(
            OrgRole::Member,
            &Permission::OrgMembersView
        ));
    }

    #[test]
    fn member_lacks_manage_for_restricted_resources() {
        assert!(!role_has_permission(
            OrgRole::Member,
            &Permission::OrgHarnessesManage
        ));
        assert!(!role_has_permission(
            OrgRole::Member,
            &Permission::OrgLlmProvidersManage
        ));
        assert!(!role_has_permission(
            OrgRole::Member,
            &Permission::OrgSettingsManage
        ));
        assert!(!role_has_permission(
            OrgRole::Member,
            &Permission::OrgMembersManage
        ));
    }

    // -- IsPlatformUser rule --

    const TEST_PLATFORM: Policy = Policy {
        id: "durable.manage",
        rules: &[Rule::IsPlatformUser],
    };

    #[test]
    fn platform_user_passes_platform_policy() {
        let mut caller = owner_caller();
        caller.is_platform_user = true;
        assert!(TEST_PLATFORM.evaluate(&caller).is_ok());
    }

    #[test]
    fn non_platform_user_fails_platform_policy() {
        let caller = owner_caller(); // is_platform_user = false
        assert!(TEST_PLATFORM.evaluate(&caller).is_err());
    }

    #[test]
    fn internal_caller_is_platform_user() {
        let caller = Caller::internal(1);
        assert!(caller.is_platform_user);
        assert!(TEST_PLATFORM.evaluate(&caller).is_ok());
    }

    // -- ResourceConfigResponse serialization --

    #[test]
    fn resource_config_response_serializes() {
        let mut policies = HashMap::new();
        policies.insert("harness.manage".to_string(), true);
        policies.insert("harness.dangerous".to_string(), false);
        let response = ResourceConfigResponse { policies };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["policies"]["harness.manage"], true);
        assert_eq!(json["policies"]["harness.dangerous"], false);
    }
}
