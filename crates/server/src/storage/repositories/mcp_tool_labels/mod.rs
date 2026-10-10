// PostgreSQL repository: saved per-tool risk labels on organization MCP servers.
//
// Spec: knowledge/integrations/mcp-servers.md ("Tool risk labels").

pub(super) mod rows;
use rows::*;

use super::Database;
use anyhow::Result;
use everruns_server_macros::sql;
use uuid::Uuid;

impl Database {
    /// Labels and suggestions for the given servers, in one query.
    pub async fn list_mcp_tool_labels(
        &self,
        org_id: i64,
        server_ids: &[Uuid],
    ) -> Result<Vec<McpToolLabelRow>> {
        if server_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<_, McpToolLabelRow>(sql!(
            r#"
            SELECT {McpToolLabelRow} FROM mcp_tool_labels
            WHERE org_id = $1 AND mcp_server_id = ANY($2)
            ORDER BY mcp_server_id, tool_name
            "#
        ))
        .bind(org_id)
        .bind(server_ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Person-set labels for the named organization servers, in one query.
    /// Returns `(server name, tool name, label)`; rows without a label are
    /// left out.
    pub async fn list_mcp_tool_labels_by_server_names(
        &self,
        org_id: i64,
        server_names: &[String],
    ) -> Result<Vec<(String, String, String)>> {
        if server_names.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<_, (String, String, String)>(
            r#"
            SELECT s.name, l.tool_name, l.label
            FROM mcp_tool_labels l
            JOIN mcp_servers s ON s.id = l.mcp_server_id AND s.org_id = l.org_id
            WHERE l.org_id = $1
              AND s.name = ANY($2)
              AND s.owner_virtual_user_id IS NULL
              AND s.status <> 'deleted'
              AND l.label IS NOT NULL
            "#,
        )
        .bind(org_id)
        .bind(server_names)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Set or clear (`None`) a person's label for one tool. Setting a label
    /// settles the question, so it drops any suggestion; clearing keeps the
    /// row and the suggestion stored next to it.
    pub async fn set_mcp_tool_label(
        &self,
        org_id: i64,
        server_id: Uuid,
        tool_name: &str,
        label: Option<&str>,
        set_by: Option<Uuid>,
    ) -> Result<McpToolLabelRow> {
        let row = sqlx::query_as::<_, McpToolLabelRow>(sql!(
            r#"
            INSERT INTO mcp_tool_labels (org_id, mcp_server_id, tool_name, label, set_by)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (mcp_server_id, tool_name) DO UPDATE SET
                label = EXCLUDED.label,
                suggested_label = CASE WHEN EXCLUDED.label IS NULL
                    THEN mcp_tool_labels.suggested_label END,
                set_by = EXCLUDED.set_by
            RETURNING {McpToolLabelRow}
            "#
        ))
        .bind(org_id)
        .bind(server_id)
        .bind(tool_name)
        .bind(label)
        .bind(set_by)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Store an automated suggestion for one tool. A tool a person already
    /// labeled is left alone (the label settled it), so nothing is written and
    /// `None` comes back.
    pub async fn set_mcp_tool_suggestion(
        &self,
        org_id: i64,
        server_id: Uuid,
        tool_name: &str,
        suggested_label: &str,
    ) -> Result<Option<McpToolLabelRow>> {
        let row = sqlx::query_as::<_, McpToolLabelRow>(sql!(
            r#"
            INSERT INTO mcp_tool_labels (org_id, mcp_server_id, tool_name, suggested_label)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (mcp_server_id, tool_name) DO UPDATE SET
                suggested_label = EXCLUDED.suggested_label
            WHERE mcp_tool_labels.label IS NULL
            RETURNING {McpToolLabelRow}
            "#
        ))
        .bind(org_id)
        .bind(server_id)
        .bind(tool_name)
        .bind(suggested_label)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Tool lists found through per-agent or per-person discovery of one
    /// server (the only tool lists an OAuth server has).
    pub async fn list_mcp_discovered_tool_sets(
        &self,
        org_id: i64,
        server_id: Uuid,
    ) -> Result<Vec<serde_json::Value>> {
        let rows = sqlx::query_scalar::<_, serde_json::Value>(
            r#"
            SELECT cached_tools FROM mcp_service_tool_caches
            WHERE org_id = $1 AND mcp_server_id = $2
            ORDER BY tools_cached_at DESC
            LIMIT 50
            "#,
        )
        .bind(org_id)
        .bind(server_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
