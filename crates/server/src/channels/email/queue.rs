//! Database-fenced inbox batches. At most one batch per inbox and four per org.
//! A read does not mark work complete. Every disposition is independently fenced
//! so partial progress survives crashes and an expired worker cannot erase mail.

use channelkit_agentmail::EmailEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

pub const BATCH_SIZE: i64 = 25;
pub const BATCH_BYTES: i64 = 16_384;
const MAX_PENDING: i64 = 10_000;
const MAX_STORED_BYTES: i64 = 256 * 1024 * 1024;

/// Bounded retention sweeps preserve deduplication after body expiration. Held
/// mail and uncertain sends require review and are never silently discarded.
pub async fn retain(pool: &PgPool) -> anyhow::Result<()> {
    let mut tx = everruns_durable::begin_transaction(pool).await?;
    let ids:Vec<Uuid>=sqlx::query_scalar("SELECT m.id FROM email_intake m WHERE m.archived_at IS NULL AND m.state IN ('completed','ignored','failed') AND m.updated_at<now()-interval '30 days' AND NOT EXISTS(SELECT 1 FROM email_outbox o WHERE o.intake_id=m.id AND o.state NOT IN ('sent','failed')) ORDER BY m.updated_at LIMIT 100 FOR UPDATE SKIP LOCKED").fetch_all(&mut *tx).await?;
    sqlx::query("UPDATE email_outbox SET body='{}'::jsonb WHERE intake_id=ANY($1)")
        .bind(&ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE email_intake SET message='{}'::jsonb,stored_bytes=2,sender='',subject='',preview='',archived_at=now() WHERE id=ANY($1)").bind(&ids).execute(&mut *tx).await?;
    let expired:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM email_intake WHERE archived_at<now()-interval '90 days' ORDER BY archived_at LIMIT 100 FOR UPDATE SKIP LOCKED").fetch_all(&mut *tx).await?;
    sqlx::query("DELETE FROM email_outbox WHERE intake_id=ANY($1)")
        .bind(&expired)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM email_intake WHERE id=ANY($1)")
        .bind(&expired)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM email_rate_limits WHERE window_start<now()-interval '1 day'")
        .execute(&mut *tx)
        .await?;
    everruns_durable::commit_transaction(tx).await?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, FromRow)]
pub struct MailSummary {
    pub id: Uuid,
    pub inbox_id: Uuid,
    pub message_id: String,
    pub thread_id: String,
    pub sender: String,
    pub subject: String,
    pub preview: String,
    pub state: String,
    pub reason: Option<String>,
    pub received_at: DateTime<Utc>,
}
const SUMMARY: &str =
    "id,inbox_id,message_id,thread_id,sender,subject,preview,state,reason,received_at";

#[derive(Serialize)]
pub struct Batch {
    pub lease_id: Uuid,
    pub messages: Vec<MailSummary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Inspect,
    Ignore,
    Review,
    Defer,
}
impl Disposition {
    fn state(&self) -> &'static str {
        match self {
            Self::Inspect => "held",
            Self::Ignore => "ignored",
            Self::Review => "held",
            Self::Defer => "deferred",
        }
    }
}

#[derive(Clone)]
pub struct EmailQueue {
    pool: PgPool,
}
impl EmailQueue {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// An organization row serializes all admission/claim quota reservations,
    /// including different inboxes and different server processes.
    async fn lock_org(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: i64,
    ) -> anyhow::Result<()> {
        sqlx::query("SELECT org_id FROM organizations WHERE org_id=$1 FOR UPDATE")
            .bind(org)
            .fetch_one(&mut **tx)
            .await?;
        Ok(())
    }
    pub async fn ingest(&self, org: i64, inbox: Uuid, event: &EmailEvent) -> anyhow::Result<bool> {
        let body = serde_json::to_value(&event.message)?;
        let bytes = serde_json::to_vec(&body)?.len();
        anyhow::ensure!(bytes <= 1_048_576, "email body limit");
        let mut tx = everruns_durable::begin_transaction(&self.pool).await?;
        Self::lock_org(&mut tx, org).await?;
        let enabled: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM email_inboxes WHERE org_id=$1 AND id=$2 AND enabled)",
        )
        .bind(org)
        .bind(inbox)
        .fetch_one(&mut *tx)
        .await?;
        anyhow::ensure!(enabled, "inbox disabled");
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM email_intake WHERE org_id=$1 AND inbox_id=$2 AND (event_id=$3 OR message_id=$4))").bind(org).bind(inbox).bind(&event.event_id).bind(&event.message.message_id).fetch_one(&mut *tx).await?;
        if exists {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(false);
        }
        let (count, size): (i64, i64) = sqlx::query_as("SELECT count(*) FILTER (WHERE archived_at IS NULL),coalesce(sum(stored_bytes),0)::bigint FROM email_intake WHERE org_id=$1").bind(org).fetch_one(&mut *tx).await?;
        anyhow::ensure!(
            count < MAX_PENDING && size + bytes as i64 <= MAX_STORED_BYTES,
            "email storage quota"
        );
        let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM email_intake WHERE org_id=$1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
        anyhow::ensure!(receipts < 100_000, "email receipt quota");
        // All labels are hints. Unsafe/provider-held messages stay out of triage.
        let held = event.message.labels.iter().any(|s| {
            matches!(
                s.as_str(),
                "spam" | "blocked" | "unauthenticated" | "agent-armor-held"
            )
        });
        sqlx::query("INSERT INTO email_intake (id,org_id,inbox_id,event_id,message_id,thread_id,sender,subject,preview,message,stored_bytes,state,reason) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
            .bind(Uuid::now_v7()).bind(org).bind(inbox).bind(&event.event_id).bind(&event.message.message_id).bind(&event.message.thread_id)
            .bind(excerpt(&event.message.from, 256)).bind(excerpt(&event.message.subject, 256)).bind(excerpt(&event.message.preview, 400))
            .bind(body).bind(bytes as i32).bind(if held { "held" } else { "pending" }).bind(if held { Some("provider_hold") } else { None }).execute(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        Ok(true)
    }
    pub async fn list(
        &self,
        org: i64,
        inbox: Uuid,
        before: Option<Uuid>,
    ) -> anyhow::Result<Vec<MailSummary>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {SUMMARY} FROM email_intake WHERE org_id=$1 AND inbox_id=$2 AND archived_at IS NULL AND ($3::uuid IS NULL OR id < $3) ORDER BY id DESC LIMIT 100")))
            .bind(org).bind(inbox).bind(before).fetch_all(&self.pool).await?)
    }
    pub async fn message(
        &self,
        org: i64,
        inbox: Uuid,
        id: Uuid,
    ) -> anyhow::Result<Option<channelkit_agentmail::EmailMessage>> {
        let value: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT message FROM email_intake WHERE org_id=$1 AND inbox_id=$2 AND id=$3 AND archived_at IS NULL",
        )
        .bind(org)
        .bind(inbox)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        value
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }
    pub async fn claim(&self, org: i64, inbox: Uuid) -> anyhow::Result<Option<Batch>> {
        let mut tx = everruns_durable::begin_transaction(&self.pool).await?;
        Self::lock_org(&mut tx, org).await?;
        sqlx::query("UPDATE email_intake SET state=CASE WHEN attempts>=3 THEN 'held' ELSE 'pending' END,reason='triage_lease_expired',lease_id=NULL,lease_until=NULL WHERE org_id=$1 AND state='triaging' AND lease_until<=now()")
            .bind(org).execute(&mut *tx).await?;
        let (active, same): (i64, bool) = sqlx::query_as("SELECT count(DISTINCT lease_id),coalesce(bool_or(inbox_id=$2),false) FROM email_intake WHERE org_id=$1 AND state='triaging'").bind(org).bind(inbox).fetch_one(&mut *tx).await?;
        if active >= 4 || same {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(None);
        }
        let enabled: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM email_inboxes WHERE org_id=$1 AND id=$2 AND enabled)",
        )
        .bind(org)
        .bind(inbox)
        .fetch_one(&mut *tx)
        .await?;
        if !enabled {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(None);
        }
        let (pending, old_enough): (i64, bool) = sqlx::query_as("SELECT count(*),coalesce(min(received_at)<=now()-interval '5 seconds',false) FROM email_intake WHERE org_id=$1 AND inbox_id=$2 AND state IN ('pending','deferred') AND available_at<=now()").bind(org).bind(inbox).fetch_one(&mut *tx).await?;
        if pending < BATCH_SIZE && !old_enough {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(None);
        }
        if !reserve(&mut tx, org, "triage", 4, 60).await? {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(None);
        }
        let lease_id = Uuid::now_v7();
        let query = format!(
            "WITH candidates AS (SELECT id,sum(octet_length(sender)+octet_length(subject)+octet_length(preview)+256) OVER (ORDER BY received_at,id) AS bytes FROM email_intake WHERE org_id=$1 AND inbox_id=$2 AND state IN ('pending','deferred') AND available_at<=now() ORDER BY received_at,id LIMIT $3), chosen AS (SELECT id FROM candidates WHERE bytes <= $4) UPDATE email_intake SET state='triaging',attempts=attempts+1,lease_id=$5,lease_until=now()+interval '2 minutes',updated_at=now() WHERE id IN (SELECT id FROM chosen) RETURNING {SUMMARY}"
        );
        let messages = sqlx::query_as(sqlx::AssertSqlSafe(query))
            .bind(org)
            .bind(inbox)
            .bind(BATCH_SIZE)
            .bind(BATCH_BYTES)
            .bind(lease_id)
            .fetch_all(&mut *tx)
            .await?;
        everruns_durable::commit_transaction(tx).await?;
        Ok(Some(Batch { lease_id, messages }))
    }
    pub async fn claim_run(&self, org: i64, inbox: Uuid) -> anyhow::Result<Option<(Uuid, Uuid)>> {
        let mut tx = everruns_durable::begin_transaction(&self.pool).await?;
        Self::lock_org(&mut tx, org).await?;
        sqlx::query("UPDATE email_intake SET state=CASE WHEN dispatch_attempts>=3 THEN 'held' ELSE 'ready' END,lease_id=NULL,lease_until=NULL,reason='handoff_lease_expired' WHERE org_id=$1 AND state='running' AND lease_id IS NOT NULL AND lease_until<=now()")
            .bind(org).execute(&mut *tx).await?;
        let (active,same):(i64,bool)=sqlx::query_as("SELECT count(*),coalesce(bool_or(inbox_id=$2),false) FROM email_intake WHERE org_id=$1 AND state='running'").bind(org).bind(inbox).fetch_one(&mut *tx).await?;
        if active >= 4 || same {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(None);
        }
        let id:Option<Uuid>=sqlx::query_scalar("SELECT m.id FROM email_intake m JOIN email_inboxes i ON i.id=m.inbox_id WHERE m.org_id=$1 AND m.inbox_id=$2 AND m.state='ready' AND m.available_at<=now() AND i.enabled ORDER BY m.received_at,m.id LIMIT 1 FOR UPDATE OF m")
            .bind(org).bind(inbox).fetch_optional(&mut *tx).await?;
        let Some(id) = id else {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(None);
        };
        if !reserve(&mut tx, org, "execution", 120, 3600).await? {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(None);
        }
        let lease = Uuid::now_v7();
        sqlx::query("UPDATE email_intake SET state='running',dispatch_attempts=dispatch_attempts+1,lease_id=$2,lease_until=now()+interval '2 minutes',updated_at=now() WHERE id=$1").bind(id).bind(lease).execute(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        Ok(Some((id, lease)))
    }
    /// Triage cannot approve a send: inspect is a hold awaiting content/policy
    /// checks. Omitted IDs remain leased and recover on expiration.
    pub async fn dispose(
        &self,
        org: i64,
        inbox: Uuid,
        id: Uuid,
        lease: Uuid,
        disposition: Disposition,
        reason: &str,
    ) -> anyhow::Result<bool> {
        let result = sqlx::query("UPDATE email_intake SET state=$5,reason=$6,lease_id=NULL,lease_until=NULL,available_at=now()+interval '1 minute',updated_at=now() WHERE org_id=$1 AND inbox_id=$2 AND id=$3 AND lease_id=$4 AND state='triaging' AND lease_until>now()")
            .bind(org).bind(inbox).bind(id).bind(lease).bind(disposition.state()).bind(excerpt(reason,512)).execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }
}

/// A byte ceiling bounds prompts even for languages with poor token compression.
pub fn excerpt(value: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for c in value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
    {
        if out.len() + c.len_utf8() > max_bytes {
            break;
        }
        out.push(if c.is_whitespace() { ' ' } else { c });
    }
    out
}

/// Fixed-window reservations are conservative across crashes. A failed model
/// call still consumes its reservation so outages cannot create retry storms.
pub async fn reserve(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: i64,
    kind: &str,
    limit: i32,
    seconds: i32,
) -> anyhow::Result<bool> {
    let count:Option<i32>=sqlx::query_scalar("INSERT INTO email_rate_limits (org_id,kind,window_start,count) VALUES ($1,$2,to_timestamp(floor(extract(epoch FROM now())/$4::int)*$4::int),1) ON CONFLICT (org_id,kind,window_start) DO UPDATE SET count=email_rate_limits.count+1 WHERE email_rate_limits.count<$3 RETURNING count")
        .bind(org).bind(kind).bind(limit).bind(seconds).fetch_optional(&mut **tx).await?;
    Ok(count.is_some())
}
