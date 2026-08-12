// Organization types for multitenancy
// See knowledge/security/multitenancy.md
//
// Decision: Hierarchical org roles (Owner > Admin > Member) using PartialOrd.
// External auth providers map their roles to OrgRole via AuthBackend trait.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

// EVE-837: the `Organization` aggregate entity moved to the `everruns-platform`
// crate. EVE-845 moved the remaining auth-facing identity values that no core
// code names — `OrgMembership`, the `ANONYMOUS_USER_*` constants, and the
// public-id generation/validation helpers (`generate_org_public_id`,
// `validate_org_public_id`) — to `everruns-platform` as well. What stays here
// does so because core's permissions layer and runtime name it: `OrgRole`
// (portable turn authorization), the `DEFAULT_ORG_*` constants, and the
// internal<->public id conversion helpers.

/// Default organization ID (internal, for DB queries)
pub const DEFAULT_ORG_ID: i64 = 1;

/// Default organization public ID (external, for API)
pub const DEFAULT_ORG_PUBLIC_ID: &str = "org_00000000000000000000000000000001";

/// Organization-level role with hierarchical permissions.
/// Owner > Admin > Member — checked via `has_permission()`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum OrgRole {
    Member,
    Admin,
    #[default]
    Owner,
}

impl OrgRole {
    /// Check if this role has at least the `required` permission level.
    pub fn has_permission(self, required: OrgRole) -> bool {
        self.level() >= required.level()
    }

    /// String representation for DB storage.
    pub fn as_str(self) -> &'static str {
        match self {
            OrgRole::Member => "member",
            OrgRole::Admin => "admin",
            OrgRole::Owner => "owner",
        }
    }

    fn level(self) -> u8 {
        match self {
            OrgRole::Member => 0,
            OrgRole::Admin => 1,
            OrgRole::Owner => 2,
        }
    }
}

impl fmt::Display for OrgRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OrgRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "member" => Ok(OrgRole::Member),
            "admin" => Ok(OrgRole::Admin),
            "owner" => Ok(OrgRole::Owner),
            _ => Err(format!("invalid org role: {s}")),
        }
    }
}

/// Derive a deterministic public_id from an internal org_id.
///
/// For `DEFAULT_ORG_ID` this returns `DEFAULT_ORG_PUBLIC_ID`.
/// For other IDs it produces `org_<032x>` so callers can avoid
/// an async DB lookup when only the public_id format is needed.
pub fn org_public_id_from_internal(org_id: i64) -> String {
    if org_id == DEFAULT_ORG_ID {
        return DEFAULT_ORG_PUBLIC_ID.to_string();
    }
    format!("org_{:032x}", org_id)
}

/// Recover the internal `org_id` from an [`OrgId`] derived via
/// [`org_public_id_from_internal`].
///
/// `org_public_id_from_internal` encodes the internal `i64` as the low bits of
/// the public id's 32-hex payload, which round-trips through the `OrgId` UUID.
/// Used at host boundaries where a public [`OrgId`] must be projected back to
/// the deployment's internal organization key.
pub fn org_internal_id_from_public(org_id: crate::typed_id::OrgId) -> i64 {
    org_id.uuid().as_u128() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_org_internal_id_round_trips() {
        for internal in [DEFAULT_ORG_ID, 5, 42, 1_000_000] {
            let public = org_public_id_from_internal(internal);
            let org_id: crate::typed_id::OrgId = public.parse().expect("parse org id");
            assert_eq!(org_internal_id_from_public(org_id), internal);
        }
    }

    #[test]
    fn test_org_role_hierarchy() {
        assert!(OrgRole::Owner.has_permission(OrgRole::Owner));
        assert!(OrgRole::Owner.has_permission(OrgRole::Admin));
        assert!(OrgRole::Owner.has_permission(OrgRole::Member));

        assert!(!OrgRole::Admin.has_permission(OrgRole::Owner));
        assert!(OrgRole::Admin.has_permission(OrgRole::Admin));
        assert!(OrgRole::Admin.has_permission(OrgRole::Member));

        assert!(!OrgRole::Member.has_permission(OrgRole::Owner));
        assert!(!OrgRole::Member.has_permission(OrgRole::Admin));
        assert!(OrgRole::Member.has_permission(OrgRole::Member));
    }

    #[test]
    fn test_org_role_str_roundtrip() {
        for role in [OrgRole::Member, OrgRole::Admin, OrgRole::Owner] {
            let s = role.as_str();
            let parsed: OrgRole = s.parse().unwrap();
            assert_eq!(parsed, role);
        }
    }

    #[test]
    fn test_org_role_default() {
        assert_eq!(OrgRole::default(), OrgRole::Owner);
    }
}
