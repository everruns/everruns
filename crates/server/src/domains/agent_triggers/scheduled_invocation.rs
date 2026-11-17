//! Exactly one committed dispatch per durable schedule task, including retries.

use super::{AgentTriggerInvocationResult, invoke_agent_trigger};
use crate::domains::{common::CommandError, messages::MessageService, sessions::SessionService};
use crate::storage::{EncryptionService, StorageBackend};
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
    let trigger = trigger_id
        .parse()
        .map_err(|_| CommandError::bad_request("Invalid trigger id"))?;
    crate::storage::transaction::scope(
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
                return Ok(serde_json::from_value(result)
                    .map_err(|error| CommandError::internal(error.into()))?);
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
    .await
}
