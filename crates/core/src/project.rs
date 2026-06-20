// Project types for the project layer (nested inside an organization).
// See specs/multitenancy.md (Projects) and the "Project structure" design (F).
//
// A Project is the day-to-day work scope users switch between; an Organization
// is the billing/team wrapper. Project-scoped resources belong to exactly one
// project. Every org has exactly one `default` project.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Default project ID (internal, for DB queries). Seeded for the default org.
pub const DEFAULT_PROJECT_ID: i64 = 1;

/// Default project public ID (external, for API). Mirrors DEFAULT_ORG_PUBLIC_ID.
pub const DEFAULT_PROJECT_PUBLIC_ID: &str = "proj_00000000000000000000000000000001";

/// Project entity (domain type).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Project {
    /// External identifier (proj_<32-hex-chars>).
    pub public_id: String,
    /// Owning organization public_id.
    pub org_public_id: String,
    /// Display name (unique per org, case-insensitive).
    pub name: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this is the org's default project.
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Project membership info (for user context). Projects inherit org membership;
/// `role` is carried for symmetry with `OrgMembership` and future per-project
/// roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectSummary {
    /// Internal project_id for DB queries (not serialized to API).
    #[serde(skip_serializing)]
    pub project_id: i64,
    /// External identifier.
    pub public_id: String,
    /// Owning org public_id.
    pub org_public_id: String,
    /// Display name.
    pub name: String,
    /// Whether this is the org's default project.
    pub is_default: bool,
}

/// Generate a new project public ID.
/// Format: proj_<32-hex-chars> (UUIDv4 lowercase hex, no dashes).
pub fn generate_project_public_id() -> String {
    let uuid = Uuid::new_v4();
    format!("proj_{}", uuid.simple())
}

/// Validate project public ID format.
/// Pattern: ^proj_[0-9a-f]{32}$
pub fn validate_project_public_id(public_id: &str) -> bool {
    let Some(suffix) = public_id.strip_prefix("proj_") else {
        return false;
    };
    suffix.len() == 32
        && suffix
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_project_public_id() {
        let id = generate_project_public_id();
        assert!(id.starts_with("proj_"));
        assert_eq!(id.len(), 37); // "proj_" + 32 hex chars
        assert!(validate_project_public_id(&id));
    }

    #[test]
    fn test_validate_project_public_id() {
        assert!(validate_project_public_id(DEFAULT_PROJECT_PUBLIC_ID));
        assert!(validate_project_public_id(
            "proj_2f3c1b3e6a9d4c6f8a1d4e9c9b7f21a0"
        ));
        assert!(!validate_project_public_id("project_123"));
        assert!(!validate_project_public_id("proj_123"));
        assert!(!validate_project_public_id(
            "proj_2F3C1B3E6A9D4C6F8A1D4E9C9B7F21A0"
        ));
        assert!(!validate_project_public_id(
            "org_00000000000000000000000000000001"
        ));
    }
}
