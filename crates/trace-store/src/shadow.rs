//! Synthetic shadow workload. Both stores answer the same sequence window; lag
//! is recorded separately from parity. Product HTTP responses never depend on it.
use crate::{EventQuery, EventRecord, EventSummary, Scope, TraceStore, catalog};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShadowOutcome {
    Match,
    Mismatch,
    Lagging,
    Skipped,
    Error,
    MissingSource,
}
impl ShadowOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::Mismatch => "mismatch",
            Self::Lagging => "lagging",
            Self::Skipped => "skipped",
            Self::Error => "error",
            Self::MissingSource => "missing_source",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShadowReport {
    pub session_id: Uuid,
    pub outcome: ShadowOutcome,
    pub source_head: i32,
    pub exported_head: i32,
    pub after_sequence: i32,
    pub through_sequence: i32,
    pub window_through_sequence: i32,
    pub next_sequence: i32,
    pub source_rows: usize,
    pub trace_rows: usize,
    pub summary_matches: Option<bool>,
    pub payload_matches: Option<bool>,
    pub source_seconds: f64,
    pub trace_seconds: f64,
    pub source_payload_seconds: f64,
    pub trace_payload_seconds: f64,
    pub segments_scanned: usize,
}
impl ShadowReport {
    fn empty(session_id: Uuid, outcome: ShadowOutcome) -> Self {
        Self {
            session_id,
            outcome,
            source_head: 0,
            exported_head: 0,
            after_sequence: 0,
            through_sequence: 0,
            window_through_sequence: 0,
            next_sequence: 0,
            source_rows: 0,
            trace_rows: 0,
            summary_matches: None,
            payload_matches: None,
            source_seconds: 0.0,
            trace_seconds: 0.0,
            source_payload_seconds: 0.0,
            trace_payload_seconds: 0.0,
            segments_scanned: 0,
        }
    }
}
impl TraceStore {
    /// Fair round-robin sampling of actual retained sessions, not captured HTTP
    /// traffic. Latest evidence is durable and contains no copied payload/diff.
    pub async fn shadow_once(
        &self,
        scope: Scope,
        max_sessions: usize,
    ) -> Result<Vec<ShadowReport>> {
        ensure!(
            (1..=100).contains(&max_sessions),
            "shadow session batch must be 1..=100"
        );
        let mut tx = catalog::begin(&self.pool).await?;
        let sessions:Vec<Uuid>=sqlx::query_scalar("SELECT s.id FROM sessions s JOIN event_sequences q ON q.session_id=s.id LEFT JOIN trace_store_shadow_checks c ON c.store_name=$1 AND c.org_id=s.org_id AND c.session_id=s.id WHERE s.org_id=$2 AND q.next_sequence>1 ORDER BY c.checked_at ASC NULLS FIRST,s.id LIMIT $3")
            .bind(&self.name).bind(scope.org_id()).bind(i64::try_from(max_sessions)?).fetch_all(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        let mut reports = Vec::new();
        for session in sessions {
            reports.push(self.shadow_session(scope, session, 100).await?);
        }
        Ok(reports)
    }

    pub async fn shadow_session(
        &self,
        scope: Scope,
        session: Uuid,
        limit: usize,
    ) -> Result<ShadowReport> {
        ensure!(
            (1..=1000).contains(&limit),
            "shadow row limit must be 1..=1000"
        );
        let mut tx = self.begin(false).await?;
        let heads:Option<(i32,i32,bool,i32)>=sqlx::query_as("SELECT q.next_sequence-1,COALESCE(c.exported_sequence,0),COALESCE(c.deleted,FALSE),COALESCE(p.next_sequence,0) FROM sessions s JOIN event_sequences q ON q.session_id=s.id LEFT JOIN trace_store_sessions c ON c.store_name=$1 AND c.org_id=s.org_id AND c.session_id=s.id LEFT JOIN trace_store_shadow_checks p ON p.store_name=$1 AND p.org_id=s.org_id AND p.session_id=s.id WHERE s.org_id=$2 AND s.id=$3 FOR KEY SHARE OF s")
            .bind(&self.name).bind(scope.org_id()).bind(session).fetch_optional(&mut *tx).await?;
        let Some((source, exported, deleted, cursor)) = heads else {
            everruns_durable::commit_transaction(tx).await?;
            return Ok(ShadowReport::empty(session, ShadowOutcome::Skipped));
        };
        let shared = source.min(exported);
        let after = if cursor < shared { cursor } else { 0 };
        let mut report = ShadowReport::empty(
            session,
            if deleted {
                ShadowOutcome::Skipped
            } else {
                ShadowOutcome::Lagging
            },
        );
        report.source_head = source;
        report.exported_head = exported;
        report.after_sequence = after;
        report.through_sequence = shared;
        everruns_durable::commit_transaction(tx).await?;
        if !deleted && shared > 0 {
            match tokio::time::timeout(
                self.limits.query_timeout.saturating_mul(3),
                self.compare_window(scope, &mut report, limit),
            )
            .await
            {
                Ok(Ok(())) => {}
                _ => {
                    report.outcome = ShadowOutcome::Error;
                    report.next_sequence = after;
                }
            }
        }
        let mut tx = catalog::begin(&self.pool).await?;
        let visible:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions s WHERE s.id=$1 AND s.org_id=$2 AND NOT EXISTS(SELECT 1 FROM trace_store_sessions c WHERE c.store_name=$3 AND c.org_id=s.org_id AND c.session_id=s.id AND c.deleted))")
            .bind(session).bind(scope.org_id()).bind(&self.name).fetch_one(&mut *tx).await?;
        if !visible {
            report.outcome = ShadowOutcome::Skipped;
            report.summary_matches = None;
            report.payload_matches = None;
            report.next_sequence = after;
        }
        sqlx::query("INSERT INTO trace_store_shadow_checks(store_name,org_id,session_id,next_sequence,report) SELECT $1,$2,s.id,$4,$5 FROM sessions s WHERE s.org_id=$2 AND s.id=$3 ON CONFLICT(store_name,org_id,session_id) DO UPDATE SET next_sequence=EXCLUDED.next_sequence,report=EXCLUDED.report,checked_at=now()")
            .bind(&self.name).bind(scope.org_id()).bind(session).bind(report.next_sequence).bind(serde_json::to_value(&report)?).execute(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        Ok(report)
    }

    async fn compare_window(
        &self,
        scope: Scope,
        report: &mut ShadowReport,
        limit: usize,
    ) -> Result<()> {
        let source_start = Instant::now();
        let mut tx = catalog::begin(&self.pool).await?;
        catalog::lock_source(&mut tx, scope, report.session_id).await?;
        let rows:Vec<EventSummary>=sqlx::query_as("SELECT DISTINCT ON(sequence) id AS event_id,session_id,sequence,event_type,ts AS occurred_at,CASE WHEN octet_length(context->>'turn_id')<=128 THEN context->>'turn_id' END AS turn_id,CASE WHEN octet_length(context->>'exec_id')<=128 THEN context->>'exec_id' END AS exec_id,CASE WHEN octet_length(context->>'parent_exec_id')<=128 THEN context->>'parent_exec_id' END AS parent_exec_id FROM ((SELECT id,session_id,sequence,event_type,ts,context,0 AS priority FROM events WHERE session_id=$1 AND sequence>$2 AND sequence<=$3 ORDER BY sequence LIMIT $4) UNION ALL (SELECT id,session_id,sequence,event_type,ts,context,1 AS priority FROM archived_events WHERE session_id=$1 AND sequence>$2 AND sequence<=$3 ORDER BY sequence LIMIT $4)) retained ORDER BY sequence,priority LIMIT $4")
            .bind(report.session_id).bind(report.after_sequence).bind(report.through_sequence).bind(i64::try_from(limit+1)?).fetch_all(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        report.source_seconds = source_start.elapsed().as_secs_f64();
        let source_has_more = rows.len() > limit;
        // The extra source row bounds the physical window without changing the
        // page's has_more contract. A million-event session needs only the
        // segments covering this page, not every segment through its head.
        let window_through = if source_has_more {
            rows.last().context("empty window")?.sequence
        } else {
            report.through_sequence
        };
        report.window_through_sequence = window_through;
        let rows = rows.into_iter().take(limit).collect::<Vec<_>>();
        let trace_start = Instant::now();
        let page = self
            .query(
                scope,
                &EventQuery {
                    session_id: Some(report.session_id),
                    after_sequence: Some(report.after_sequence),
                    through_sequence: Some(window_through),
                    limit,
                    ..Default::default()
                },
            )
            .await?;
        report.trace_seconds = trace_start.elapsed().as_secs_f64();
        report.source_rows = rows.len();
        report.trace_rows = page.events.len();
        report.segments_scanned = page.segments_scanned;
        report.summary_matches = Some(rows == page.events && source_has_more == page.has_more);
        report.next_sequence = if source_has_more {
            rows.last().context("empty continuation")?.sequence
        } else {
            report.through_sequence
        };
        if let Some(sample) = rows.first() {
            let source_start = Instant::now();
            let mut tx = catalog::begin(&self.pool).await?;
            catalog::lock_source(&mut tx, scope, report.session_id).await?;
            let record:Option<EventRecord>=sqlx::query_as("SELECT id,session_id,sequence,event_type,ts,created_at,context,data,metadata,tags FROM (SELECT id,session_id,sequence,event_type,ts,created_at,context,data,metadata,tags,0 AS priority FROM events WHERE session_id=$1 AND id=$2 UNION ALL SELECT id,session_id,sequence,event_type,ts,created_at,context,data,metadata,tags,1 AS priority FROM archived_events WHERE session_id=$1 AND id=$2) retained ORDER BY priority LIMIT 1")
                .bind(report.session_id).bind(sample.event_id).fetch_optional(&mut *tx).await?;
            everruns_durable::commit_transaction(tx).await?;
            report.source_payload_seconds = source_start.elapsed().as_secs_f64();
            let trace_start = Instant::now();
            let copied = self
                .read_event_at(scope, report.session_id, sample.event_id, sample.sequence)
                .await?;
            report.trace_payload_seconds = trace_start.elapsed().as_secs_f64();
            report.payload_matches = Some(record.is_some() && record == copied);
        }
        report.outcome = if report.source_rows == 0 && report.trace_rows == 0 {
            ShadowOutcome::MissingSource
        } else if report.summary_matches == Some(false) || report.payload_matches == Some(false) {
            ShadowOutcome::Mismatch
        } else if report.exported_head < report.source_head {
            ShadowOutcome::Lagging
        } else {
            ShadowOutcome::Match
        };
        Ok(())
    }
}
