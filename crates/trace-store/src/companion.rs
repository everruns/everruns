//! Optional companion orchestration, independent from the primary server.
use crate::{Scope, ShadowReport, TraceStore, catalog};
use anyhow::{Result, ensure};
use serde::Serialize;
use uuid::Uuid;

#[derive(Default, Debug, Serialize)]
pub struct CycleReport {
    pub exported: usize,
    pub errors: usize,
    pub pending_sessions: i64,
    pub lag_sequences: i64,
    pub index_bytes: i64,
    pub payload_bytes: i64,
    pub backfill_pending: i64,
    pub backfill_failed: i64,
    pub unhealthy_checks: i64,
    pub comparisons: Vec<ShadowReport>,
}
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Progress {
    pub pending_sessions: i64,
    pub lag_sequences: i64,
    pub index_bytes: i64,
    pub payload_bytes: i64,
    pub unhealthy_checks: i64,
    pub backfill_pending: i64,
    pub backfill_failed: i64,
}
impl TraceStore {
    /// Explicit all-org mode discovers bounded batches with a durable rotating
    /// cursor. Advancing before work is safe: incomplete targets retain their
    /// own checkpoints and return on the next sweep, including after a restart.
    pub async fn next_org_scopes(&self, max_orgs: usize) -> Result<Vec<Scope>> {
        ensure!((1..=100).contains(&max_orgs), "org batch must be 1..=100");
        let mut tx = self.begin(false).await?;
        let cursor: i64 = sqlx::query_scalar(
            "SELECT observer_org_id FROM trace_store_locations WHERE store_name=$1 FOR UPDATE",
        )
        .bind(&self.name)
        .fetch_one(&mut *tx)
        .await?;
        let sql = "SELECT org_id FROM (SELECT s.org_id FROM sessions s JOIN event_sequences q ON q.session_id=s.id WHERE s.org_id>$1 AND q.next_sequence>1 UNION SELECT r.org_id FROM trace_store_backfills r WHERE r.store_name=$3 AND r.org_id>$1 AND EXISTS(SELECT 1 FROM trace_store_backfill_targets t WHERE t.run_id=r.id AND t.status IN ('pending','failed'))) candidates ORDER BY org_id LIMIT $2";
        let mut ids: Vec<i64> = sqlx::query_scalar(sql)
            .bind(cursor)
            .bind(i64::try_from(max_orgs)?)
            .bind(&self.name)
            .fetch_all(&mut *tx)
            .await?;
        if ids.is_empty() && cursor > 0 {
            ids = sqlx::query_scalar(sql)
                .bind(0_i64)
                .bind(i64::try_from(max_orgs)?)
                .bind(&self.name)
                .fetch_all(&mut *tx)
                .await?;
        }
        let next = ids.last().copied().unwrap_or(0);
        sqlx::query("UPDATE trace_store_locations SET observer_org_id=$2 WHERE store_name=$1")
            .bind(&self.name)
            .bind(next)
            .execute(&mut *tx)
            .await?;
        everruns_durable::commit_transaction(tx).await?;
        ids.into_iter().map(Scope::new).collect()
    }

    /// Resume the latest finite run. Live catch-up handles sessions appended or
    /// created after its snapshot, so restarts need not create additional runs.
    pub async fn resume_backfill(&self, scope: Scope) -> Result<crate::BackfillStatus> {
        let mut tx = catalog::begin(&self.pool).await?;
        let id:Option<Uuid>=sqlx::query_scalar("SELECT id FROM trace_store_backfills WHERE store_name=$1 AND org_id=$2 ORDER BY created_at DESC,id DESC LIMIT 1")
            .bind(&self.name).bind(scope.org_id()).fetch_optional(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        if let Some(id) = id {
            self.backfill_status(scope, id).await
        } else {
            self.start_backfill(scope).await
        }
    }
    pub async fn progress(&self, scope: Scope) -> Result<Progress> {
        self.progress_for(Some(scope.org_id())).await
    }
    /// Trusted operator aggregate; never expose without all-org authority.
    pub async fn progress_all(&self) -> Result<Progress> {
        self.progress_for(None).await
    }
    /// Trusted all-org observer only; gauges must describe the entire store,
    /// not whichever org batch happened to run last. Keep old failures visible.
    pub async fn refresh_all_org_progress(&self, report: &mut CycleReport) -> Result<()> {
        let p = self.progress_all().await?;
        report.pending_sessions = p.pending_sessions;
        report.lag_sequences = p.lag_sequences;
        report.index_bytes = p.index_bytes;
        report.payload_bytes = p.payload_bytes;
        report.unhealthy_checks = p.unhealthy_checks;
        report.backfill_pending = p.backfill_pending;
        report.backfill_failed = p.backfill_failed;
        Ok(())
    }
    async fn progress_for(&self, org_id: Option<i64>) -> Result<Progress> {
        let mut tx = catalog::begin(&self.pool).await?;
        let (pending_sessions,lag_sequences):(i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE q.next_sequence-1>COALESCE(c.exported_sequence,0)),COALESCE(sum(GREATEST(q.next_sequence-1-COALESCE(c.exported_sequence,0),0)),0)::bigint FROM sessions s JOIN event_sequences q ON q.session_id=s.id LEFT JOIN trace_store_sessions c ON c.store_name=$1 AND c.org_id=s.org_id AND c.session_id=s.id WHERE ($2::bigint IS NULL OR s.org_id=$2) AND NOT COALESCE(c.deleted,FALSE)")
            .bind(&self.name).bind(org_id).fetch_one(&mut *tx).await?;
        let (index_bytes,payload_bytes):(i64,i64)=sqlx::query_as("SELECT COALESCE(sum(m.index_bytes),0)::bigint,COALESCE(sum(m.payload_bytes),0)::bigint FROM trace_store_segments m JOIN sessions s ON s.id=m.session_id AND s.org_id=m.org_id JOIN trace_store_sessions c ON c.store_name=m.store_name AND c.org_id=m.org_id AND c.session_id=m.session_id WHERE m.store_name=$1 AND ($2::bigint IS NULL OR m.org_id=$2) AND NOT m.retired AND NOT c.deleted")
            .bind(&self.name).bind(org_id).fetch_one(&mut *tx).await?;
        let unhealthy_checks:i64=sqlx::query_scalar("SELECT count(*) FROM trace_store_shadow_checks p JOIN sessions s ON s.id=p.session_id AND s.org_id=p.org_id WHERE p.store_name=$1 AND ($2::bigint IS NULL OR p.org_id=$2) AND p.report->>'outcome' IN ('mismatch','error','missing_source') AND NOT EXISTS(SELECT 1 FROM trace_store_sessions c WHERE c.store_name=p.store_name AND c.org_id=p.org_id AND c.session_id=p.session_id AND c.deleted)")
            .bind(&self.name).bind(org_id).fetch_one(&mut *tx).await?;
        let (backfill_pending, backfill_failed): (i64,i64) = sqlx::query_as("WITH latest AS (SELECT DISTINCT ON(org_id) id FROM trace_store_backfills WHERE store_name=$1 AND ($2::bigint IS NULL OR org_id=$2) ORDER BY org_id,created_at DESC,id DESC) SELECT count(*) FILTER(WHERE t.status='pending'),count(*) FILTER(WHERE t.status='failed') FROM trace_store_backfill_targets t JOIN latest r ON r.id=t.run_id")
            .bind(&self.name).bind(org_id).fetch_one(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        Ok(Progress {
            pending_sessions,
            lag_sequences,
            index_bytes,
            payload_bytes,
            unhealthy_checks,
            backfill_pending,
            backfill_failed,
        })
    }
    /// Errors are counted with fixed categories; raw driver errors never enter
    /// reports or metrics. Other orgs and the primary service remain independent.
    pub async fn parallel_cycle(
        &self,
        scopes: &[Scope],
        max_sessions: usize,
    ) -> Result<CycleReport> {
        ensure!(
            !scopes.is_empty() && scopes.len() <= 100,
            "explicit org scope required (1..=100)"
        );
        ensure!(
            (1..=100).contains(&max_sessions),
            "session batch must be 1..=100"
        );
        let mut report = CycleReport::default();
        for &scope in scopes {
            match self.resume_backfill(scope).await {
                Ok(run) => {
                    let status = if run.complete() {
                        Ok(run)
                    } else {
                        self.advance_backfill(scope, run.run_id, max_sessions).await
                    };
                    match status {
                        Ok(status) => {
                            report.exported += status.exported_this_pass;
                            if status.complete() {
                                match self.sync_once(scope, max_sessions).await {
                                    Ok(n) => report.exported += n,
                                    Err(_) => report.errors += 1,
                                }
                            }
                        }
                        Err(_) => report.errors += 1,
                    }
                }
                Err(_) => report.errors += 1,
            }
            match self.shadow_once(scope, max_sessions).await {
                Ok(checks) => report.comparisons.extend(checks),
                Err(_) => report.errors += 1,
            }
            match self.progress(scope).await {
                Ok(p) => {
                    report.backfill_pending += p.backfill_pending;
                    report.backfill_failed += p.backfill_failed;
                    report.pending_sessions += p.pending_sessions;
                    report.lag_sequences += p.lag_sequences;
                    report.index_bytes += p.index_bytes;
                    report.payload_bytes += p.payload_bytes;
                    report.unhealthy_checks += p.unhealthy_checks;
                }
                Err(_) => report.errors += 1,
            }
        }
        Ok(report)
    }
}
