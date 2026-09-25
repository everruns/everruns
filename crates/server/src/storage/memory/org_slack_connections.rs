use anyhow::Result;
use chrono::{DateTime, Utc};

use super::InMemoryDatabase;
use crate::storage::{OrgSlackConnectionRow, RotateOrgSlackConnection, UpsertOrgSlackConnection};

impl InMemoryDatabase {
    pub async fn upsert_org_slack_connection(
        &self,
        input: UpsertOrgSlackConnection,
    ) -> Result<OrgSlackConnectionRow> {
        let now = Self::now();
        let mut connections = self.org_slack_connections.write();
        let generation = connections
            .get(&input.org_id)
            .map_or(1, |row| row.token_generation + 1);
        let created_at = connections
            .get(&input.org_id)
            .map_or(now, |row| row.created_at);
        let row = OrgSlackConnectionRow {
            id: connections
                .get(&input.org_id)
                .map_or_else(uuid::Uuid::now_v7, |row| row.id),
            org_id: input.org_id,
            access_token_encrypted: Some(input.access_token_encrypted),
            refresh_token_encrypted: Some(input.refresh_token_encrypted),
            access_token_expires_at: Some(input.access_token_expires_at),
            state: "connected".to_string(),
            token_generation: generation,
            created_at,
            updated_at: now,
        };
        connections.insert(input.org_id, row.clone());
        Ok(row)
    }

    pub async fn get_org_slack_connection(
        &self,
        org_id: i64,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        Ok(self.org_slack_connections.read().get(&org_id).cloned())
    }

    pub async fn claim_org_slack_connection_rotation(
        &self,
        org_id: i64,
        expected_generation: i64,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        let mut connections = self.org_slack_connections.write();
        let Some(row) = connections.get_mut(&org_id) else {
            return Ok(None);
        };
        if row.token_generation != expected_generation || row.state != "connected" {
            return Ok(None);
        }
        row.token_generation += 1;
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn rotate_org_slack_connection(
        &self,
        input: RotateOrgSlackConnection,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        let mut connections = self.org_slack_connections.write();
        let Some(row) = connections.get_mut(&input.org_id) else {
            return Ok(None);
        };
        if row.token_generation != input.expected_generation || row.state != "connected" {
            return Ok(None);
        }
        row.access_token_encrypted = Some(input.access_token_encrypted);
        row.refresh_token_encrypted = Some(input.refresh_token_encrypted);
        row.access_token_expires_at = Some(input.access_token_expires_at);
        row.token_generation += 1;
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn mark_org_slack_reconnect_required(
        &self,
        org_id: i64,
        expected_generation: i64,
    ) -> Result<bool> {
        let mut connections = self.org_slack_connections.write();
        let Some(row) = connections.get_mut(&org_id) else {
            return Ok(false);
        };
        if row.token_generation != expected_generation || row.state != "connected" {
            return Ok(false);
        }
        row.state = "reconnect_required".to_string();
        row.access_token_encrypted = None;
        row.refresh_token_encrypted = None;
        row.access_token_expires_at = None;
        row.token_generation += 1;
        row.updated_at = Self::now();
        Ok(true)
    }

    pub async fn list_due_org_slack_connections(
        &self,
        rotate_before: DateTime<Utc>,
    ) -> Result<Vec<OrgSlackConnectionRow>> {
        let rows = self
            .org_slack_connections
            .read()
            .values()
            .filter(|row| {
                row.state == "connected"
                    && row
                        .access_token_expires_at
                        .is_some_and(|expires_at| expires_at <= rotate_before)
            })
            .cloned()
            .collect();
        Ok(rows)
    }

    pub async fn delete_org_slack_connection(&self, org_id: i64) -> Result<bool> {
        Ok(self.org_slack_connections.write().remove(&org_id).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(org_id: i64) -> UpsertOrgSlackConnection {
        UpsertOrgSlackConnection {
            org_id,
            access_token_encrypted: vec![1],
            refresh_token_encrypted: vec![2],
            access_token_expires_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn connections_are_tenant_isolated() {
        let db = InMemoryDatabase::new();
        db.upsert_org_slack_connection(connection(41))
            .await
            .unwrap();
        db.upsert_org_slack_connection(connection(42))
            .await
            .unwrap();
        db.delete_org_slack_connection(41).await.unwrap();
        assert!(db.get_org_slack_connection(41).await.unwrap().is_none());
        assert!(db.get_org_slack_connection(42).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn rotation_claim_and_replacement_are_generation_guarded() {
        let db = InMemoryDatabase::new();
        let original = db
            .upsert_org_slack_connection(connection(41))
            .await
            .unwrap();
        let claimed = db
            .claim_org_slack_connection_rotation(41, original.token_generation)
            .await
            .unwrap()
            .expect("claim succeeds");
        assert!(
            db.claim_org_slack_connection_rotation(41, original.token_generation)
                .await
                .unwrap()
                .is_none()
        );
        let rotated = db
            .rotate_org_slack_connection(RotateOrgSlackConnection {
                org_id: 41,
                expected_generation: claimed.token_generation,
                access_token_encrypted: vec![3],
                refresh_token_encrypted: vec![4],
                access_token_expires_at: Utc::now(),
            })
            .await
            .unwrap()
            .expect("replacement succeeds");
        assert_eq!(rotated.access_token_encrypted, Some(vec![3]));
    }

    #[tokio::test]
    async fn stale_reconnect_result_cannot_overwrite_newer_tokens() {
        let db = InMemoryDatabase::new();
        let original = db
            .upsert_org_slack_connection(connection(41))
            .await
            .unwrap();
        let claimed = db
            .claim_org_slack_connection_rotation(41, original.token_generation)
            .await
            .unwrap()
            .expect("claim succeeds");
        db.upsert_org_slack_connection(connection(41))
            .await
            .unwrap();
        assert!(
            !db.mark_org_slack_reconnect_required(41, claimed.token_generation)
                .await
                .unwrap()
        );
        assert_eq!(
            db.get_org_slack_connection(41)
                .await
                .unwrap()
                .unwrap()
                .state,
            "connected"
        );
    }
}
