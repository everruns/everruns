// In-memory agent check rule storage. See specs/agent-checks.md.

use anyhow::Result;
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::models::*;

const MAX_AGENT_CHECK_RULE_ROWS_PER_ORG_READ: usize = 256;

impl InMemoryDatabase {
    pub async fn list_agent_check_rules(&self, org_id: i64) -> Result<Vec<AgentCheckRuleRow>> {
        let mut rows: Vec<AgentCheckRuleRow> = self
            .agent_check_rules
            .read()
            .values()
            .filter(|r| r.org_id == org_id)
            .cloned()
            .collect();
        rows.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
        rows.truncate(MAX_AGENT_CHECK_RULE_ROWS_PER_ORG_READ);
        Ok(rows)
    }

    pub async fn count_custom_agent_check_rules_excluding(
        &self,
        org_id: i64,
        excluded_rule_id: &str,
    ) -> Result<i64> {
        let count = self
            .agent_check_rules
            .read()
            .values()
            .filter(|r| {
                r.org_id == org_id && r.kind != "builtin_override" && r.rule_id != excluded_rule_id
            })
            .count();
        Ok(i64::try_from(count).unwrap_or(i64::MAX))
    }

    pub async fn upsert_agent_check_rule(
        &self,
        org_id: i64,
        input: UpsertAgentCheckRuleRow,
    ) -> Result<AgentCheckRuleRow> {
        let now = Self::now();
        let mut guard = self.agent_check_rules.write();
        if let Some(existing) = guard
            .values_mut()
            .find(|r| r.org_id == org_id && r.rule_id == input.rule_id)
        {
            existing.kind = input.kind;
            existing.enabled = input.enabled;
            existing.severity_override = input.severity_override;
            existing.config = input.config;
            existing.updated_at = now;
            return Ok(existing.clone());
        }
        let row = AgentCheckRuleRow {
            id: Uuid::now_v7(),
            org_id,
            rule_id: input.rule_id,
            kind: input.kind,
            enabled: input.enabled,
            severity_override: input.severity_override,
            config: input.config,
            created_at: now,
            updated_at: now,
        };
        guard.insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn delete_agent_check_rule(&self, org_id: i64, rule_id: &str) -> Result<bool> {
        let mut guard = self.agent_check_rules.write();
        let key = guard
            .iter()
            .find(|(_, r)| r.org_id == org_id && r.rule_id == rule_id)
            .map(|(k, _)| *k);
        if let Some(key) = key {
            guard.remove(&key);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
