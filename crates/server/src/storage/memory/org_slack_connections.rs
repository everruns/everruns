use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::{OrgSlackConnectionRow, RotateOrgSlackConnection, UpsertOrgSlackConnection};

impl InMemoryDatabase {
    pub async fn upsert_org_slack_connection(
        &self,
        input: UpsertOrgSlackConnection,
    ) -> Result<OrgSlackConnectionRow> {
        let now = Self::now();
        let mut connections = self.org_slack_connections.write();
        let existing = connections
            .values()
            .find(|row| {
                row.org_id == input.org_id && row.team_id.as_deref() == Some(&input.team_id)
            })
            .cloned();
        let row = OrgSlackConnectionRow {
            id: existing.as_ref().map_or_else(Uuid::now_v7, |row| row.id),
            org_id: input.org_id,
            team_id: Some(input.team_id),
            team_name: input
                .team_name
                .or_else(|| existing.as_ref().and_then(|row| row.team_name.clone())),
            access_token_encrypted: Some(input.access_token_encrypted),
            refresh_token_encrypted: Some(input.refresh_token_encrypted),
            access_token_expires_at: Some(input.access_token_expires_at),
            state: "connected".to_string(),
            token_generation: existing.as_ref().map_or(1, |row| row.token_generation + 1),
            created_at: existing.as_ref().map_or(now, |row| row.created_at),
            updated_at: now,
        };
        connections.insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_org_slack_connection(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        Ok(self
            .org_slack_connections
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    pub async fn list_org_slack_connections(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrgSlackConnectionRow>> {
        let mut rows: Vec<_> = self
            .org_slack_connections
            .read()
            .values()
            .filter(|row| row.org_id == org_id)
            .cloned()
            .collect();
        rows.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(rows)
    }

    pub async fn claim_org_slack_connection_rotation(
        &self,
        id: Uuid,
        expected_generation: i64,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        let mut connections = self.org_slack_connections.write();
        let Some(row) = connections.get_mut(&id) else {
            return Ok(None);
        };
        if row.token_generation != expected_generation || row.state != "connected" {
            return Ok(None);
        }
        row.state = "rotating".to_string();
        row.token_generation += 1;
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn rotate_org_slack_connection(
        &self,
        input: RotateOrgSlackConnection,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        let mut connections = self.org_slack_connections.write();
        let Some(row) = connections.get_mut(&input.id) else {
            return Ok(None);
        };
        if row.token_generation != input.expected_generation || row.state != "rotating" {
            return Ok(None);
        }
        row.access_token_encrypted = Some(input.access_token_encrypted);
        row.refresh_token_encrypted = Some(input.refresh_token_encrypted);
        row.access_token_expires_at = Some(input.access_token_expires_at);
        if row.team_id.is_none() {
            row.team_id = input.team_id;
        }
        row.state = "connected".to_string();
        row.token_generation += 1;
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn mark_org_slack_reconnect_required(
        &self,
        id: Uuid,
        expected_generation: i64,
    ) -> Result<bool> {
        let mut connections = self.org_slack_connections.write();
        let Some(row) = connections.get_mut(&id) else {
            return Ok(false);
        };
        if row.token_generation != expected_generation || row.state != "rotating" {
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
                    && (row.team_id.is_none()
                        || row
                            .access_token_expires_at
                            .is_some_and(|expires_at| expires_at <= rotate_before))
            })
            .cloned()
            .collect();
        Ok(rows)
    }

    pub async fn delete_org_slack_connection(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut connections = self.org_slack_connections.write();
        if connections.get(&id).is_some_and(|row| row.org_id == org_id) {
            connections.remove(&id);
            return Ok(true);
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(org_id: i64, team_id: &str) -> UpsertOrgSlackConnection {
        UpsertOrgSlackConnection {
            org_id,
            team_id: team_id.to_string(),
            team_name: None,
            access_token_encrypted: vec![1],
            refresh_token_encrypted: vec![2],
            access_token_expires_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn connections_are_tenant_isolated() {
        let db = InMemoryDatabase::new();
        let theirs = db
            .upsert_org_slack_connection(connection(41, "T1"))
            .await
            .unwrap();
        db.upsert_org_slack_connection(connection(42, "T1"))
            .await
            .unwrap();
        // Another org cannot read or delete it by id.
        assert!(
            db.get_org_slack_connection(42, theirs.id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !db.delete_org_slack_connection(42, theirs.id)
                .await
                .unwrap()
        );
        assert_eq!(db.list_org_slack_connections(41).await.unwrap().len(), 1);
        assert_eq!(db.list_org_slack_connections(42).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_org_connects_several_workspaces_and_reconnecting_one_replaces_it() {
        let db = InMemoryDatabase::new();
        let first = db
            .upsert_org_slack_connection(connection(41, "T1"))
            .await
            .unwrap();
        db.upsert_org_slack_connection(connection(41, "T2"))
            .await
            .unwrap();
        let again = db
            .upsert_org_slack_connection(connection(41, "T1"))
            .await
            .unwrap();
        assert_eq!(again.id, first.id);
        assert!(again.token_generation > first.token_generation);
        assert_eq!(db.list_org_slack_connections(41).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn rotation_claim_and_replacement_are_generation_guarded() {
        let db = InMemoryDatabase::new();
        let original = db
            .upsert_org_slack_connection(connection(41, "T1"))
            .await
            .unwrap();
        let claimed = db
            .claim_org_slack_connection_rotation(original.id, original.token_generation)
            .await
            .unwrap()
            .expect("claim succeeds");
        assert_eq!(claimed.state, "rotating");
        assert!(
            db.claim_org_slack_connection_rotation(original.id, original.token_generation)
                .await
                .unwrap()
                .is_none()
        );
        let rotated = db
            .rotate_org_slack_connection(RotateOrgSlackConnection {
                id: original.id,
                expected_generation: claimed.token_generation,
                team_id: Some("T_OTHER".to_string()),
                access_token_encrypted: vec![3],
                refresh_token_encrypted: vec![4],
                access_token_expires_at: Utc::now(),
            })
            .await
            .unwrap()
            .expect("replacement succeeds");
        assert_eq!(rotated.access_token_encrypted, Some(vec![3]));
        // A recorded workspace is never overwritten by rotation.
        assert_eq!(rotated.team_id.as_deref(), Some("T1"));
    }

    #[tokio::test]
    async fn a_connection_without_a_workspace_is_due_and_rotation_fills_it() {
        let db = InMemoryDatabase::new();
        let row = db
            .upsert_org_slack_connection(UpsertOrgSlackConnection {
                access_token_expires_at: Utc::now() + chrono::Duration::hours(6),
                ..connection(41, "T1")
            })
            .await
            .unwrap();
        // A row written before migration 156 recorded workspaces.
        db.org_slack_connections
            .write()
            .get_mut(&row.id)
            .unwrap()
            .team_id = None;

        let due = db.list_due_org_slack_connections(Utc::now()).await.unwrap();
        assert_eq!(due.len(), 1, "unidentified rows rotate regardless of expiry");

        let claimed = db
            .claim_org_slack_connection_rotation(row.id, row.token_generation)
            .await
            .unwrap()
            .unwrap();
        let rotated = db
            .rotate_org_slack_connection(RotateOrgSlackConnection {
                id: row.id,
                expected_generation: claimed.token_generation,
                team_id: Some("T_FROM_SLACK".to_string()),
                access_token_encrypted: vec![3],
                refresh_token_encrypted: vec![4],
                access_token_expires_at: Utc::now() + chrono::Duration::hours(12),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rotated.team_id.as_deref(), Some("T_FROM_SLACK"));
        assert!(
            db.list_due_org_slack_connections(Utc::now())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn stale_reconnect_result_cannot_overwrite_newer_tokens() {
        let db = InMemoryDatabase::new();
        let original = db
            .upsert_org_slack_connection(connection(41, "T1"))
            .await
            .unwrap();
        let claimed = db
            .claim_org_slack_connection_rotation(original.id, original.token_generation)
            .await
            .unwrap()
            .expect("claim succeeds");
        db.upsert_org_slack_connection(connection(41, "T1"))
            .await
            .unwrap();
        assert!(
            !db.mark_org_slack_reconnect_required(original.id, claimed.token_generation)
                .await
                .unwrap()
        );
        assert_eq!(
            db.get_org_slack_connection(41, original.id)
                .await
                .unwrap()
                .unwrap()
                .state,
            "connected"
        );
    }
}
