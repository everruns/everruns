// PostgreSQL repository: data plane v2 review fixes (the `dp_review_fixes`
// migration): host call receipts, the org settings sync obligation, and
// the signing secrets of v2 task push configs.

use anyhow::Result;
use serde_json::Value;

use super::Database;

/// A host call's receipt as [`Database::dp_take_host_call_receipt`] finds it.
#[derive(Debug, Clone, PartialEq)]
pub enum HostCallReceipt {
    /// First time: the caller runs the call and stores its answer.
    New,
    /// Seen and finished: the answer the first run sealed.
    Answered(Value),
    /// Seen and not finished: running now, or it stopped midway.
    Unknown,
}

impl Database {
    /// Take the receipt `receipt_id` of a write host call of `org_id`.
    pub async fn dp_take_host_call_receipt(
        &self,
        receipt_id: &str,
        org_id: i64,
        call_name: &str,
    ) -> Result<HostCallReceipt> {
        let inserted = sqlx::query(
            "INSERT INTO dp_host_call_receipts (receipt_id, org_id, call_name)
             VALUES ($1, $2, $3) ON CONFLICT (receipt_id) DO NOTHING",
        )
        .bind(receipt_id)
        .bind(org_id)
        .bind(call_name)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if inserted == 1 {
            return Ok(HostCallReceipt::New);
        }
        // Another org's id reads as unknown: it never sees that answer.
        let answer: Option<Option<Value>> = sqlx::query_scalar(
            "SELECT answer FROM dp_host_call_receipts WHERE receipt_id = $1 AND org_id = $2",
        )
        .bind(receipt_id)
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match answer.flatten() {
            Some(answer) => HostCallReceipt::Answered(answer),
            None => HostCallReceipt::Unknown,
        })
    }

    /// Record the sealed answer of the call `receipt_id`.
    pub async fn dp_answer_host_call_receipt(
        &self,
        receipt_id: &str,
        answer: &Value,
    ) -> Result<()> {
        sqlx::query("UPDATE dp_host_call_receipts SET answer = $2 WHERE receipt_id = $1")
            .bind(receipt_id)
            .bind(answer)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Drop receipts older than `days`: a call is retried within seconds,
    /// so an older receipt guards nothing.
    pub async fn dp_expire_host_call_receipts(&self, days: i32) -> Result<u64> {
        Ok(sqlx::query(
            "DELETE FROM dp_host_call_receipts
              WHERE created_at < now() - make_interval(days => $1)",
        )
        .bind(days)
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    /// Orgs whose settings changed since their last sync: `(org, rev)`.
    pub async fn dp_orgs_settings_behind(&self, limit: i64) -> Result<Vec<(i64, i64)>> {
        Ok(sqlx::query_as(
            "SELECT org_id, settings_rev FROM dp_orgs
              WHERE settings_rev > settings_synced_rev
              ORDER BY org_id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// `org_id`'s settings change count, when it is on a data plane.
    pub async fn dp_org_settings_rev(&self, org_id: i64) -> Result<Option<i64>> {
        Ok(
            sqlx::query_scalar("SELECT settings_rev FROM dp_orgs WHERE org_id = $1")
                .bind(org_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// `org_id`'s settings are synced up to change `rev`.
    pub async fn dp_settings_synced(&self, org_id: i64, rev: i64) -> Result<()> {
        sqlx::query(
            "UPDATE dp_orgs SET settings_synced_rev = GREATEST(settings_synced_rev, $2)
              WHERE org_id = $1",
        )
        .bind(org_id)
        .bind(rev)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The org of v2 session `session_id`, if it is one.
    pub async fn dp_session_org(&self, session_id: &str) -> Result<Option<i64>> {
        Ok(
            sqlx::query_scalar("SELECT org_id FROM dp_session_summaries WHERE session_id = $1")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Keep the signing secret of push config `config_id` (a retry with the
    /// same config keeps the first).
    pub async fn dp_put_task_push_secret(
        &self,
        config_id: &str,
        org_id: i64,
        session_id: &str,
        secret_encrypted: &[u8],
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO dp_task_push_secrets (config_id, org_id, session_id, secret_encrypted)
             VALUES ($1, $2, $3, $4) ON CONFLICT (config_id) DO NOTHING",
        )
        .bind(config_id)
        .bind(org_id)
        .bind(session_id)
        .bind(secret_encrypted)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The encrypted signing secret of `org_id`'s push config `config_id`.
    pub async fn dp_task_push_secret(
        &self,
        org_id: i64,
        session_id: &str,
        config_id: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(sqlx::query_scalar(
            "SELECT secret_encrypted FROM dp_task_push_secrets
              WHERE config_id = $1 AND org_id = $2 AND session_id = $3",
        )
        .bind(config_id)
        .bind(org_id)
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?)
    }
}
