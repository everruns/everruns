// Background sweep: time out sessions stuck in `waiting_for_tool_results`.
// Decision: periodic sweep (every 30s) rather than per-session timers; survives restarts.
// Decision: timeout is 5 minutes per knowledge/execution/client-side-tools.md, configurable via env var.

use crate::services::EventService;
use crate::services::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::storage::StorageBackend;
use crate::storage::models::{ClaimWaitingTurnResult, WaitingTurnResolutionPlan};
use chrono::Utc;
use everruns_core::events::{
    EventContext, EventData, EventRequest, ToolCompletedData, deserialize_event_data,
};
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use everruns_worker::AgentRunner;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Default timeout for waiting_for_tool_results sessions (5 minutes).
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// How often the sweep runs (30 seconds).
const SWEEP_INTERVAL_SECS: u64 = 30;

/// Spawn a background task that periodically times out stale
/// `waiting_for_tool_results` sessions.
pub fn spawn_tool_result_timeout_sweep(
    db: Arc<StorageBackend>,
    runner: Arc<dyn AgentRunner>,
    event_delivery: crate::event_delivery::EventDelivery,
) -> JoinHandle<()> {
    let timeout_secs = std::env::var("TOOL_RESULT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    tokio::spawn(async move {
        tracing::info!(
            timeout_secs,
            sweep_interval_secs = SWEEP_INTERVAL_SECS,
            "Tool result timeout sweep started"
        );

        let event_service = EventService::new(db.clone(), event_delivery);

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(SWEEP_INTERVAL_SECS)).await;

            if let Err(e) =
                sweep_timed_out_sessions(&db, &runner, &event_service, timeout_secs).await
            {
                tracing::warn!(error = %e, "Tool result timeout sweep error");
            }
        }
    })
}

async fn sweep_timed_out_sessions(
    db: &Arc<StorageBackend>,
    runner: &Arc<dyn AgentRunner>,
    event_service: &EventService,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    let duration = chrono::Duration::try_seconds(timeout_secs.min(i64::MAX as u64) as i64)
        .unwrap_or(chrono::Duration::seconds(DEFAULT_TIMEOUT_SECS as i64));
    let cutoff = Utc::now() - duration;

    let timed_out = db.list_sessions_waiting_tool_results_before(cutoff).await?;

    if timed_out.is_empty() {
        return Ok(());
    }

    tracing::info!(
        count = timed_out.len(),
        "Found timed-out waiting_for_tool_results sessions"
    );

    for (session_id, org_id) in timed_out {
        if let Err(e) = timeout_session(db, event_service, runner, session_id, org_id).await {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to timeout session"
            );
        }
    }

    Ok(())
}

async fn timeout_session(
    db: &Arc<StorageBackend>,
    event_service: &EventService,
    runner: &Arc<dyn AgentRunner>,
    session_id: SessionId,
    org_id: i64,
) -> anyhow::Result<()> {
    let tool_call_ids = find_pending_tool_call_ids(db, session_id).await?;
    if tool_call_ids.is_empty() {
        tracing::warn!(
            session_id = %session_id,
            "No tool.call_requested event found for timed-out session"
        );
    }
    let turn_id = TurnId::from_uuid(session_id.uuid());
    let message_id = MessageId::from_uuid(session_id.uuid());
    let events = tool_call_ids
        .into_iter()
        .map(|tool_call_id| {
            EventRequest::new(
                session_id,
                EventContext::turn(turn_id, message_id),
                ToolCompletedData::failure(
                    tool_call_id,
                    String::new(),
                    "timeout".to_string(),
                    "Timed out waiting for client tool results".to_string(),
                    None,
                ),
            )
        })
        .collect();
    let plan = WaitingTurnResolutionPlan {
        kind: "timeout".to_string(),
        events,
        session_values: Vec::new(),
        response: serde_json::Value::Null,
    };
    let claim = match db.recover_waiting_turn(org_id, session_id, plan).await? {
        ClaimWaitingTurnResult::Claimed(claim) => claim,
        ClaimWaitingTurnResult::Conflict { .. } | ClaimWaitingTurnResult::SessionNotFound => {
            return Ok(());
        }
    };
    execute_waiting_turn_resolution(db, event_service, runner, org_id, session_id, &claim).await?;
    tracing::info!(
        session_id = %session_id,
        resolution_kind = %claim.plan.kind,
        "Workflow resumed after parked-turn recovery"
    );
    Ok(())
}

/// Find pending tool call IDs from the most recent tool.call_requested event.
async fn find_pending_tool_call_ids(
    db: &Arc<StorageBackend>,
    session_id: SessionId,
) -> anyhow::Result<Vec<String>> {
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(1), // Only need the most recent one
        )
        .await?;

    // list_events with limit returns the LAST N events, so this is the most recent
    if let Some(event) = events.last() {
        let data = deserialize_event_data(&event.event_type, event.data.clone());
        if let EventData::ToolCallRequested(req) = data {
            return Ok(req.tool_calls.iter().map(|tc| tc.id.clone()).collect());
        }
    }

    Ok(vec![])
}
