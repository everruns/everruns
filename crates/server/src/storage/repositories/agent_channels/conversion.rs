// PostgreSQL repository: the one-time conversion of retired `api_endpoint`
// channels to `api` channels (agent execution API, phase 6).
//
// Decision: each channel converts in its own transaction that holds the row
// with `FOR UPDATE SKIP LOCKED` and re-checks its type, so replicas starting
// together never convert one channel twice, and a crash leaves the channel
// either untouched or fully converted (key, config and session tags together).
// The policy lives in `domains::agent_channels::api_endpoint_conversion`.

use super::Database;
use anyhow::Result;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

/// A retired `api_endpoint` channel as stored, with the org its agent belongs to.
#[derive(Debug, Clone, FromRow)]
pub struct ApiEndpointChannelRow {
    pub id: Uuid,
    pub public_id: String,
    pub org_id: i64,
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub auth: Option<serde_json::Value>,
    pub auth_encrypted: Option<Vec<u8>>,
    pub enabled: bool,
    pub status: String,
}

/// What a converted channel's row becomes.
#[derive(Debug, Clone)]
pub struct ConvertedApiChannelRow {
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub enabled: bool,
    pub status: String,
}

/// One `api_endpoint` channel, locked until [`LockedApiEndpointChannel::finish`]
/// commits its conversion or the value is dropped (rolled back).
pub struct LockedApiEndpointChannel {
    tx: Transaction<'static, Postgres>,
    pub row: ApiEndpointChannelRow,
}

impl Database {
    /// Ids of the channels still of the retired `api_endpoint` type.
    pub async fn list_api_endpoint_channel_ids(&self) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM agent_channels WHERE channel_type = 'api_endpoint' ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Lock `id` for conversion. `None` when it is no longer an `api_endpoint`
    /// channel or another replica holds it.
    pub async fn lock_api_endpoint_channel(
        &self,
        id: Uuid,
    ) -> Result<Option<LockedApiEndpointChannel>> {
        let mut tx = self.pool.raw().begin().await?;
        let row = sqlx::query_as::<_, ApiEndpointChannelRow>(
            r#"
            SELECT ae.id, ae.public_id, a.org_id, ae.channel_config,
                   ae.channel_config_encrypted, ae.auth, ae.auth_encrypted,
                   ae.enabled, ae.status
            FROM agent_channels AS ae
            JOIN agents AS a ON a.id = ae.agent_id
            WHERE ae.id = $1 AND ae.channel_type = 'api_endpoint'
            FOR UPDATE OF ae SKIP LOCKED
            "#,
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        Ok(row.map(|row| LockedApiEndpointChannel { tx, row }))
    }
}

impl LockedApiEndpointChannel {
    /// Store the channel's key as an agent key with the same hash, so its
    /// secret keeps working. `None` when another key already has that hash.
    pub async fn import_key(
        &mut self,
        name: &str,
        token_hash: &str,
        token_prefix: &str,
    ) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO agent_keys (org_id, channel_id, name, token_hash, token_prefix)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT DO NOTHING
            RETURNING id
            "#,
        )
        .bind(self.row.org_id)
        .bind(self.row.id)
        .bind(name)
        .bind(token_hash)
        .bind(token_prefix)
        .fetch_optional(&mut *self.tx)
        .await?)
    }

    /// Add `tags` to the channel's sessions that carry `legacy_tag` and not
    /// yet the first of `tags`. Only sessions this channel started
    /// (`sessions.channel_id`): a tag alone can be written by anyone who can
    /// edit a session.
    pub async fn retag_sessions(&mut self, legacy_tag: &str, tags: &[String]) -> Result<u64> {
        let Some(marker) = tags.first() else {
            return Ok(0);
        };
        let result = sqlx::query(
            r#"
            UPDATE sessions
            SET tags = tags || $3::TEXT[]
            WHERE org_id = $1 AND channel_id = $2
              AND tags @> ARRAY[$4]::TEXT[]
              AND NOT tags @> ARRAY[$5]::TEXT[]
            "#,
        )
        .bind(self.row.org_id)
        .bind(self.row.id)
        .bind(tags)
        .bind(legacy_tag)
        .bind(marker)
        .execute(&mut *self.tx)
        .await?;
        Ok(result.rows_affected())
    }

    /// Turn the row into an `api` channel and commit everything done under
    /// the lock.
    pub async fn finish(mut self, converted: ConvertedApiChannelRow) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE agent_channels
            SET channel_type = 'api',
                channel_config = $2,
                channel_config_encrypted = $3,
                auth = NULL,
                auth_encrypted = NULL,
                enabled = $4,
                status = $5,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(self.row.id)
        .bind(converted.channel_config)
        .bind(converted.channel_config_encrypted)
        .bind(converted.enabled)
        .bind(converted.status)
        .execute(&mut *self.tx)
        .await?;
        self.tx.commit().await?;
        Ok(())
    }
}
