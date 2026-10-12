//! Per-org web search defaults on `organization_settings.web_search_defaults`.
//! Read apart from `OrganizationSettingsRow`, like the egress grants, so the
//! settings patch keeps its shape. NULL means the built-in defaults. See
//! knowledge/integrations/web-search.md.
use super::StorageBackend;
use anyhow::Result;
use everruns_contracts::runtime::web_search::WebSearchDefaults;

impl StorageBackend {
    /// The org's web search defaults; the built-in defaults when it never set
    /// any. A stored value that no longer parses (a field renamed since) also
    /// reads as the defaults, logged, rather than breaking every search.
    pub async fn org_web_search_defaults(&self, org_id: i64) -> Result<WebSearchDefaults> {
        let stored: Option<Option<serde_json::Value>> = sqlx::query_scalar(
            "SELECT web_search_defaults FROM organization_settings WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(match stored.flatten() {
            Some(value) => serde_json::from_value(value).unwrap_or_else(|error| {
                tracing::warn!(%error, org_id, "Stored web search defaults do not parse; using built-in defaults");
                WebSearchDefaults::default()
            }),
            None => WebSearchDefaults::default(),
        })
    }

    /// Replace the org's web search defaults (already validated).
    pub async fn set_org_web_search_defaults(
        &self,
        org_id: i64,
        defaults: &WebSearchDefaults,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO organization_settings (org_id, web_search_defaults) VALUES ($1, $2) \
             ON CONFLICT (org_id) DO UPDATE SET web_search_defaults = EXCLUDED.web_search_defaults, \
             updated_at = NOW()",
        )
        .bind(org_id)
        .bind(serde_json::to_value(defaults)?)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }
}
