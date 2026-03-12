// Permissions model for fine-grained access control
//
// Decision: Permissions are hardcoded per OrgRole (no DB storage in phase 1).
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
    /// CRUD on harnesses
    OrgHarnessesManage,
    /// Delete, reset, and other dangerous harness operations
    OrgHarnessesDangerous,
    /// CRUD on agents
    OrgAgentsManage,
    /// CRUD on sessions
    OrgSessionsManage,
    /// CRUD on LLM providers
    OrgLlmProvidersManage,
    /// Organization settings
    OrgSettingsManage,
    /// Invite/remove members
    OrgMembersManage,
    /// CRUD on API keys
    OrgApiKeysManage,
}

impl Permission {
    /// String identifier for this permission.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Permission::OrgHarnessesManage => "org:harnesses:manage",
            Permission::OrgHarnessesDangerous => "org:harnesses:dangerous",
            Permission::OrgAgentsManage => "org:agents:manage",
            Permission::OrgSessionsManage => "org:sessions:manage",
            Permission::OrgLlmProvidersManage => "org:llm-providers:manage",
            Permission::OrgSettingsManage => "org:settings:manage",
            Permission::OrgMembersManage => "org:members:manage",
            Permission::OrgApiKeysManage => "org:api-keys:manage",
        }
    }

    /// All defined permissions.
    pub const ALL: &'static [Permission] = &[
        Permission::OrgHarnessesManage,
        Permission::OrgHarnessesDangerous,
        Permission::OrgAgentsManage,
        Permission::OrgSessionsManage,
        Permission::OrgLlmProvidersManage,
        Permission::OrgSettingsManage,
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
    Permission::OrgHarnessesManage,
    Permission::OrgHarnessesDangerous,
    Permission::OrgAgentsManage,
    Permission::OrgSessionsManage,
    Permission::OrgLlmProvidersManage,
    Permission::OrgSettingsManage,
    Permission::OrgMembersManage,
    Permission::OrgApiKeysManage,
];

/// Permissions granted to Admin role.
const ADMIN_PERMISSIONS: &[Permission] = &[
    Permission::OrgHarnessesManage,
    Permission::OrgAgentsManage,
    Permission::OrgSessionsManage,
    Permission::OrgLlmProvidersManage,
    Permission::OrgSettingsManage,
    Permission::OrgMembersManage,
    Permission::OrgApiKeysManage,
];

/// Permissions granted to Member role.
const MEMBER_PERMISSIONS: &[Permission] =
    &[Permission::OrgAgentsManage, Permission::OrgSessionsManage];

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
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rule::UserHasPermission(p) => write!(f, "UserHasPermission({})", p),
            Rule::UserHasRole(r) => write!(f, "UserHasRole({})", r),
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
        for rule in self.rules {
            match rule {
                Rule::UserHasPermission(perm) => {
                    if !role_has_permission(caller.role, perm) {
                        return Err(PolicyError::denied(self.id, perm.as_str()));
                    }
                }
                Rule::UserHasRole(required) => {
                    if !caller.role.has_permission(*required) {
                        return Err(PolicyError::denied(self.id, &format!("role:{}", required)));
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
}

impl Caller {
    /// Create an internal/platform caller with Owner role.
    ///
    /// Used for gRPC service calls (worker ↔ server) and other internal
    /// operations that should bypass all policy checks.
    pub fn internal(org_id: i64) -> Self {
        Self {
            org_id,
            org_public_id: crate::organization::org_public_id_from_internal(org_id),
            user_id: None,
            role: OrgRole::Owner,
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
    policies
        .iter()
        .map(|p| (p.id.to_string(), p.evaluate(caller).is_ok()))
        .collect()
}

/// Response type for config endpoints.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PolicyConfigResponse {
    /// Map of policy ID → whether the caller satisfies it.
    pub policies: HashMap<String, bool>,
}

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
        }
    }

    fn admin_caller() -> Caller {
        Caller {
            org_id: 1,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::new_v4()),
            role: OrgRole::Admin,
        }
    }

    fn member_caller() -> Caller {
        Caller {
            org_id: 1,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::new_v4()),
            role: OrgRole::Member,
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
        assert_eq!(role_permissions(OrgRole::Owner).len(), 8);
        assert_eq!(role_permissions(OrgRole::Admin).len(), 7);
        assert_eq!(role_permissions(OrgRole::Member).len(), 2);
    }

    // -- Caller --

    #[test]
    fn caller_without_user_id() {
        let caller = Caller {
            org_id: 1,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: None,
            role: OrgRole::Admin,
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

    // -- PolicyConfigResponse serialization --

    #[test]
    fn policy_config_response_serializes() {
        let mut policies = HashMap::new();
        policies.insert("harness.manage".to_string(), true);
        policies.insert("harness.dangerous".to_string(), false);
        let response = PolicyConfigResponse { policies };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["policies"]["harness.manage"], true);
        assert_eq!(json["policies"]["harness.dangerous"], false);
    }
}
