//! Per-org tool approval policy on `organization_settings.tool_approval_policy`.
//! Read apart from `OrganizationSettingsRow`, like the web search defaults.
//! NULL means no limits. See knowledge/execution/tool-approval.md.
use super::StorageBackend;
use anyhow::Result;
use everruns_contracts::tool_approval_policy::OrgToolApprovalPolicy;

impl StorageBackend {
    /// The org's approval policy; empty (no limits) when it never set one.
    ///
    /// Decision: a stored value that no longer parses fails closed to the
    /// most careful policy (only `protective`), logged, rather than silently
    /// dropping an admin's limits.
    pub async fn org_tool_approval_policy(&self, org_id: i64) -> Result<OrgToolApprovalPolicy> {
        let stored: Option<Option<serde_json::Value>> = sqlx::query_scalar(
            "SELECT tool_approval_policy FROM organization_settings WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(match stored.flatten() {
            Some(value) => serde_json::from_value(value).unwrap_or_else(|error| {
                tracing::warn!(%error, org_id, "Stored tool approval policy does not parse; allowing protective only");
                OrgToolApprovalPolicy {
                    allowed_modes: vec!["protective".to_string()],
                    ..Default::default()
                }
            }),
            None => OrgToolApprovalPolicy::default(),
        })
    }

    /// Replace the org's approval policy (already validated).
    pub async fn set_org_tool_approval_policy(
        &self,
        org_id: i64,
        policy: &OrgToolApprovalPolicy,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO organization_settings (org_id, tool_approval_policy) VALUES ($1, $2) \
             ON CONFLICT (org_id) DO UPDATE SET tool_approval_policy = EXCLUDED.tool_approval_policy, \
             updated_at = NOW()",
        )
        .bind(org_id)
        .bind(serde_json::to_value(policy)?)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }
}
