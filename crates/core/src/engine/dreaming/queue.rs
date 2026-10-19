//! Durable eligibility and coalescing, separate from the model-call step.
use super::runner::*;
use super::runner::{decode_state, fail};
use super::*;
use crate::{error::Result, typed_id::SessionId};
/// Record an actual committed successful turn, deduplicated across lifecycle retries.
pub async fn record_turn(
    store: &dyn DreamingStore,
    org: i64,
    session: SessionId,
    turn: String,
) -> Result<()> {
    let key = format!("turns:{session}");
    for _ in 0..8 {
        let (revision, mut state) = decode_state(store, org, &key).await?;
        if state.observed_turn.as_ref() == Some(&turn) {
            return Ok(());
        }
        state.observed_turn = Some(turn.clone());
        state.new_turns = Some(state.new_turns.unwrap_or(0).saturating_add(1));
        let due = state
            .job
            .as_ref()
            .filter(|j| !j.completed)
            .map(|j| j.due_at);
        if store
            .compare_exchange(
                org,
                &key,
                revision,
                &serde_json::to_string(&state).map_err(|_| fail("serialize turn counter"))?,
                Some(session),
                due,
            )
            .await?
        {
            return Ok(());
        }
    }
    Err(fail("turn counter contention"))
}

/// Coalesce unprocessed evidence. Called on committed successful turn completion.
pub async fn enqueue(
    host: &dyn DreamingHost,
    org: i64,
    session: SessionId,
    config: DreamingConfig,
    evidence: Vec<DreamEvidence>,
    now: i64,
    manual: bool,
) -> Result<bool> {
    config.validate()?;
    if !manual && config.trigger.mode == DreamTriggerMode::Manual {
        return Ok(false);
    }
    if evidence.is_empty() {
        return Ok(false);
    }
    if evidence
        .iter()
        .any(|e| e.id.len() > 256 || !matches!(e.role.as_str(), "user" | "assistant" | "tool"))
        || evidence.len() > 256
        || evidence.iter().map(|e| e.text.len()).sum::<usize>() > MAX_EVIDENCE_BYTES
    {
        return Err(fail("evidence exceeds limit"));
    }
    for d in &config.destinations {
        host.destination(session, d).await?;
    }
    let store = host.store();
    let key = job_key(session);
    for _ in 0..8 {
        let (rev, mut state) = decode_state(store.as_ref(), org, &key).await?;
        if state.job.as_ref().is_some_and(|j| !j.proposals.is_empty()) {
            return Ok(false);
        }
        if let Some(j) = state.job.as_ref().filter(|j| !j.completed) {
            let unchanged = evidence.last().is_some_and(|e| e.id == j.source_end);
            if j.attempts > 0
                || j.destination_index > 0
                || j.output.is_some()
                || config.trigger.mode != DreamTriggerMode::Idle
                || (unchanged && !manual)
            {
                return Ok(false);
            }
        }
        let start = state
            .last_source
            .as_ref()
            .and_then(|last| evidence.iter().position(|e| &e.id == last))
            .map_or(0, |n| n + 1);
        let evidence = evidence[start..].to_vec();
        let (_, turns) = decode_state(store.as_ref(), org, &format!("turns:{session}")).await?;
        let available_turns = turns
            .new_turns
            .map(|total| total.saturating_sub(state.new_turns.unwrap_or(0)) as usize)
            .unwrap_or_else(|| evidence.iter().filter(|e| e.role == "user").count());

        if evidence.is_empty()
            || (!manual && available_turns < config.trigger.min_new_turns as usize)
        {
            return Ok(false);
        }
        let end = evidence
            .last()
            .ok_or_else(|| fail("missing evidence"))?
            .id
            .clone();
        let due_at = now
            + if !manual && config.trigger.mode == DreamTriggerMode::Idle {
                i64::from(config.trigger.idle_after_seconds)
            } else {
                0
            };
        // Debouncing and a manual flush still describe the same unclaimed
        // operation. Preserve its task/Trace identity until processing begins.
        let operation_id = state
            .job
            .as_ref()
            .filter(|job| !job.completed)
            .map(|job| job.operation_id.clone())
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        state.job = Some(DreamJob {
            session_id: session,
            config: config.clone(),
            source_end: end,
            evidence,
            due_at,
            operation_id,
            destination_index: 0,
            output: None,
            proposals: Default::default(),
            review_accepting: false,
            completed: false,
            note_count: 0,
            attempts: 0,
            covered_turns: turns.new_turns.unwrap_or(0),
        });
        if store
            .compare_exchange(
                org,
                &key,
                rev,
                &serde_json::to_string(&state).map_err(|_| fail("serialize state"))?,
                Some(session),
                Some(due_at),
            )
            .await?
        {
            if let Some(job) = &state.job {
                let _ = host
                    .activity(
                        session,
                        DreamActivity {
                            operation_id: job.operation_id.clone(),
                            status: "queued".into(),
                            destination: None,
                            note_count: 0,
                            reason: None,
                            cost_usd: None,
                            model: job.config.model.clone(),
                        },
                    )
                    .await;
            }
            return Ok(true);
        }
    }
    Err(fail("concurrent enqueue; retry later"))
}
