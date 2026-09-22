use crate::services::EventService;
use crate::storage::StorageBackend;
use crate::storage::models::{UpsertSessionKeyValue, WaitingTurnResolutionClaim};
use anyhow::Result;
use everruns_core::Event;
use everruns_provider::typed_id::SessionId;
use everruns_worker::AgentRunner;
use std::sync::Arc;

pub async fn execute_waiting_turn_resolution(
    db: &Arc<StorageBackend>,
    event_service: &EventService,
    runner: &Arc<dyn AgentRunner>,
    org_id: i64,
    session_id: SessionId,
    claim: &WaitingTurnResolutionClaim,
) -> Result<Vec<Event>> {
    let result: Result<Vec<Event>> = async {
        for value in &claim.plan.session_values {
            db.upsert_session_key_value(UpsertSessionKeyValue {
                session_id,
                key: value.key.clone(),
                value: value.value.clone(),
            })
            .await?;
        }

        let mut events = Vec::with_capacity(claim.plan.events.len());
        for (index, request) in claim.plan.events.iter().cloned().enumerate() {
            events.push(
                event_service
                    .emit_waiting_turn_resolution(request, claim.resolution_id, index as i32)
                    .await?,
            );
        }

        runner
            .resume_after_tool_results(session_id, claim.resolution_id)
            .await?;
        anyhow::ensure!(
            db.complete_waiting_turn_claim(
                org_id,
                session_id,
                claim.resolution_id,
                claim.claim_token,
            )
            .await?,
            "waiting-turn resolution lease was lost before activation"
        );
        Ok(events)
    }
    .await;

    if result.is_err()
        && let Err(error) = db
            .abandon_waiting_turn_claim(org_id, session_id, claim.resolution_id, claim.claim_token)
            .await
    {
        tracing::warn!(
            session_id = %session_id,
            resolution_id = %claim.resolution_id,
            error = %error,
            "Failed to abandon waiting-turn resolution lease"
        );
    }
    result
}
