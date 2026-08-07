// Org-configurable agent check rule storage (PostgreSQL). See knowledge/evaluation/agent-checks.md.

use anyhow::Result;

use crate::storage::Database;
use crate::storage::models::*;

const MAX_AGENT_CHECK_RULE_ROWS_PER_ORG_READ: i64 = 256;

impl Database {
    pub async fn list_agent_check_rules(&self, org_id: i64) -> Result<Vec<AgentCheckRuleRow>> {
        let rows = sqlx::query_as::<_, AgentCheckRuleRow>(
            r#"
            SELECT id, org_id, rule_id, kind, enabled, severity_override, config,
                   created_at, updated_at
            FROM agent_check_rules
            WHERE org_id = $1
            ORDER BY rule_id
            LIMIT $2
            "#,
        )
        .bind(org_id)
        .bind(MAX_AGENT_CHECK_RULE_ROWS_PER_ORG_READ)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn count_custom_agent_check_rules_excluding(
        &self,
        org_id: i64,
        excluded_rule_id: &str,
    ) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM agent_check_rules
            WHERE org_id = $1
              AND kind != 'builtin_override'
              AND rule_id != $2
            "#,
        )
        .bind(org_id)
        .bind(excluded_rule_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    pub async fn upsert_agent_check_rule(
        &self,
        org_id: i64,
        input: UpsertAgentCheckRuleRow,
    ) -> Result<AgentCheckRuleRow> {
        let row = sqlx::query_as::<_, AgentCheckRuleRow>(
            r#"
            INSERT INTO agent_check_rules (org_id, rule_id, kind, enabled, severity_override, config)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (org_id, rule_id) DO UPDATE SET
                kind = EXCLUDED.kind,
                enabled = EXCLUDED.enabled,
                severity_override = EXCLUDED.severity_override,
                config = EXCLUDED.config,
                updated_at = NOW()
            RETURNING id, org_id, rule_id, kind, enabled, severity_override, config,
                      created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.rule_id)
        .bind(&input.kind)
        .bind(input.enabled)
        .bind(&input.severity_override)
        .bind(&input.config)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Delete a rule by id. Returns true when a row was removed.
    pub async fn delete_agent_check_rule(&self, org_id: i64, rule_id: &str) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM agent_check_rules WHERE org_id = $1 AND rule_id = $2")
                .bind(org_id)
                .bind(rule_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }
}
