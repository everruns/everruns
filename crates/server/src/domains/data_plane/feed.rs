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
//! - Feed lag (record time to stored) is logged per batch at debug level.
//!   The gate measures it in the cell suite (`FakeCp::feed_lag`); a metric
//!   comes with step 10's per-cell dashboards.

use chrono::{TimeZone, Utc};
use everruns_contracts::dataplane::{FeedAck, FeedBatch, FeedRecord, Record};

use super::{ControlPlane, org_internal};
use crate::domains::budgets::{BudgetService, DataPlaneUsage};
use crate::storage::DpSessionSummary;

/// Store `batch`; the position through which it is stored.
pub async fn ingest(
    cp: &ControlPlane,
    budgets: &BudgetService,
    batch: FeedBatch,
) -> anyhow::Result<FeedAck> {
    let incarnation = batch.incarnation as i64;
    let mut through = cp
        .db
        .dp_feed_position(&batch.source, incarnation)
        .await?
        .map_or((0, 0), |(i, o)| (i as u64, o as u32));
    let start = through;
    for record in &batch.records {
        if (record.index, record.ordinal) <= start {
            continue;
        }
        if let Err(error) = store(cp, budgets, &batch, record).await {
            tracing::warn!(source = %batch.source, index = record.index, %error, "feed record not stored");
            break;
        }
        through = (record.index, record.ordinal);
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
    batch: &FeedBatch,
    record: &FeedRecord,
) -> anyhow::Result<()> {
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
        Record::Health { .. } | Record::Unknown => {}
    }
    Ok(())
}
