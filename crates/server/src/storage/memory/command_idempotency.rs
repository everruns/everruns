//! Command idempotency keys in memory, with the same claim rules as
//! PostgreSQL (`storage::repositories::command_idempotency`).

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::command_idempotency::{
    ClaimIdempotencyKey, IdempotencyClaim, IdempotencyKeyScope, StoredIdempotencyKey,
};

pub(crate) type IdempotencyRowKey = (i64, Uuid, String);

#[derive(Clone, Debug)]
pub(crate) struct MemoryIdempotencyKey {
    stored: StoredIdempotencyKey,
    locked_until: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

fn row_key(scope: &IdempotencyKeyScope) -> IdempotencyRowKey {
    (scope.org_id, scope.principal_id, scope.key.clone())
}

impl InMemoryDatabase {
    pub async fn claim_command_idempotency_key(
        &self,
        input: &ClaimIdempotencyKey,
    ) -> Result<IdempotencyClaim> {
        let now = Utc::now();
        let mut rows = self.command_idempotency_keys.write();
        rows.retain(|_, row| row.expires_at >= now);
        let key = row_key(&input.scope);
        if let Some(row) = rows.get(&key) {
            let abandoned = row.stored.response.is_none()
                && row.locked_until < now
                && row.stored.fingerprint == input.fingerprint;
            if !abandoned {
                return Ok(IdempotencyClaim::Existing(row.stored.clone()));
            }
        }
        rows.insert(
            key,
            MemoryIdempotencyKey {
                stored: StoredIdempotencyKey {
                    command: input.command.clone(),
                    fingerprint: input.fingerprint.clone(),
                    response: None,
                },
                locked_until: input.locked_until,
                expires_at: input.expires_at,
            },
        );
        Ok(IdempotencyClaim::Claimed)
    }

    pub async fn complete_command_idempotency_key(
        &self,
        scope: &IdempotencyKeyScope,
        response: &[u8],
    ) -> Result<()> {
        if let Some(row) = self
            .command_idempotency_keys
            .write()
            .get_mut(&row_key(scope))
        {
            row.stored.response = Some(response.to_vec());
        }
        Ok(())
    }

    pub async fn release_command_idempotency_key(&self, scope: &IdempotencyKeyScope) -> Result<()> {
        let mut rows = self.command_idempotency_keys.write();
        let key = row_key(scope);
        if rows
            .get(&key)
            .is_some_and(|row| row.stored.response.is_none())
        {
            rows.remove(&key);
        }
        Ok(())
    }
}
