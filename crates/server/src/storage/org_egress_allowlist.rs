//! Per-org egress allowlist extension: the platform grant and the org's own
//! host patterns, on `organization_settings`. Read apart from
//! `OrganizationSettingsRow` so the org settings PATCH keeps its shape; the
//! grant and the list have their own endpoints and authority (a platform user
//! grants, org admins edit). See knowledge/operations/system-allowlist.md.
use super::StorageBackend;
use anyhow::Result;

/// The stored extension state of one organization.
#[derive(Debug, Clone, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct OrgEgressAllowlistRow {
    /// Whether a platform user allowed this org to extend the allowlist.
    pub granted: bool,
    /// The org's patterns, kept when the grant is revoked.
    pub patterns: Vec<String>,
}

impl StorageBackend {
    /// The org's extension state; defaults when it has no settings row.
    pub async fn org_egress_allowlist(&self, org_id: i64) -> Result<OrgEgressAllowlistRow> {
        let row = sqlx::query_as::<_, OrgEgressAllowlistRow>(
            "SELECT egress_allowlist_extension_granted AS granted, \
             egress_allowlist_extension AS patterns \
             FROM organization_settings WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(row.unwrap_or_default())
    }

    /// Replace the org's patterns only while its grant is on. Returns `None`
    /// without writing when the org is not granted, so a grant revoked between
    /// the caller's check and this write cannot be raced past.
    pub async fn set_org_egress_allowlist_patterns(
        &self,
        org_id: i64,
        patterns: &[String],
    ) -> Result<Option<OrgEgressAllowlistRow>> {
        let row = sqlx::query_as::<_, OrgEgressAllowlistRow>(
            "UPDATE organization_settings \
             SET egress_allowlist_extension = $2, updated_at = NOW() \
             WHERE org_id = $1 AND egress_allowlist_extension_granted \
             RETURNING egress_allowlist_extension_granted AS granted, \
             egress_allowlist_extension AS patterns",
        )
        .bind(org_id)
        .bind(patterns)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(row)
    }

    /// Grant or revoke the org's right to extend the allowlist. Revoking
    /// keeps the stored patterns.
    pub async fn set_org_egress_allowlist_granted(
        &self,
        org_id: i64,
        granted: bool,
    ) -> Result<OrgEgressAllowlistRow> {
        let row = sqlx::query_as::<_, OrgEgressAllowlistRow>(
            "INSERT INTO organization_settings (org_id, egress_allowlist_extension_granted) \
             VALUES ($1, $2) \
             ON CONFLICT (org_id) DO UPDATE SET \
             egress_allowlist_extension_granted = EXCLUDED.egress_allowlist_extension_granted, \
             updated_at = NOW() \
             RETURNING egress_allowlist_extension_granted AS granted, \
             egress_allowlist_extension AS patterns",
        )
        .bind(org_id)
        .bind(granted)
        .fetch_one(self.database().pool())
        .await?;
        Ok(row)
    }
}
