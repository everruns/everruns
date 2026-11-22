use super::{CONTINUITY_KEY, EXECUTION_KEY, policy};
use crate::storage::{CreateTraceScoreRow, ObserverRow, SessionRow, StorageBackend};
use chrono::Utc;
use everruns_contracts::typed_id::TraceScoreId;
use everruns_core::Event;
use std::sync::Arc;

/// Matching reads metadata only. Content is fetched after worker consent checks.
pub async fn enqueue_event(db: &Arc<StorageBackend>, event: &Event) -> anyhow::Result<u64> {
    if !super::EVENT_TYPES.contains(&event.event_type.as_str()) {
        return Ok(0);
    }
    let Some(lineage) = db.session_lineage(event.session_id).await? else {
        return Ok(0);
    };
    let policy = policy(db, lineage.org_id).await?;
    if !policy.enabled {
        return Ok(0);
    }
    let Some(session) = db.get_session_unscoped(event.session_id).await? else {
        return Ok(0);
    };
    if !policy.allows(&session.tags) {
        return Ok(0);
    }
    let Some(observer) = db
        .system_compaction_observer(lineage.org_id)
        .await?
        .filter(|row| row.status == "active")
    else {
        return Ok(0);
    };
    let terminal_turn = match &event.data {
        everruns_core::events::EventData::TurnCompleted(data) => Some(data.turn_id),
        everruns_core::events::EventData::TurnFailed(data) => Some(data.turn_id),
        _ => None,
    };
    if let Some(turn) = terminal_turn {
        return match event.sequence {
            Some(sequence) => {
                db.activate_monitoring_continuations(
                    session.org_id,
                    event.session_id,
                    sequence,
                    turn,
                )
                .await
            }
            None => Ok(0),
        };
    }
    if event.ts < observer.updated_at {
        return Ok(0);
    }
    let Some(observation) = everruns_core::compaction_observation::observe_compaction(&event.data)
    else {
        return Ok(0);
    };
    let mut inputs = vec![row(event, &observer, &session, EXECUTION_KEY, None)];
    if observation.continuity_applicable {
        // The compaction turn activates this once. A turn that never ends
        // produces an explicit unassessed result after ten minutes.
        inputs.push(row(
            event,
            &observer,
            &session,
            CONTINUITY_KEY,
            Some(Utc::now() + chrono::Duration::minutes(10)),
        ));
    }
    db.enqueue_trace_scores(session.org_id, &inputs).await
}

fn row(
    event: &Event,
    observer: &ObserverRow,
    session: &SessionRow,
    key: &str,
    not_before: Option<chrono::DateTime<Utc>>,
) -> CreateTraceScoreRow {
    let observation = everruns_core::compaction_observation::observe_compaction(&event.data);
    CreateTraceScoreRow {
        evidence_ref: Some(
            serde_json::json!({"event_id": event.id, "event_sequence": event.sequence, "turn_id": event.context.turn_id, "observation": observation, "consent_version": observer.updated_at}),
        ),
        public_id: TraceScoreId::new().to_string(),
        observer_id: observer.id,
        scorer_key: key.into(),
        session_id: event.session_id.uuid(),
        turn_id: event.id.to_string(),
        agent_id: session.agent_id.map(|id| id.uuid()),
        harness_id: session.harness_id.map(|id| id.uuid()),
        scope: "compaction".into(),
        not_before,
    }
}
