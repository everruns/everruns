//! Exactly one committed dispatch per durable schedule task, including retries.
//!
//! Decision: the dispatch runs in one transaction, so a failed attempt rolls
//! back its session, message and delivery row together. The failure is then
//! recorded as a `failed` delivery on its own, after the rollback: without it
//! a schedule that never starts a session leaves no trace in the trigger's
//! history (the durable task's error is the only other record).

use super::{AgentTriggerInvocationResult, invoke_agent_trigger};
use crate::domains::agent_triggers::record::TriggerDeliveryStatus;
use crate::domains::{common::CommandError, messages::MessageService, sessions::SessionService};
use crate::storage::agent_trigger_deliveries::CreateAgentTriggerDeliveryRow;
use crate::storage::{EncryptionService, StorageBackend};
use everruns_contracts::typed_id::TriggerId;
use std::sync::Arc;
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub async fn invoke_scheduled_agent_trigger(
    db: &Arc<StorageBackend>,
    session_service: &SessionService,
    message_service: &MessageService,
    channels: Option<&crate::channels::host::ServerChannels>,
    encryption: Option<&Arc<EncryptionService>>,
    org_id: i64,
    agent_id: &str,
    trigger_id: &str,
    invocation_id: Uuid,
) -> Result<AgentTriggerInvocationResult, CommandError> {
    let trigger: TriggerId = trigger_id
        .parse()
        .map_err(|_| CommandError::bad_request("Invalid trigger id"))?;
    let result = crate::storage::transaction::scope(
        db,
        Box::pin(async {
            // Serialize retries before reading the receipt. The lock is released
            // by rollback on cancellation/crash, never by an expiring lease.
            db.lock_scheduled_trigger_invocation(invocation_id).await?;
            if !db
                .scheduled_trigger_task_matches(invocation_id, org_id, agent_id, trigger_id)
                .await?
            {
                return Err(CommandError::bad_request(
                    "Scheduled invocation does not match its durable task",
                ));
            }
            if let Some(result) = db
                .scheduled_trigger_invocation(invocation_id, org_id, trigger)
                .await?
            {
                return serde_json::from_value(result)
                    .map_err(|error| CommandError::internal(error.into()));
            }
            let result = invoke_agent_trigger(
                db,
                session_service,
                message_service,
                channels,
                encryption,
                org_id,
                agent_id,
                trigger_id,
            )
            .await?;
            db.record_scheduled_trigger_invocation(
                invocation_id,
                org_id,
                trigger,
                serde_json::to_value(&result)
                    .map_err(|error| CommandError::internal(error.into()))?,
            )
            .await?;
            Ok(result)
        }),
        CommandError::internal,
    )
    .await;
    if let Err(error) = &result {
        record_failed_attempt(db, org_id, trigger, error).await;
    }
    result
}

/// Record a rolled-back attempt as a `failed` delivery, best effort.
async fn record_failed_attempt(
    db: &Arc<StorageBackend>,
    org_id: i64,
    trigger_id: TriggerId,
    error: &CommandError,
) {
    tracing::warn!(%error, %trigger_id, "scheduled agent trigger attempt failed");
    // Only a trigger of this org gets the record (the task input is checked
    // against its durable task inside the failed transaction).
    match db.get_agent_trigger(org_id, trigger_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return,
        Err(lookup_error) => {
            tracing::warn!(%lookup_error, %trigger_id, "failed to read trigger for failure record");
            return;
        }
    }
    let row = CreateAgentTriggerDeliveryRow {
        org_id,
        trigger_id,
        source: "schedule".to_string(),
        event_id: None,
        event_type: None,
        subject: None,
        status: TriggerDeliveryStatus::Failed.as_str().to_string(),
        reason: Some(super::events::failure_reason(error, trigger_id)),
    };
    if let Err(record_error) = db.record_agent_trigger_delivery(row).await {
        tracing::warn!(%record_error, %trigger_id, "failed to record scheduled trigger failure");
    }
}
