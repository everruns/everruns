//! Database-clock leases fence native response journals across worker owners.

use super::EncryptionService;
use async_trait::async_trait;
use everruns_core::native_async_store::{
    MAX_NATIVE_ASYNC_CHECKPOINT_BYTES, NATIVE_ASYNC_LEASE_SECONDS, NativeAsyncLease,
    NativeAsyncStore,
};
use everruns_provider::{
    error::{AgentLoopError, Result},
    native_async::NativeAsyncCheckpoint,
};
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct PgNativeAsyncStore {
    pool: PgPool,
    encryption: Arc<EncryptionService>,
}
impl PgNativeAsyncStore {
    pub fn new(pool: PgPool, encryption: Arc<EncryptionService>) -> Self {
        Self { pool, encryption }
    }
    fn encode(&self, checkpoint: &NativeAsyncCheckpoint) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(checkpoint).map_err(store_error)?;
        if bytes.len() > MAX_NATIVE_ASYNC_CHECKPOINT_BYTES {
            return Err(AgentLoopError::store(
                "native async checkpoint exceeds 8 MiB",
            ));
        }
        self.encryption.encrypt(&bytes).map_err(store_error)
    }
    fn decode(&self, bytes: Vec<u8>) -> Result<NativeAsyncCheckpoint> {
        serde_json::from_slice(&self.encryption.decrypt(&bytes).map_err(store_error)?)
            .map_err(store_error)
    }
}
fn store_error(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::store(format!("native async journal: {error}"))
}
fn fence_error() -> AgentLoopError {
    AgentLoopError::store("native async ownership fence lost or unavailable")
}

#[async_trait]
impl NativeAsyncStore for PgNativeAsyncStore {
    // THREAT[TM-TOOL-040]: tenant and database-clock ownership fences prevent stale workers from writing live journals.
    async fn acquire(&self, lease: NativeAsyncLease) -> Result<NativeAsyncCheckpoint> {
        let initial = self.encode(&NativeAsyncCheckpoint::default())?;
        let row: Option<(Vec<u8>,)> = sqlx::query_as(r#"
            INSERT INTO native_async_checkpoints (session_id, turn_id, org_id, owner, lease_until, payload_encrypted)
            SELECT s.id, $2, s.org_id, $4, clock_timestamp() + make_interval(secs => $5), $6
            FROM sessions s WHERE s.id = $1 AND s.org_id = $3
            ON CONFLICT (session_id, turn_id) DO UPDATE
            SET owner = EXCLUDED.owner, lease_until = EXCLUDED.lease_until
            WHERE native_async_checkpoints.org_id = EXCLUDED.org_id
              AND (native_async_checkpoints.lease_until <= clock_timestamp()
                   OR native_async_checkpoints.owner = EXCLUDED.owner)
            RETURNING payload_encrypted
        "#).bind(lease.session_id).bind(lease.turn_id).bind(lease.org_id).bind(lease.owner)
            .bind(NATIVE_ASYNC_LEASE_SECONDS as f64).bind(initial).fetch_optional(&self.pool).await.map_err(store_error)?;
        self.decode(row.ok_or_else(fence_error)?.0)
    }
    async fn load(&self, lease: NativeAsyncLease) -> Result<NativeAsyncCheckpoint> {
        let row: Option<(Vec<u8>,)> = sqlx::query_as("SELECT payload_encrypted FROM native_async_checkpoints WHERE session_id=$1 AND turn_id=$2 AND org_id=$3 AND owner=$4 AND lease_until > clock_timestamp()")
            .bind(lease.session_id).bind(lease.turn_id).bind(lease.org_id).bind(lease.owner).fetch_optional(&self.pool).await.map_err(store_error)?;
        self.decode(row.ok_or_else(fence_error)?.0)
    }
    async fn renew(&self, lease: NativeAsyncLease) -> Result<()> {
        let result = sqlx::query("UPDATE native_async_checkpoints SET lease_until=clock_timestamp()+make_interval(secs => $5) WHERE session_id=$1 AND turn_id=$2 AND org_id=$3 AND owner=$4 AND lease_until > clock_timestamp()")
            .bind(lease.session_id).bind(lease.turn_id).bind(lease.org_id).bind(lease.owner).bind(NATIVE_ASYNC_LEASE_SECONDS as f64).execute(&self.pool).await.map_err(store_error)?;
        if result.rows_affected() != 1 {
            return Err(fence_error());
        }
        Ok(())
    }
    async fn save(
        &self,
        lease: NativeAsyncLease,
        checkpoint: &NativeAsyncCheckpoint,
    ) -> Result<()> {
        let payload = self.encode(checkpoint)?;
        let result = sqlx::query("UPDATE native_async_checkpoints SET payload_encrypted=$6, lease_until=clock_timestamp()+make_interval(secs => $5) WHERE session_id=$1 AND turn_id=$2 AND org_id=$3 AND owner=$4 AND lease_until > clock_timestamp()")
            .bind(lease.session_id).bind(lease.turn_id).bind(lease.org_id).bind(lease.owner).bind(NATIVE_ASYNC_LEASE_SECONDS as f64).bind(payload).execute(&self.pool).await.map_err(store_error)?;
        if result.rows_affected() != 1 {
            return Err(fence_error());
        }
        Ok(())
    }
    async fn release(&self, lease: NativeAsyncLease) -> Result<()> {
        let result = sqlx::query("UPDATE native_async_checkpoints SET lease_until=clock_timestamp() WHERE session_id=$1 AND turn_id=$2 AND org_id=$3 AND owner=$4 AND lease_until > clock_timestamp()")
            .bind(lease.session_id).bind(lease.turn_id).bind(lease.org_id).bind(lease.owner).execute(&self.pool).await.map_err(store_error)?;
        if result.rows_affected() != 1 {
            return Err(fence_error());
        }
        Ok(())
    }
}
