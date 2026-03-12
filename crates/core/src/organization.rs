// Organization types for multitenancy
// See specs/multitenancy.md
//
// Decision: Hierarchical org roles (Owner > Admin > Member) using PartialOrd.
// External auth providers map their roles to OrgRole via AuthBackend trait.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Default organization ID (internal, for DB queries)
pub const DEFAULT_ORG_ID: i64 = 1;

/// Default organization public ID (external, for API)
pub const DEFAULT_ORG_PUBLIC_ID: &str = "org_00000000000000000000000000000001";

/// Well-known anonymous user UUID for auth=none mode.
/// This is a real database user seeded at startup, so all code paths
/// (org membership, API keys, etc.) work without special-casing.
pub const ANONYMOUS_USER_ID: Uuid = Uuid::from_bytes([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
]);

/// Anonymous user email
pub const ANONYMOUS_USER_EMAIL: &str = "anonymous@local";

/// Anonymous user display name
pub const ANONYMOUS_USER_NAME: &str = "Anonymous";

/// Organization entity (domain type)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Organization {
    /// External identifier (org_<32-hex-chars>)
    pub public_id: String,
    /// Display name
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

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

/// Organization membership info (for user context)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OrgMembership {
    /// Internal org_id for DB queries (not serialized to API)
    #[serde(skip_serializing)]
    pub org_id: i64,
    /// External identifier
    pub public_id: String,
    /// Display name
    pub name: String,
    /// User's role in this organization
    #[serde(default)]
    pub role: OrgRole,
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

/// Generate a new organization public ID
/// Format: org_<32-hex-chars> (UUIDv4 lowercase hex, no dashes)
pub fn generate_org_public_id() -> String {
    let uuid = Uuid::new_v4();
    format!("org_{}", uuid.simple())
}

/// Validate organization public ID format
/// Pattern: ^org_[0-9a-f]{32}$
pub fn validate_org_public_id(public_id: &str) -> bool {
    if !public_id.starts_with("org_") {
        return false;
    }
    let suffix = &public_id[4..];
    suffix.len() == 32
        && suffix
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_org_public_id() {
        let id = generate_org_public_id();
        assert!(id.starts_with("org_"));
        assert_eq!(id.len(), 36); // "org_" + 32 hex chars
        assert!(validate_org_public_id(&id));
    }

    #[test]
    fn test_validate_org_public_id() {
        // Valid
        assert!(validate_org_public_id(
            "org_00000000000000000000000000000001"
        ));
        assert!(validate_org_public_id(
            "org_2f3c1b3e6a9d4c6f8a1d4e9c9b7f21a0"
        ));

        // Invalid - wrong prefix
        assert!(!validate_org_public_id(
            "organization_12345678901234567890123456789012"
        ));

        // Invalid - too short
        assert!(!validate_org_public_id("org_123"));

        // Invalid - too long
        assert!(!validate_org_public_id(
            "org_123456789012345678901234567890123"
        ));

        // Invalid - uppercase
        assert!(!validate_org_public_id(
            "org_2F3C1B3E6A9D4C6F8A1D4E9C9B7F21A0"
        ));

        // Invalid - non-hex characters
        assert!(!validate_org_public_id(
            "org_ghijklmnopqrstuvwxyz1234567890"
        ));
    }

    #[test]
    fn test_default_org_public_id_valid() {
        assert!(validate_org_public_id(DEFAULT_ORG_PUBLIC_ID));
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
