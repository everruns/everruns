// Organization entity (platform domain aggregate).
//
// Moved out of `everruns-core` in EVE-837. The cross-cutting organization value
// types and multitenancy constants (OrgRole, OrgMembership, DEFAULT_ORG_ID,
// DEFAULT_ORG_PUBLIC_ID, ANONYMOUS_USER_*, and the org public-id helpers) remain
// in `everruns-core` because core's permissions layer, auth-facing membership,
// and runtime fixtures depend on them; they are re-exported from this crate's
// root for a unified consumer surface.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
