// PostgreSQL repository: the shared channel host's bindings, delivery keys
// and pending deliveries (migration 210).
//
// Decisions:
// - Binding is `INSERT .. ON CONFLICT DO NOTHING` followed by a read, so the
//   first writer wins and every caller learns the winner.
// - Delivery keys older than a day are pruned by the next sighting on the same
//   channel: no reaper, and the index on (channel_id, seen_at) keeps it cheap.
// - A pending delivery is claimable once its lease ran out; claiming and
//   renewing set the holder, so two instances never run one delivery.

use super::Database;
use anyhow::Result;
use std::time::Duration;
use uuid::Uuid;

/// One reply still owed to a platform.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChannelPendingDeliveryRow {
    pub channel_id: Uuid,
    pub session_id: Uuid,
    pub input_message_id: String,
    pub target: serde_json::Value,
    pub after_sequence: i64,
}

fn interval_secs(lease: Duration) -> f64 {
    lease.as_secs_f64()
}

impl Database {
    /// The session bound to `binding_key` on a channel.
    pub async fn channel_binding(
        &self,
        channel_id: Uuid,
        binding_key: &str,
    ) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar::<_, Uuid>(
            "SELECT session_id FROM channel_bindings WHERE channel_id = $1 AND binding_key = $2",
        )
        .bind(channel_id)
        .bind(binding_key)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Bind `binding_key` to `session_id` unless it is bound already; returns
    /// the session it is bound to afterwards.
    pub async fn bind_channel_session(
        &self,
        channel_id: Uuid,
        binding_key: &str,
        session_id: Uuid,
    ) -> Result<Uuid> {
        Ok(sqlx::query_scalar::<_, Uuid>(
            r#"
            WITH inserted AS (
                INSERT INTO channel_bindings (channel_id, binding_key, session_id)
                VALUES ($1, $2, $3)
                ON CONFLICT (channel_id, binding_key) DO NOTHING
                RETURNING session_id
            )
            SELECT session_id FROM inserted
            UNION ALL
            SELECT session_id FROM channel_bindings
            WHERE channel_id = $1 AND binding_key = $2
            LIMIT 1
            "#,
        )
        .bind(channel_id)
        .bind(binding_key)
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Record a platform delivery key: `true` the first time it is seen.
    pub async fn record_channel_delivery_key(
        &self,
        channel_id: Uuid,
        dedup_key: &str,
    ) -> Result<bool> {
        let inserted = sqlx::query_scalar::<_, i32>(
            r#"
            WITH pruned AS (
                DELETE FROM channel_seen
                WHERE channel_id = $1 AND seen_at < NOW() - INTERVAL '1 day'
            )
            INSERT INTO channel_seen (channel_id, dedup_key)
            VALUES ($1, $2)
            ON CONFLICT (channel_id, dedup_key) DO NOTHING
            RETURNING 1
            "#,
        )
        .bind(channel_id)
        .bind(dedup_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(inserted.is_some())
    }

    /// Save a pending delivery held by `holder` for `lease`.
    pub async fn save_channel_pending_delivery(
        &self,
        row: &ChannelPendingDeliveryRow,
        holder: &str,
        lease: Duration,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO channel_pending_deliveries
                (session_id, input_message_id, channel_id, target, after_sequence, holder, lease_until)
            VALUES ($1, $2, $3, $4, $5, $6, NOW() + make_interval(secs => $7))
            ON CONFLICT (session_id, input_message_id) DO UPDATE SET
                target = EXCLUDED.target,
                after_sequence = EXCLUDED.after_sequence,
                holder = EXCLUDED.holder,
                lease_until = EXCLUDED.lease_until
            "#,
        )
        .bind(row.session_id)
        .bind(&row.input_message_id)
        .bind(row.channel_id)
        .bind(&row.target)
        .bind(row.after_sequence)
        .bind(holder)
        .bind(interval_secs(lease))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Extend `holder`'s lease on a delivery it runs.
    pub async fn renew_channel_pending_delivery(
        &self,
        session_id: Uuid,
        input_message_id: &str,
        holder: &str,
        lease: Duration,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE channel_pending_deliveries
            SET lease_until = NOW() + make_interval(secs => $4)
            WHERE session_id = $1 AND input_message_id = $2 AND holder = $3
            "#,
        )
        .bind(session_id)
        .bind(input_message_id)
        .bind(holder)
        .bind(interval_secs(lease))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Clear a delivery whose turn ended.
    pub async fn clear_channel_pending_delivery(
        &self,
        session_id: Uuid,
        input_message_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "DELETE FROM channel_pending_deliveries WHERE session_id = $1 AND input_message_id = $2",
        )
        .bind(session_id)
        .bind(input_message_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Claim every delivery whose lease ran out for `holder`.
    pub async fn claim_channel_pending_deliveries(
        &self,
        holder: &str,
        lease: Duration,
    ) -> Result<Vec<ChannelPendingDeliveryRow>> {
        Ok(sqlx::query_as::<_, ChannelPendingDeliveryRow>(
            r#"
            UPDATE channel_pending_deliveries AS pending
            SET holder = $1, lease_until = NOW() + make_interval(secs => $2)
            FROM (
                SELECT session_id, input_message_id
                FROM channel_pending_deliveries
                WHERE lease_until < NOW()
                ORDER BY created_at
                LIMIT 500
                FOR UPDATE SKIP LOCKED
            ) AS expired
            WHERE pending.session_id = expired.session_id
              AND pending.input_message_id = expired.input_message_id
            RETURNING pending.channel_id, pending.session_id, pending.input_message_id,
                pending.target, pending.after_sequence
            "#,
        )
        .bind(holder)
        .bind(interval_secs(lease))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Give a claimed delivery back so a later recovery can take it.
    pub async fn release_channel_pending_delivery(
        &self,
        session_id: Uuid,
        input_message_id: &str,
        holder: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE channel_pending_deliveries
            SET lease_until = NOW()
            WHERE session_id = $1 AND input_message_id = $2 AND holder = $3
            "#,
        )
        .bind(session_id)
        .bind(input_message_id)
        .bind(holder)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The ingress row of a channel by its internal id, for the channel host's
    /// resolver.
    pub async fn get_ingress_channel_by_id(
        &self,
        channel_id: Uuid,
    ) -> Result<Option<crate::storage::IngressChannelRow>> {
        let public_id =
            sqlx::query_scalar::<_, String>("SELECT public_id FROM agent_channels WHERE id = $1")
                .bind(channel_id)
                .fetch_optional(&self.pool)
                .await?;
        match public_id {
            Some(public_id) => self.get_ingress_channel_by_public_id(&public_id).await,
            None => Ok(None),
        }
    }
}
