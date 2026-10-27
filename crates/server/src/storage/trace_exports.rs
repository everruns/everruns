//! Tenant-scoped customer tracing storage. Never return credentials to public responses.
use super::StorageBackend;
use crate::domains::trace_exports::{DestinationConfig, RoutingPolicy};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

// A bounded list and two correlated aggregate subqueries avoid per-row API lookups.
const DESTINATION_LIST: &str = "SELECT d.*, EXISTS(SELECT 1 FROM tracing_policies p WHERE p.org_id=d.org_id AND p.agent_id IS NULL AND p.policy->'destination_ids' @> jsonb_build_array(d.id::text)) AS organization_default, (SELECT count(*) FROM tracing_policies p WHERE p.org_id=d.org_id AND p.agent_id IS NOT NULL AND p.policy->'destination_ids' @> jsonb_build_array(d.id::text)) AS agent_selections FROM tracing_destinations d WHERE d.org_id=$1 ORDER BY d.id LIMIT 32";
const DESTINATION_ONE: &str = "SELECT d.*, EXISTS(SELECT 1 FROM tracing_policies p WHERE p.org_id=d.org_id AND p.agent_id IS NULL AND p.policy->'destination_ids' @> jsonb_build_array(d.id::text)) AS organization_default, (SELECT count(*) FROM tracing_policies p WHERE p.org_id=d.org_id AND p.agent_id IS NOT NULL AND p.policy->'destination_ids' @> jsonb_build_array(d.id::text)) AS agent_selections FROM tracing_destinations d WHERE d.org_id=$1 AND d.id=$2";

#[derive(Clone, FromRow)]
pub struct TracingDestinationRow {
    pub id: Uuid,
    pub org_id: i64,
    pub connection_id: Uuid,
    pub config: serde_json::Value,
    pub revision: i64,
    pub last_accepted_at: Option<DateTime<Utc>>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub dropped_events: i64,
    pub organization_default: bool,
    pub agent_selections: i64,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct TracingPolicyView {
    pub policy: RoutingPolicy,
    pub revision: i64,
}
impl StorageBackend {
    pub async fn tracing_destinations(&self, org: i64) -> Result<Vec<TracingDestinationRow>> {
        Ok(sqlx::query_as(DESTINATION_LIST)
            .bind(org)
            .fetch_all(self.database().tx_pool())
            .await?)
    }
    pub async fn tracing_destination(
        &self,
        org: i64,
        id: Uuid,
    ) -> Result<Option<TracingDestinationRow>> {
        Ok(sqlx::query_as(DESTINATION_ONE)
            .bind(org)
            .bind(id)
            .fetch_optional(self.database().tx_pool())
            .await?)
    }
    /// Routing and send-time checks do not need potentially expensive policy usage counts.
    pub async fn tracing_export_destinations(
        &self,
        org: i64,
    ) -> Result<Vec<TracingDestinationRow>> {
        Ok(sqlx::query_as("SELECT d.*,FALSE AS organization_default,0::bigint AS agent_selections FROM tracing_destinations d WHERE org_id=$1 LIMIT 32")
            .bind(org).fetch_all(self.database().tx_pool()).await?)
    }
    pub async fn tracing_export_destination(
        &self,
        org: i64,
        id: Uuid,
    ) -> Result<Option<TracingDestinationRow>> {
        Ok(sqlx::query_as("SELECT d.*,FALSE AS organization_default,0::bigint AS agent_selections FROM tracing_destinations d WHERE org_id=$1 AND id=$2")
            .bind(org).bind(id).fetch_optional(self.database().tx_pool()).await?)
    }
    pub async fn tracing_secrets(&self, org: i64, id: Uuid) -> Result<Option<Vec<u8>>> {
        Ok(sqlx::query_scalar(
            "SELECT secrets_encrypted FROM tracing_connections WHERE org_id=$1 AND id=$2",
        )
        .bind(org)
        .bind(id)
        .fetch_optional(self.database().tx_pool())
        .await?)
    }
    pub async fn create_tracing_destination(
        &self,
        org: i64,
        config: &DestinationConfig,
        secrets: Vec<u8>,
    ) -> Result<Uuid> {
        let mut tx = self.database().tx_pool().begin().await?;
        // Serialize the quota check with every other configuration mutation for this org.
        sqlx::query("SELECT org_id FROM organizations WHERE org_id=$1 FOR UPDATE")
            .bind(org)
            .execute(&mut *tx)
            .await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM tracing_destinations WHERE org_id=$1")
                .bind(org)
                .fetch_one(&mut *tx)
                .await?;
        anyhow::ensure!(count < 32, "Destination limit reached");
        let connection:Uuid=sqlx::query_scalar("INSERT INTO tracing_connections (org_id,secrets_encrypted) VALUES ($1,$2) RETURNING id").bind(org).bind(secrets).fetch_one(&mut *tx).await?;
        let id=sqlx::query_scalar("INSERT INTO tracing_destinations (org_id,connection_id,config) VALUES ($1,$2,$3) RETURNING id").bind(org).bind(connection).bind(serde_json::to_value(config)?).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(id)
    }
    pub async fn update_tracing_destination(
        &self,
        org: i64,
        id: Uuid,
        revision: i64,
        config: &DestinationConfig,
        secrets: Option<Vec<u8>>,
    ) -> Result<bool> {
        let mut tx = self.database().tx_pool().begin().await?;
        sqlx::query("SELECT org_id FROM organizations WHERE org_id=$1 FOR UPDATE")
            .bind(org)
            .execute(&mut *tx)
            .await?;
        let connection:Option<Uuid>=sqlx::query_scalar("UPDATE tracing_destinations SET config=$4,revision=revision+1,last_error=NULL,last_accepted_at=NULL,last_attempt_at=NULL WHERE org_id=$1 AND id=$2 AND revision=$3 RETURNING connection_id").bind(org).bind(id).bind(revision).bind(serde_json::to_value(config)?).fetch_optional(&mut *tx).await?;
        if let (Some(connection), Some(secrets)) = (connection, secrets) {
            sqlx::query(
                "UPDATE tracing_connections SET secrets_encrypted=$3 WHERE org_id=$1 AND id=$2",
            )
            .bind(org)
            .bind(connection)
            .bind(secrets)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(connection.is_some())
    }
    pub async fn delete_tracing_destination(
        &self,
        org: i64,
        id: Uuid,
        revision: i64,
    ) -> Result<bool> {
        let mut tx = self.database().tx_pool().begin().await?;
        sqlx::query("SELECT org_id FROM organizations WHERE org_id=$1 FOR UPDATE")
            .bind(org)
            .execute(&mut *tx)
            .await?;
        let used:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tracing_policies WHERE org_id=$1 AND policy->'destination_ids' @> $2::jsonb)").bind(org).bind(serde_json::json!([id])).fetch_one(&mut *tx).await?;
        anyhow::ensure!(
            !used,
            "Remove this destination from routing policies before deleting it"
        );
        let connection:Option<Uuid>=sqlx::query_scalar("DELETE FROM tracing_destinations WHERE org_id=$1 AND id=$2 AND revision=$3 RETURNING connection_id").bind(org).bind(id).bind(revision).fetch_optional(&mut *tx).await?;
        if let Some(c) = connection {
            sqlx::query("DELETE FROM tracing_connections WHERE org_id=$1 AND id=$2")
                .bind(org)
                .bind(c)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(connection.is_some())
    }
    pub async fn tracing_policy(&self, org: i64, agent: Option<Uuid>) -> Result<TracingPolicyView> {
        let row:Option<(serde_json::Value,i64)>=sqlx::query_as("SELECT policy,revision FROM tracing_policies WHERE org_id=$1 AND agent_id IS NOT DISTINCT FROM $2").bind(org).bind(agent).fetch_optional(self.database().tx_pool()).await?;
        match row {
            Some((p, r)) => Ok(TracingPolicyView {
                policy: serde_json::from_value(p)?,
                revision: r,
            }),
            None => Ok(TracingPolicyView {
                policy: RoutingPolicy::default(),
                revision: 0,
            }),
        }
    }
    pub async fn save_tracing_policy(
        &self,
        org: i64,
        agent: Option<Uuid>,
        revision: i64,
        policy: &RoutingPolicy,
    ) -> Result<bool> {
        let mut tx = self.database().tx_pool().begin().await?;
        sqlx::query("SELECT org_id FROM organizations WHERE org_id=$1 FOR UPDATE")
            .bind(org)
            .execute(&mut *tx)
            .await?;
        if let Some(agent) = agent {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agents WHERE org_id=$1 AND id=$2)")
                    .bind(org)
                    .bind(agent)
                    .fetch_one(&mut *tx)
                    .await?;
            anyhow::ensure!(exists, "Agent outside organization");
        }
        let current:Option<(serde_json::Value,i64)>=sqlx::query_as("SELECT policy,revision FROM tracing_policies WHERE org_id=$1 AND agent_id IS NOT DISTINCT FROM $2 FOR UPDATE").bind(org).bind(agent).fetch_optional(&mut *tx).await?;
        if current.as_ref().map_or(0, |(_, r)| *r) != revision {
            return Ok(false);
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tracing_destinations WHERE org_id=$1 AND id=ANY($2)",
        )
        .bind(org)
        .bind(&policy.destination_ids)
        .fetch_one(&mut *tx)
        .await?;
        anyhow::ensure!(
            count as usize == policy.destination_ids.len(),
            "Unknown destination selection"
        );
        sqlx::query("INSERT INTO tracing_policies (org_id,agent_id,policy) VALUES ($1,$2,$3) ON CONFLICT (org_id,agent_id) DO UPDATE SET policy=EXCLUDED.policy,revision=tracing_policies.revision+1,updated_at=NOW()")
            .bind(org).bind(agent).bind(serde_json::to_value(policy)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
    pub async fn record_tracing_delivery(
        &self,
        org: i64,
        id: Uuid,
        revision: i64,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE tracing_destinations SET last_attempt_at=NOW(),last_accepted_at=CASE WHEN $4::text IS NULL THEN NOW() ELSE last_accepted_at END,last_error=$4 WHERE org_id=$1 AND id=$2 AND revision=$3")
            .bind(org).bind(id).bind(revision).bind(error).execute(self.database().tx_pool()).await?;
        Ok(())
    }
}
