// Organization types for multitenancy
// See specs/multitenancy.md

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Default organization ID (internal, for DB queries)
pub const DEFAULT_ORG_ID: i64 = 1;

/// Default organization public ID (external, for API)
pub const DEFAULT_ORG_PUBLIC_ID: &str = "org_00000000000000000000000000000001";

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
}
