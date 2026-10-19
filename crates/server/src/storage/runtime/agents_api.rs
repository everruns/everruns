//! Database-clock leases fence OpenAI Agents API orchestration state across
//! worker owners (EVE-1123). Mirrors the native async journal: tenant and
//! owner checks on every statement, encrypted payload, bounded size.

use crate::storage::EncryptionService;
use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_core::agents_api_store::{
    AGENTS_API_LEASE_SECONDS, AgentsApiCheckpoint, AgentsApiLease, AgentsApiStore,
    MAX_AGENTS_API_CHECKPOINT_BYTES,
};
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct PgAgentsApiStore {
    pool: PgPool,
    encryption: Arc<EncryptionService>,
}

impl PgAgentsApiStore {
    pub fn new(pool: PgPool, encryption: Arc<EncryptionService>) -> Self {
        Self { pool, encryption }
    }

    /// The store over a PostgreSQL backend, when encryption is configured.
    pub fn shared(
        db: &crate::storage::StorageBackend,
        encryption: Option<&Arc<EncryptionService>>,
    ) -> Option<Arc<dyn AgentsApiStore>> {
        Some(Arc::new(Self::new(db.pool().clone(), encryption?.clone())))
    }

    fn encode(&self, checkpoint: &AgentsApiCheckpoint) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(checkpoint).map_err(store_error)?;
        if bytes.len() > MAX_AGENTS_API_CHECKPOINT_BYTES {
            return Err(AgentLoopError::store("agents api checkpoint exceeds 8 MiB"));
        }
        self.encryption.encrypt(&bytes).map_err(store_error)
    }

    fn decode(&self, bytes: Vec<u8>) -> Result<AgentsApiCheckpoint> {
        serde_json::from_slice(&self.encryption.decrypt(&bytes).map_err(store_error)?)
            .map_err(store_error)
    }
}

impl PgAgentsApiStore {
    /// Retention (EVE-1126): release the provider sessions of checkpoints
    /// idle for longer than `idle_for`, at most `limit` per call. Only an
    /// unleased checkpoint of a session that is not mid-turn, and that holds
    /// no parked call, is released. The Everruns record stays; the next turn
    /// starts a new provider session. The tombstone trigger queues the
    /// released id for remote deletion in the same statement. Returns how
    /// many provider sessions were released.
    pub async fn release_idle_provider_sessions(
        &self,
        idle_for: std::time::Duration,
        limit: i64,
    ) -> Result<u64> {
        let rows: Vec<(uuid::Uuid, Vec<u8>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            r#"
            SELECT a.id, a.payload_encrypted, a.updated_at
            FROM agents_api_sessions a
            JOIN sessions s ON s.id = a.session_id
            WHERE a.provider_session_id IS NOT NULL
              AND a.updated_at < clock_timestamp() - make_interval(secs => $1)
              AND a.lease_until <= clock_timestamp()
              AND s.status IN ('started', 'idle')
            ORDER BY a.updated_at
            LIMIT $2
            "#,
        )
        .bind(idle_for.as_secs_f64())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        let mut released = 0;
        for (id, payload, updated_at) in rows {
            let mut checkpoint = self.decode(payload)?;
            if checkpoint.holds_pause() {
                continue;
            }
            checkpoint.release_provider_session();
            let payload = self.encode(&checkpoint)?;
            // Compare-and-swap on updated_at and the lease: a worker that
            // took the session meanwhile keeps it.
            let result = sqlx::query(
                "UPDATE agents_api_sessions SET payload_encrypted = $2, provider_session_id = NULL, \
                 updated_at = clock_timestamp() \
                 WHERE id = $1 AND updated_at = $3 AND lease_until <= clock_timestamp() \
                 AND provider_session_id IS NOT NULL",
            )
            .bind(id)
            .bind(payload)
            .bind(updated_at)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            released += result.rows_affected();
        }
        Ok(released)
    }
}

fn store_error(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::store(format!("agents api journal: {error}"))
}

fn fence_error() -> AgentLoopError {
    AgentLoopError::store("agents api ownership fence lost or unavailable")
}

#[async_trait]
impl AgentsApiStore for PgAgentsApiStore {
    // THREAT[TM-TOOL-051]: tenant and database-clock ownership fences keep a
    // stale worker from submitting inputs or tool results to a provider session.
    async fn acquire(&self, lease: AgentsApiLease) -> Result<AgentsApiCheckpoint> {
        let initial = self.encode(&AgentsApiCheckpoint::default())?;
        let row: Option<(Vec<u8>,)> = sqlx::query_as(
            r#"
            INSERT INTO agents_api_sessions (session_id, org_id, owner, lease_until, payload_encrypted)
            SELECT s.id, s.org_id, $3, clock_timestamp() + make_interval(secs => $4), $5
            FROM sessions s WHERE s.id = $1 AND s.org_id = $2
            ON CONFLICT (session_id) DO UPDATE
            SET owner = EXCLUDED.owner, lease_until = EXCLUDED.lease_until
            WHERE agents_api_sessions.org_id = EXCLUDED.org_id
              AND (agents_api_sessions.lease_until <= clock_timestamp()
                   OR agents_api_sessions.owner = EXCLUDED.owner)
            RETURNING payload_encrypted
            "#,
        )
        .bind(lease.session_id)
        .bind(lease.org_id)
        .bind(lease.owner)
        .bind(AGENTS_API_LEASE_SECONDS as f64)
        .bind(initial)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        self.decode(row.ok_or_else(fence_error)?.0)
    }

    async fn renew(&self, lease: AgentsApiLease) -> Result<()> {
        let result = sqlx::query(
            "UPDATE agents_api_sessions SET lease_until = clock_timestamp() + make_interval(secs => $4) \
             WHERE session_id = $1 AND org_id = $2 AND owner = $3 AND lease_until > clock_timestamp()",
        )
        .bind(lease.session_id)
        .bind(lease.org_id)
        .bind(lease.owner)
        .bind(AGENTS_API_LEASE_SECONDS as f64)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        if result.rows_affected() != 1 {
            return Err(fence_error());
        }
        Ok(())
    }

    async fn save(&self, lease: AgentsApiLease, checkpoint: &AgentsApiCheckpoint) -> Result<()> {
        let payload = self.encode(checkpoint)?;
        let result = sqlx::query(
            "UPDATE agents_api_sessions SET payload_encrypted = $5, provider_session_id = $6, \
             provider_key = $7, updated_at = clock_timestamp(), lease_until = clock_timestamp() + make_interval(secs => $4) \
             WHERE session_id = $1 AND org_id = $2 AND owner = $3 AND lease_until > clock_timestamp()",
        )
        .bind(lease.session_id)
        .bind(lease.org_id)
        .bind(lease.owner)
        .bind(AGENTS_API_LEASE_SECONDS as f64)
        .bind(payload)
        .bind(checkpoint.provider_session_id.as_deref())
        .bind(checkpoint.provider_key.as_deref())
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        if result.rows_affected() != 1 {
            return Err(fence_error());
        }
        Ok(())
    }

    async fn release(&self, lease: AgentsApiLease) -> Result<()> {
        let result = sqlx::query(
            "UPDATE agents_api_sessions SET lease_until = clock_timestamp() \
             WHERE session_id = $1 AND org_id = $2 AND owner = $3 AND lease_until > clock_timestamp()",
        )
        .bind(lease.session_id)
        .bind(lease.org_id)
        .bind(lease.owner)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        if result.rows_affected() != 1 {
            return Err(fence_error());
        }
        Ok(())
    }
}
