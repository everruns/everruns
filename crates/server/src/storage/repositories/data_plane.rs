// PostgreSQL repository: the control plane's side of /dp/v1 (data plane v2
// step 7). See `domains::data_plane` and the `data_plane_control` migration.

use anyhow::Result;
use serde_json::Value;

use super::Database;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DataPlaneRow {
    pub data_plane_id: String,
    pub url: String,
    pub admin_url: String,
}

/// An org on a data plane.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DpOrgRow {
    pub org_id: i64,
    pub data_plane_id: String,
    pub version: i64,
    pub delivered_epoch: i64,
    pub delivered_version: i64,
    pub data_key_id: String,
    /// Encrypted with the server's secrets key.
    pub data_key_encrypted: Vec<u8>,
    pub enabled: bool,
    pub state: String,
    pub attempts: i32,
}

macro_rules! org_columns {
    () => {
        "org_id, data_plane_id, version, delivered_epoch, delivered_version, \
         data_key_id, data_key_encrypted, enabled, state, attempts"
    };
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DpConfigItemRow {
    pub kind: String,
    pub item_id: String,
    pub revision: i64,
    pub body: Option<Value>,
    pub version: i64,
}

/// A grant as the ledger holds it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DpGrantRow {
    pub grant_id: String,
    pub granted_micros: i64,
    pub used_micros: i64,
    pub closed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DpSigningKeyRow {
    pub kid: String,
    /// PKCS#8, encrypted with the server's secrets key.
    pub private_key_encrypted: Vec<u8>,
    pub public_jwk: Value,
    pub retired_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One settlement as the ledger applies it.
pub struct DpSettlement<'a> {
    pub grant_id: &'a str,
    pub used_micros: i64,
    pub reserved_micros: i64,
    pub closed: bool,
}

/// What an allowance request asked for, and what may be granted.
pub struct DpAllowanceAsk<'a> {
    pub org_id: i64,
    pub request_id: &'a str,
    pub cell: &'a str,
    pub want_micros: i64,
    pub settlements: Vec<DpSettlement<'a>>,
    /// What the org's usage policies still allow, in micros; `None`: no cap.
    pub cap_micros: Option<i64>,
    /// Why nothing may be granted at all (`suspended`), if anything.
    pub refuse: Option<&'a str>,
    pub spend_until: chrono::DateTime<chrono::Utc>,
    pub per_call_micros: Option<i64>,
}

/// A session summary from the feed.
pub struct DpSessionSummary<'a> {
    pub session_id: &'a str,
    pub org_id: i64,
    pub data_plane_id: &'a str,
    pub agent_id: &'a str,
    pub session_version: i64,
    pub title: &'a str,
    pub status: &'a str,
    pub turns: i64,
    pub cost_micros: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_activity_at: chrono::DateTime<chrono::Utc>,
}

/// The channel the dispatcher listens on; the payload is the org id.
pub const DP_CONFIG_CHANNEL: &str = "dp_config";

impl Database {
    pub async fn dp_upsert_data_plane(&self, id: &str, url: &str, admin_url: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO data_planes (data_plane_id, url, admin_url) VALUES ($1, $2, $3)
             ON CONFLICT (data_plane_id) DO UPDATE
               SET url = excluded.url, admin_url = excluded.admin_url, updated_at = now()",
        )
        .bind(id)
        .bind(url)
        .bind(admin_url)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn dp_data_plane(&self, id: &str) -> Result<Option<DataPlaneRow>> {
        Ok(sqlx::query_as(
            "SELECT data_plane_id, url, admin_url FROM data_planes WHERE data_plane_id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn dp_epoch(&self) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT epoch FROM dp_control_epoch")
            .fetch_one(&self.pool)
            .await?)
    }

    /// Start a new control plane epoch (after a restore): every org's next
    /// delivery is a full snapshot.
    pub async fn dp_bump_epoch(&self) -> Result<i64> {
        let epoch: i64 =
            sqlx::query_scalar("UPDATE dp_control_epoch SET epoch = epoch + 1 RETURNING epoch")
                .fetch_one(&self.pool)
                .await?;
        sqlx::query("UPDATE dp_orgs SET next_attempt_at = now(), attempts = 0")
            .execute(&self.pool)
            .await?;
        sqlx::query("SELECT pg_notify($1, '*')")
            .bind(DP_CONFIG_CHANNEL)
            .execute(&self.pool)
            .await?;
        Ok(epoch)
    }

    pub async fn dp_org(&self, org_id: i64) -> Result<Option<DpOrgRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            org_columns!(),
            " FROM dp_orgs WHERE org_id = $1"
        ))
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Put `org_id` on `data_plane_id` with its new data key. An org already
    /// on a data plane keeps its key.
    pub async fn dp_insert_org(
        &self,
        org_id: i64,
        data_plane_id: &str,
        data_key_id: &str,
        data_key_encrypted: &[u8],
    ) -> Result<DpOrgRow> {
        sqlx::query(
            "INSERT INTO dp_orgs (org_id, data_plane_id, data_key_id, data_key_encrypted)
             VALUES ($1, $2, $3, $4) ON CONFLICT (org_id) DO NOTHING",
        )
        .bind(org_id)
        .bind(data_plane_id)
        .bind(data_key_id)
        .bind(data_key_encrypted)
        .execute(&self.pool)
        .await?;
        self.dp_org(org_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("org {org_id} vanished"))
    }

    pub async fn dp_set_org_enabled(&self, org_id: i64) -> Result<()> {
        sqlx::query("UPDATE dp_orgs SET enabled = true, updated_at = now() WHERE org_id = $1")
            .bind(org_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn dp_set_org_state(&self, org_id: i64, state: &str) -> Result<()> {
        sqlx::query("UPDATE dp_orgs SET state = $2, updated_at = now() WHERE org_id = $1")
            .bind(org_id)
            .bind(state)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Orgs whose data plane is behind, due for a delivery attempt.
    pub async fn dp_orgs_due(&self, epoch: i64, limit: i64) -> Result<Vec<DpOrgRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            org_columns!(),
            " FROM dp_orgs
              WHERE (NOT enabled OR delivered_epoch <> $1 OR delivered_version < version)
                AND next_attempt_at <= now()
              ORDER BY next_attempt_at LIMIT $2"
        ))
        .bind(epoch)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The data plane applied `version` in `epoch`.
    pub async fn dp_delivered(&self, org_id: i64, epoch: i64, version: i64) -> Result<()> {
        sqlx::query(
            "UPDATE dp_orgs SET delivered_epoch = $2, delivered_version = $3, attempts = 0,
                    last_error = NULL, next_attempt_at = now(), updated_at = now()
              WHERE org_id = $1",
        )
        .bind(org_id)
        .bind(epoch)
        .bind(version)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A delivery failed: back off (doubling, at most a minute).
    pub async fn dp_delivery_failed(&self, org_id: i64, error: &str) -> Result<()> {
        sqlx::query(
            "UPDATE dp_orgs SET attempts = attempts + 1, last_error = $2,
                    next_attempt_at = now() + make_interval(secs => LEAST(60, 0.2 * power(2, LEAST(attempts, 9)))),
                    updated_at = now()
              WHERE org_id = $1",
        )
        .bind(org_id)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Set item `(kind, item_id)` of `org_id`'s config to `body` (`None`:
    /// removed) at `revision`. An equal item is left alone; otherwise the
    /// org's version moves and the dispatcher is woken. Returns the new
    /// version, or `None` when nothing changed.
    pub async fn dp_put_item(
        &self,
        org_id: i64,
        kind: &str,
        item_id: &str,
        revision: i64,
        body: Option<&Value>,
    ) -> Result<Option<i64>> {
        let mut tx = self.pool.begin().await?;
        // Serializes the org's versions.
        let locked: Option<i64> =
            sqlx::query_scalar("SELECT version FROM dp_orgs WHERE org_id = $1 FOR UPDATE")
                .bind(org_id)
                .fetch_optional(&mut *tx)
                .await?;
        if locked.is_none() {
            return Ok(None);
        }
        let current: Option<(i64, Option<Value>)> = sqlx::query_as(
            "SELECT revision, body FROM dp_config_items WHERE org_id = $1 AND kind = $2 AND item_id = $3",
        )
        .bind(org_id)
        .bind(kind)
        .bind(item_id)
        .fetch_optional(&mut *tx)
        .await?;
        match &current {
            Some((held, _)) if *held > revision => return Ok(None),
            Some((held, held_body)) if *held == revision && held_body.as_ref() == body => {
                return Ok(None);
            }
            None if body.is_none() => return Ok(None),
            _ => {}
        }
        let version: i64 = sqlx::query_scalar(
            "UPDATE dp_orgs SET version = version + 1, updated_at = now() WHERE org_id = $1 RETURNING version",
        )
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO dp_config_items (org_id, kind, item_id, revision, body, version)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (org_id, kind, item_id) DO UPDATE
               SET revision = excluded.revision, body = excluded.body, version = excluded.version",
        )
        .bind(org_id)
        .bind(kind)
        .bind(item_id)
        .bind(revision)
        .bind(body)
        .bind(version)
        .execute(&mut *tx)
        .await?;
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(DP_CONFIG_CHANNEL)
            .bind(org_id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(version))
    }

    /// Items changed after `version` (removals included).
    pub async fn dp_items_since(&self, org_id: i64, version: i64) -> Result<Vec<DpConfigItemRow>> {
        Ok(sqlx::query_as(
            "SELECT kind, item_id, revision, body, version FROM dp_config_items
              WHERE org_id = $1 AND version > $2 ORDER BY version",
        )
        .bind(org_id)
        .bind(version)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Every live item: a full snapshot.
    pub async fn dp_items_live(&self, org_id: i64) -> Result<Vec<DpConfigItemRow>> {
        Ok(sqlx::query_as(
            "SELECT kind, item_id, revision, body, version FROM dp_config_items
              WHERE org_id = $1 AND body IS NOT NULL ORDER BY version",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn dp_item(
        &self,
        org_id: i64,
        kind: &str,
        item_id: &str,
    ) -> Result<Option<DpConfigItemRow>> {
        Ok(sqlx::query_as(
            "SELECT kind, item_id, revision, body, version FROM dp_config_items
              WHERE org_id = $1 AND kind = $2 AND item_id = $3",
        )
        .bind(org_id)
        .bind(kind)
        .bind(item_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// How far `source`'s feed is stored.
    pub async fn dp_feed_position(
        &self,
        source: &str,
        incarnation: i64,
    ) -> Result<Option<(i64, i64)>> {
        Ok(sqlx::query_as(
            "SELECT stored_index, stored_ordinal FROM dp_feed_sources WHERE source = $1 AND incarnation = $2",
        )
        .bind(source)
        .bind(incarnation)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The newest incarnation `source` (a cell) has reported from.
    pub async fn dp_feed_newest_incarnation(&self, source: &str) -> Result<Option<i64>> {
        Ok(
            sqlx::query_scalar("SELECT max(incarnation) FROM dp_feed_sources WHERE source = $1")
                .bind(source)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn dp_set_feed_position(
        &self,
        source: &str,
        incarnation: i64,
        through: (i64, i64),
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO dp_feed_sources (source, incarnation, stored_index, stored_ordinal)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (source, incarnation) DO UPDATE
               SET stored_index = excluded.stored_index, stored_ordinal = excluded.stored_ordinal,
                   updated_at = now()
               WHERE (dp_feed_sources.stored_index, dp_feed_sources.stored_ordinal)
                   < (excluded.stored_index, excluded.stored_ordinal)",
        )
        .bind(source)
        .bind(incarnation)
        .bind(through.0)
        .bind(through.1)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store a summary unless a newer one is stored.
    pub async fn dp_upsert_session_summary(&self, s: DpSessionSummary<'_>) -> Result<()> {
        sqlx::query(
            "INSERT INTO dp_session_summaries
                (session_id, org_id, data_plane_id, agent_id, session_version, title, status,
                 turns, cost_micros, created_at, last_activity_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (session_id) DO UPDATE
               SET data_plane_id = excluded.data_plane_id, agent_id = excluded.agent_id,
                   session_version = excluded.session_version, title = excluded.title,
                   status = excluded.status, turns = excluded.turns,
                   cost_micros = excluded.cost_micros,
                   last_activity_at = excluded.last_activity_at, updated_at = now()
               WHERE dp_session_summaries.session_version < excluded.session_version",
        )
        .bind(s.session_id)
        .bind(s.org_id)
        .bind(s.data_plane_id)
        .bind(s.agent_id)
        .bind(s.session_version)
        .bind(s.title)
        .bind(s.status)
        .bind(s.turns)
        .bind(s.cost_micros)
        .bind(s.created_at)
        .bind(s.last_activity_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A v2 session's record follows its data plane summary: idle after a
    /// turn, the turn's outcome, and when it ended. A row of another org,
    /// or none, is left alone.
    pub async fn dp_apply_session_summary(
        &self,
        org_id: i64,
        session_id: uuid::Uuid,
        last_turn_status: &str,
        last_turn_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET status = 'idle', last_turn_status = $3, last_turn_at = $4,
                    updated_at = now()
              WHERE org_id = $1 AND id = $2
                AND (last_turn_at IS NULL OR last_turn_at <= $4)",
        )
        .bind(org_id)
        .bind(session_id)
        .bind(last_turn_status)
        .bind(last_turn_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Whether `org_id` has, or had, sessions on a data plane.
    pub async fn dp_org_ever(&self, org_id: i64) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, i32>(
            "SELECT 1 FROM dp_orgs WHERE org_id = $1
             UNION ALL
             SELECT 1 FROM dp_session_summaries WHERE org_id = $1
             LIMIT 1",
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?
        .is_some())
    }

    /// Whether `session_id` (its public id) runs on a data plane: it has a
    /// summary from the moment the control plane created it there.
    pub async fn dp_session_on_data_plane(&self, session_id: &str) -> Result<bool> {
        Ok(
            sqlx::query_scalar::<_, i32>(
                "SELECT 1 FROM dp_session_summaries WHERE session_id = $1",
            )
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?
            .is_some(),
        )
    }

    /// v2 sessions of `org_id`, most recently active first.
    pub async fn dp_session_summaries(
        &self,
        org_id: i64,
        limit: i64,
    ) -> Result<Vec<(String, String, String, i64, i64)>> {
        Ok(sqlx::query_as(
            "SELECT session_id, status, title, turns, cost_micros FROM dp_session_summaries
              WHERE org_id = $1 ORDER BY last_activity_at DESC LIMIT $2",
        )
        .bind(org_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn dp_add_feed_cost(&self, org_id: i64, micros: i64) -> Result<()> {
        sqlx::query(
            "UPDATE dp_orgs SET feed_cost_micros = feed_cost_micros + $2 WHERE org_id = $1",
        )
        .bind(org_id)
        .bind(micros)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn dp_insert_org_event(
        &self,
        org_id: i64,
        kind: &str,
        detail: &str,
        source: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO dp_org_events (org_id, kind, detail, source, at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(org_id)
        .bind(kind)
        .bind(detail)
        .bind(source)
        .bind(at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Answer an allowance request once: apply its settlements, then grant
    /// what the org's policies still allow after every open grant and every
    /// settled cost the feed has not journaled yet. One transaction, the
    /// org's row locked, so concurrent requests never grant the same money.
    pub async fn dp_allowance(&self, ask: DpAllowanceAsk<'_>) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        let feed_cost: Option<i64> =
            sqlx::query_scalar("SELECT feed_cost_micros FROM dp_orgs WHERE org_id = $1 FOR UPDATE")
                .bind(ask.org_id)
                .fetch_optional(&mut *tx)
                .await?;
        let feed_cost = feed_cost
            .ok_or_else(|| anyhow::anyhow!("org {} is not on a data plane", ask.org_id))?;
        let answered: Option<Value> =
            sqlx::query_scalar("SELECT answer FROM dp_allowance_requests WHERE request_id = $1")
                .bind(ask.request_id)
                .fetch_optional(&mut *tx)
                .await?;
        // Settlements are cumulative: applying one again changes nothing.
        for s in &ask.settlements {
            sqlx::query(
                "UPDATE dp_grants SET used_micros = GREATEST(used_micros, $3),
                        reserved_micros = $4, closed = closed OR $5,
                        closed_at = CASE WHEN $5 AND closed_at IS NULL THEN now() ELSE closed_at END
                  WHERE grant_id = $1 AND org_id = $2 AND NOT closed",
            )
            .bind(s.grant_id)
            .bind(ask.org_id)
            .bind(s.used_micros)
            .bind(s.reserved_micros)
            .bind(s.closed)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(answer) = answered {
            tx.commit().await?;
            return Ok(answer);
        }
        let (open, used_all): (Option<i64>, Option<i64>) = sqlx::query_as(
            "SELECT SUM(CASE WHEN closed THEN 0 ELSE granted_micros - used_micros END)::BIGINT,
                    SUM(used_micros)::BIGINT
               FROM dp_grants WHERE org_id = $1",
        )
        .bind(ask.org_id)
        .fetch_one(&mut *tx)
        .await?;
        let exposure = open.unwrap_or(0) + (used_all.unwrap_or(0) - feed_cost).max(0);
        let available = match (ask.refuse, ask.cap_micros) {
            (Some(_), _) => 0,
            (None, Some(cap)) => (cap - exposure).max(0),
            (None, None) => ask.want_micros,
        };
        let amount = ask.want_micros.min(available).max(0);
        let grant_id = format!("grant_{}", uuid::Uuid::new_v4().simple());
        if amount > 0 {
            sqlx::query(
                "INSERT INTO dp_grants (grant_id, org_id, cell, granted_micros, spend_until)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&grant_id)
            .bind(ask.org_id)
            .bind(ask.cell)
            .bind(amount)
            .bind(ask.spend_until)
            .execute(&mut *tx)
            .await?;
        }
        let reason = match (amount, ask.refuse) {
            (0, Some(why)) => Some(why.to_owned()),
            (0, None) if ask.want_micros > 0 => Some("budget_exhausted".to_owned()),
            _ => None,
        };
        let answer = serde_json::to_value(everruns_contracts::dataplane::AllowanceGrant {
            grant_id,
            granted_micros: amount,
            spend_until_unix_ms: ask.spend_until.timestamp_millis(),
            reason,
            per_call_micros: ask.per_call_micros,
        })?;
        sqlx::query(
            "INSERT INTO dp_allowance_requests (request_id, org_id, answer) VALUES ($1, $2, $3)",
        )
        .bind(ask.request_id)
        .bind(ask.org_id)
        .bind(&answer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(answer)
    }

    /// Grants of `org_id`, newest first.
    pub async fn dp_grants(&self, org_id: i64) -> Result<Vec<DpGrantRow>> {
        Ok(sqlx::query_as(
            "SELECT grant_id, granted_micros, used_micros, closed FROM dp_grants
              WHERE org_id = $1 ORDER BY created_at DESC",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn dp_signing_keys(&self) -> Result<Vec<DpSigningKeyRow>> {
        Ok(sqlx::query_as(
            "SELECT kid, private_key_encrypted, public_jwk, retired_at FROM dp_signing_keys
              ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Store a new signing key; a concurrent first key wins and is returned.
    pub async fn dp_insert_signing_key(&self, row: &DpSigningKeyRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO dp_signing_keys (kid, private_key_encrypted, public_jwk)
             SELECT $1, $2, $3
              WHERE NOT EXISTS (SELECT 1 FROM dp_signing_keys WHERE retired_at IS NULL)",
        )
        .bind(&row.kid)
        .bind(&row.private_key_encrypted)
        .bind(&row.public_jwk)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
