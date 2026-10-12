// Organization, membership, invitation, settings and task webhook rows.

use crate::kernel_imports::contracts::driver_registry::ServiceKind;
use crate::kernel_imports::contracts::typed_id::{HarnessId, ModelId, ProviderId};
use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Organization row from database
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct OrganizationRow {
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// External identity provider ID (e.g., PropelAuth org ID). NULL for OSS.
    #[sqlx(default)]
    pub external_id: Option<String>,
    /// User who created this organization. NULL for seeded/external orgs.
    #[sqlx(default)]
    pub created_by: Option<Uuid>,
    /// When the org's creator finished or skipped the setup wizard. NULL means
    /// onboarding is still incomplete and the resume redirect sends the current
    /// org's members back to /setup. Seeded/default and externally-synced orgs
    /// are created already-complete. See migration 090.
    #[sqlx(default)]
    pub onboarding_completed_at: Option<DateTime<Utc>>,
}

/// Organization member row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct OrganizationMemberRow {
    pub org_id: i64,
    pub user_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

/// Result of adding an organization member under the member-capacity guard.
#[derive(Debug, Clone)]
pub enum AddOrganizationMemberOutcome {
    Added(OrganizationMemberRow),
    AlreadyMember(OrganizationMemberRow),
    MemberLimitReached,
}

/// Organization member with user info (for API responses)
#[derive(Debug, Clone, FromRow)]
pub struct OrganizationMemberWithUserRow {
    pub user_id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

/// Organization with role (for user's org list)
#[derive(Debug, Clone, FromRow)]
pub struct OrganizationWithRoleRow {
    pub org_id: i64,
    pub public_id: String,
    pub name: String,
    pub role: String,
}

/// Input for creating an organization
#[derive(Debug, Clone)]
pub struct CreateOrganizationRow {
    pub public_id: String,
    pub name: String,
    pub created_by: Option<Uuid>,
}

/// Input for updating an organization
#[derive(Debug, Clone, Default)]
pub struct UpdateOrganization {
    pub name: Option<String>,
}

/// Org-level "default provider per service" map (EVE-569): tier-2 service
/// resolution defaults keyed by [`ServiceKind`]. Empty when no defaults are
/// configured. Persisted as a JSONB object (snake_case service kind -> provider
/// public id).
pub type ServiceProviderDefaults = std::collections::HashMap<ServiceKind, ProviderId>;

/// Organization settings row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct OrganizationSettingsRow {
    pub org_id: i64,
    pub default_model_id: Option<ModelId>,
    pub default_harness_id: Option<HarnessId>,
    pub base_harness_id: Option<HarnessId>,
    /// Org-level default provider per service (EVE-569); empty means none.
    pub default_provider_per_service: sqlx::types::Json<ServiceProviderDefaults>,
    pub system_decisions: String, // `SystemDecisions` as stored
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateOrganizationSettings {
    pub default_model_id: UpdateField<ModelId>,
    pub default_harness_id: UpdateField<HarnessId>,
    pub base_harness_id: UpdateField<HarnessId>,
    /// Replaces the whole per-service map: `Set` overwrites, `Clear` empties.
    pub default_provider_per_service: UpdateField<ServiceProviderDefaults>,
    pub system_decisions: Option<crate::storage::SystemDecisions>, // `None` keeps it
}

/// Organization task webhook row from database
#[derive(Debug, Clone, sqlx::FromRow, everruns_server_macros::Columns)]
pub struct OrgTaskWebhookRow {
    pub id: i64,
    pub public_id: String,
    pub org_id: i64,
    pub url: String,
    pub secret: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a task webhook
#[derive(Debug, Clone)]
pub struct CreateOrgTaskWebhook {
    pub public_id: String,
    pub org_id: i64,
    pub url: String,
    pub secret: Option<String>,
    pub enabled: bool,
}

/// Input for updating a task webhook (all fields optional)
#[derive(Debug, Clone, Default)]
pub struct UpdateOrgTaskWebhook {
    pub url: Option<String>,
    pub secret: Option<Option<String>>,
    pub enabled: Option<bool>,
}

/// Input for creating an organization member
#[derive(Debug, Clone)]
pub struct CreateOrganizationMemberRow {
    pub org_id: i64,
    pub user_id: Uuid,
}

/// Organization invitation row from database (EVE-602).
///
/// The raw invite token is never stored; only `token_hash` (SHA-256) is kept.
/// Status is derived from the timestamp columns (`accepted_at`, `revoked_at`,
/// `expires_at`) rather than a separate enum so the row is the single source of
/// truth.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct OrgInvitationRow {
    pub id: i64,
    pub public_id: String,
    pub org_id: i64,
    pub email: String,
    pub role: String,
    pub invited_by: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub accepted_by: Option<Uuid>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Actionable invitation with organization display data.
#[derive(Debug, Clone, FromRow)]
pub struct OutstandingOrgInvitationRow {
    pub public_id: String,
    pub org_id: i64,
    pub org_name: String,
    pub email: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Result of atomically claiming an invitation and ensuring its membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptOrgInvitationOutcome {
    Accepted { org_id: i64, role: String },
    MemberLimitReached,
    NotActionable,
}

/// Input for creating an organization invitation.
#[derive(Debug, Clone)]
pub struct CreateOrgInvitation {
    pub public_id: String,
    pub org_id: i64,
    pub email: String,
    pub role: String,
    pub invited_by: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}
