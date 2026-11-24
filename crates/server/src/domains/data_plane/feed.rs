//! `POST /dp/v1/feed`: what the data plane reports back (cp-dp-protocol.md,
//! "Feed").
//!
//! Decisions:
//! - Each source's feed arrives in order, so a record at or before the
//!   stored position is a repeat and is skipped without a lookup. A
//!   restored control plane has lost positions; then the second guards
//!   hold: usage is journaled once per `attempt_id` (the journal's unique
//!   source index), and a session summary never replaces a newer version.
//! - The acknowledgement names the last record stored. A record that fails
//!   to store ends the batch there, so the data plane sends it again.
//! - A batch from an older incarnation of a shard than one already heard
//!   (a node of a shard that was rebuilt, still running somewhere) is
//!   stored, and flagged: its usage was really spent and is journaled once
//!   per attempt, and its session summaries carry versions below the
//!   rebuilt sessions' (`control::turn_finished`), so they change nothing.
//!   Refusing it would lose that spend.
//! - Monitoring (knowledge/operations/monitoring.md): a shard's minute
//!   `Health` report carries no organization and is stored before the
//!   organization check; finished turns and model calls are tallied per
//!   batch and added to the minute history once the batch's records are
//!   stored. A failure there is logged and never refuses the batch.
//! - A session event the org's policy asked for goes to the control
//!   plane's listeners (`event_tap.rs`, step 11.5), and to the org's
//!   warehouse files first when it exports them (`warehouse.rs`).
//! - A v2 task's transition goes to its webhooks (`task_webhooks.rs`).
//! - Feed lag (record time to stored) is logged per batch at debug level.
//!   The gate measures it in the shard suite (`FakeCp::feed_lag`); a metric
//!   comes with step 10's per-shard dashboards.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use everruns_contracts::dataplane::{FeedAck, FeedBatch, FeedRecord, Record};
use everruns_core::EventListener;

use super::{ControlPlane, org_internal};
use crate::domains::budgets::{BudgetService, DataPlaneUsage};
use crate::storage::{DpSessionSummary, ShardBatchCounters, ShardMinuteReport};

/// Store `batch`; the position through which it is stored.
pub async fn ingest(
    cp: &ControlPlane,
    budgets: &BudgetService,
    listeners: &[Arc<dyn EventListener>],
    batch: FeedBatch,
) -> anyhow::Result<FeedAck> {
    let incarnation = batch.incarnation as i64;
    if let Some(newest) = cp.db.dp_feed_newest_incarnation(&batch.source).await?
        && newest > incarnation
    {
        tracing::warn!(
            source = %batch.source,
            incarnation,
            newest,
            "feed from a shard incarnation that was rebuilt; storing its usage only once"
        );
    }
    let mut through = cp
        .db
        .dp_feed_position(&batch.source, incarnation)
        .await?
        .map_or((0, 0), |(i, o)| (i as u64, o as u32));
    let start = through;
    super::warehouse::export(cp, &batch, start).await?;
    let mut counters = ShardBatchCounters::default();
    for record in &batch.records {
        if (record.index, record.ordinal) <= start {
            continue;
        }
        if let Err(error) = store(cp, budgets, listeners, &batch, record, &mut counters).await {
            tracing::warn!(source = %batch.source, index = record.index, %error, "feed record not stored");
            break;
        }
        through = (record.index, record.ordinal);
    }
    if let Err(error) = cp
        .db
        .monitoring_add_shard_counters(&batch.source, &batch.data_plane, &counters)
        .await
    {
        tracing::warn!(source = %batch.source, %error, "feed counters not added to monitoring");
    }
    if through > start {
        cp.db
            .dp_set_feed_position(
                &batch.source,
                incarnation,
                (through.0 as i64, through.1 as i64),
            )
            .await?;
    }
    if let Some(last) = batch.records.last() {
        let lag = Utc::now().timestamp_millis() - last.at_unix_ms;
        tracing::debug!(source = %batch.source, lag_ms = lag, "feed stored");
    }
    Ok(FeedAck {
        source: batch.source,
        incarnation: batch.incarnation,
        stored_through: through,
    })
}

fn at(ms: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Utc::now)
}

async fn store(
    cp: &ControlPlane,
    budgets: &BudgetService,
    listeners: &[Arc<dyn EventListener>],
    batch: &FeedBatch,
    record: &FeedRecord,
    counters: &mut ShardBatchCounters,
) -> anyhow::Result<()> {
    if let Record::Health { .. } = &record.record {
        if let Err(error) = store_health(cp, batch, record).await {
            tracing::warn!(source = %batch.source, %error, "shard minute report not stored");
        }
        return Ok(());
    }
    // A record of an org this control plane does not know is dropped: the
    // data plane learns of the org from this control plane only.
    let Some(org_id) = org_internal(&record.org_id) else {
        return Ok(());
    };
    if cp.db.dp_org(org_id).await?.is_none() {
        return Ok(());
    }
    match &record.record {
        Record::Usage {
            session_id,
            attempt_id,
            agent_id,
            provider,
            model,
            input_tokens,
            output_tokens,
            cached_tokens,
            cost_micros,
            kind,
            ..
        } => {
            let new = budgets
                .meter_data_plane_usage(DataPlaneUsage {
                    org_id,
                    attempt_id,
                    session_id,
                    agent_id,
                    provider,
                    model,
                    kind,
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    cached_tokens: *cached_tokens,
                    cost_micros: *cost_micros,
                })
                .await?;
            if new {
                cp.db.dp_add_feed_cost(org_id, *cost_micros).await?;
                let c = counters.entry(org_id, at(record.at_unix_ms));
                c.model_calls += 1;
                c.input_tokens += *input_tokens as i64;
                c.output_tokens += *output_tokens as i64;
                c.cost_usd += *cost_micros as f64 / 1_000_000.0;
            }
        }
        Record::Session {
            session_id,
            session_version,
            agent_id,
            title,
            status,
            created_unix_ms,
            last_activity_unix_ms,
            turns,
            cost_micros,
            last_turn,
        } => {
            cp.db
                .dp_upsert_session_summary(DpSessionSummary {
                    session_id,
                    org_id,
                    data_plane_id: &batch.data_plane,
                    agent_id,
                    session_version: *session_version as i64,
                    title,
                    status,
                    turns: *turns as i64,
                    cost_micros: *cost_micros,
                    created_at: at(*created_unix_ms),
                    last_activity_at: at(*last_activity_unix_ms),
                })
                .await?;
            let c = counters.entry(org_id, at(*last_activity_unix_ms));
            match last_turn.as_str() {
                "completed" => c.turns_completed += 1,
                "failed" => c.turns_failed += 1,
                "cancelled" => c.turns_cancelled += 1,
                _ => {}
            }
            // The session's own record (created by the control plane's
            // `POST /v1/platform/sessions`) shows the same state in every list.
            if let Ok(id) = session_id.parse::<everruns_contracts::typed_id::SessionId>() {
                let outcome = match (last_turn.as_str(), status.as_str()) {
                    ("completed" | "failed" | "cancelled", _) => last_turn.as_str(),
                    (_, "failed") => "failed",
                    _ => "completed",
                };
                cp.db
                    .dp_apply_session_summary(
                        org_id,
                        id.uuid(),
                        outcome,
                        at(*last_activity_unix_ms),
                    )
                    .await?;
            }
        }
        Record::SecretRefreshed {
            secret_id,
            revision,
            ciphertext,
            nonce,
            data_key_id,
        } => {
            // A token the data plane refreshed: kept unless a newer
            // revision is held (the revision guard in `dp_put_item`).
            let sealed = everruns_contracts::dataplane::SealedSecret {
                secret_id: secret_id.clone(),
                revision: *revision,
                ciphertext: ciphertext.clone(),
                nonce: nonce.clone(),
                data_key_id: data_key_id.clone(),
            };
            super::mcp::refreshed(cp, org_id, &sealed).await?;
            let change = everruns_contracts::dataplane::ConfigChange::Secret(sealed).to_value();
            cp.db
                .dp_put_item(org_id, "secret", secret_id, *revision as i64, Some(&change))
                .await?;
        }
        Record::ConfigApplied { version } => {
            tracing::debug!(org_id, version, "data plane applied config");
        }
        Record::OrgEvent { kind, detail } => {
            cp.db
                .dp_insert_org_event(org_id, kind, detail, &batch.source, at(record.at_unix_ms))
                .await?;
        }
        Record::Event { session_id, event } => {
            super::event_tap::deliver(cp, listeners, org_id, session_id, event).await?;
        }
        Record::TaskTransition {
            session_id,
            transition,
            task,
            targets,
        } => {
            super::task_webhooks::deliver(cp, org_id, session_id, transition, task, targets)
                .await?;
        }
        Record::SessionResource {
            session_id,
            resource_id,
            resource,
        } => {
            super::session_records::resource(cp, org_id, session_id, resource_id, resource).await?;
        }
        Record::Lease { session_id, lease } => {
            super::session_records::lease(cp, org_id, session_id, lease).await?;
        }
        Record::Subagent { session_id, origin } => {
            super::subagents::record(cp, org_id, session_id, origin).await?;
        }
        Record::Health { .. } | Record::Unknown => {}
    }
    Ok(())
}

/// A shard's minute report: platform-wide, with per-organization population
/// for organizations this control plane knows.
async fn store_health(
    cp: &ControlPlane,
    batch: &FeedBatch,
    record: &FeedRecord,
) -> anyhow::Result<()> {
    let Record::Health {
        sessions,
        running_turns,
        minute_unix_ms,
        queued,
        resumed_turns,
        forwarded,
        term,
        leader,
        nodes,
        nodes_answering,
        rss_peak_bytes,
        memory_limit_bytes,
        cpu_peak_cores,
        cpu_limit_cores,
        admit_p99_ms,
        ship_lag_p99_ms,
        orgs,
        ..
    } = &record.record
    else {
        return Ok(());
    };
    // A report from before this field existed has no minute: skip it.
    if *minute_unix_ms <= 0 {
        return Ok(());
    }
    let mut known = Vec::with_capacity(orgs.len());
    for o in orgs {
        if let Some(id) = org_internal(&o.org_id)
            && cp.db.dp_org(id).await?.is_some()
        {
            known.push((
                id,
                i64::from(o.sessions),
                i64::from(o.running_turns),
                i64::from(o.queued),
                i64::from(o.agents_working),
            ));
        }
    }
    let saturate = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    let stored = cp
        .db
        .monitoring_record_shard_report(&ShardMinuteReport {
            shard: batch.source.clone(),
            data_plane: batch.data_plane.clone(),
            minute: at(*minute_unix_ms),
            leader: leader.clone(),
            nodes: *nodes as i32,
            nodes_answering: *nodes_answering as i32,
            sessions: i64::from(*sessions),
            running: i64::from(*running_turns),
            queued: i64::from(*queued),
            resumed: i64::from(*resumed_turns),
            rss_peak_bytes: saturate(*rss_peak_bytes),
            memory_limit_bytes: saturate(*memory_limit_bytes),
            cpu_peak_cores: *cpu_peak_cores,
            cpu_limit_cores: *cpu_limit_cores,
            admit_p99_ms: *admit_p99_ms as i32,
            ship_lag_p99_ms: *ship_lag_p99_ms as i32,
            raft_term: saturate(*term),
            forwarded: saturate(*forwarded),
            orgs: known,
        })
        .await?;
    if !stored {
        tracing::debug!(source = %batch.source, "shard minute already reported");
    }
    Ok(())
}
