//! Fixed backfill targets make completion finite even while sessions append.
use crate::{Scope, TraceStore, catalog};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BackfillStatus {
    pub run_id: Uuid,
    pub total: i64,
    pub pending: i64,
    pub copied: i64,
    pub skipped: i64,
    pub failed: i64,
    #[sqlx(skip)]
    pub exported_this_pass: usize,
}
impl BackfillStatus {
    pub fn complete(&self) -> bool {
        self.pending == 0 && self.failed == 0
    }
}
impl TraceStore {
    pub async fn start_backfill(&self, scope: Scope) -> Result<BackfillStatus> {
        let mut tx = self.begin(false).await?;
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO trace_store_backfills(id,store_name,org_id) VALUES($1,$2,$3)")
            .bind(id)
            .bind(&self.name)
            .bind(scope.org_id())
            .execute(&mut *tx)
            .await?;
        let added=sqlx::query("INSERT INTO trace_store_backfill_targets(run_id,session_id,target_sequence,status) SELECT $1,s.id,q.next_sequence-1,CASE WHEN c.deleted THEN 'skipped' WHEN c.exported_sequence>=q.next_sequence-1 THEN 'copied' ELSE 'pending' END FROM sessions s JOIN event_sequences q ON q.session_id=s.id LEFT JOIN trace_store_sessions c ON c.session_id=s.id AND c.org_id=s.org_id AND c.store_name=$2 WHERE s.org_id=$3 AND q.next_sequence>1 ORDER BY s.id LIMIT 100001")
            .bind(id).bind(&self.name).bind(scope.org_id()).execute(&mut *tx).await?.rows_affected();
        ensure!(added <= 100_000, "backfill target budget exceeded");
        everruns_durable::commit_transaction(tx).await?;
        self.backfill_status(scope, id).await
    }

    pub async fn backfill_status(&self, scope: Scope, run: Uuid) -> Result<BackfillStatus> {
        let mut tx = catalog::begin(&self.pool).await?;
        let status=sqlx::query_as("SELECT r.id AS run_id,count(t.session_id) AS total,count(*) FILTER(WHERE t.status='pending') AS pending,count(*) FILTER(WHERE t.status='copied') AS copied,count(*) FILTER(WHERE t.status='skipped') AS skipped,count(*) FILTER(WHERE t.status='failed') AS failed FROM trace_store_backfills r LEFT JOIN trace_store_backfill_targets t ON t.run_id=r.id WHERE r.id=$1 AND r.store_name=$2 AND r.org_id=$3 GROUP BY r.id")
            .bind(run).bind(&self.name).bind(scope.org_id()).fetch_optional(&mut *tx).await?.context("backfill not found in organization")?;
        everruns_durable::commit_transaction(tx).await?;
        Ok(status)
    }

    /// One session batch per target, with resumable checkpoints. A failed target
    /// is retried on a later pass and never prevents unrelated targets progressing.
    pub async fn advance_backfill(
        &self,
        scope: Scope,
        run: Uuid,
        max_sessions: usize,
    ) -> Result<BackfillStatus> {
        ensure!(
            (1..=100).contains(&max_sessions),
            "session batch must be 1..=100"
        );
        self.backfill_status(scope, run).await?;
        let mut coordination = catalog::begin(&self.pool).await?;
        let claimed: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,37023))")
                .bind(run.to_string())
                .fetch_one(&mut *coordination)
                .await?;
        let mut exported_this_pass = 0;
        if claimed {
            let targets:Vec<(Uuid,i32)>=sqlx::query_as("SELECT session_id,target_sequence FROM trace_store_backfill_targets WHERE run_id=$1 AND status IN ('pending','failed') ORDER BY updated_at,session_id LIMIT $2")
                .bind(run).bind(i64::try_from(max_sessions)?).fetch_all(&mut *coordination).await?;
            for (session, target) in targets {
                let live:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions s WHERE s.id=$1 AND s.org_id=$2 AND NOT EXISTS(SELECT 1 FROM trace_store_sessions c WHERE c.store_name=$3 AND c.org_id=s.org_id AND c.session_id=s.id AND c.deleted))")
                    .bind(session).bind(scope.org_id()).bind(&self.name).fetch_one(&mut *coordination).await?;
                let status = if !live {
                    "skipped"
                } else {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(60),
                        self.export_through(scope, session, Some(target)),
                    )
                    .await
                    {
                        Ok(Ok(n)) => {
                            exported_this_pass += n;
                            let head:Option<i32>=sqlx::query_scalar("SELECT exported_sequence FROM trace_store_sessions WHERE store_name=$1 AND org_id=$2 AND session_id=$3")
                                .bind(&self.name).bind(scope.org_id()).bind(session).fetch_optional(&mut *coordination).await?;
                            if head.is_some_and(|head| head >= target) {
                                "copied"
                            } else {
                                "pending"
                            }
                        }
                        _ => "failed",
                    }
                };
                // Commit progress independently from the coordination lease. A
                // process crash cannot erase previously completed target updates.
                sqlx::query("UPDATE trace_store_backfill_targets SET status=$3,attempts=attempts+1,error_code=$4,updated_at=now() WHERE run_id=$1 AND session_id=$2")
                    .bind(run).bind(session).bind(status).bind(if status=="failed" {Some("export_failed")} else {None})
                    .execute(&self.pool).await?;
            }
        }
        everruns_durable::commit_transaction(coordination).await?;
        let mut status = self.backfill_status(scope, run).await?;
        status.exported_this_pass = exported_this_pass;
        Ok(status)
    }
}
