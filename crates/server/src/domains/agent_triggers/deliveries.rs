// Delivery history for agent triggers: what each received event became.

use crate::domains::agents::AGENT_VIEW;
use crate::domains::common::*;
use everruns_contracts::typed_id::SessionId;
use serde::Deserialize;
use utoipa::ToSchema;

// ============================================================================
// ListAgentTriggerDeliveries
// ============================================================================

/// Recent events a trigger received and what happened to each.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListAgentTriggerDeliveries {
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    /// Return records older than this delivery (exclusive).
    pub before: Option<uuid::Uuid>,
    /// Owning agent's prefixed public identifier.
    pub agent_id: String,
    /// Agent trigger's prefixed public identifier.
    pub trigger_id: String,
    /// Maximum rows to return (1-200, default 50).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[command(
    name = "list_agent_trigger_deliveries",
    category = "agent_triggers",
    description = "List retained trigger deliveries (90 days), with source, authenticated manual requester, outcome, session and input-message links. Supports time windows and a stable before cursor. Legacy schedule labels with ambiguous ingress are unknown. Older or previously pruned deliveries are unavailable.",
    method = "GET",
    path = "/v1/platform/agents/{agent_id}/triggers/{trigger_id}/deliveries",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents", "triggers", "deliveries"], "list").with_examples(&[CliExample::new("Find out why a webhook or event trigger did not start a session", "everruns agents triggers deliveries list --agent-id agent_01h9 --trigger-id trg_01h9 --limit 20")]),
)]
impl Command for ListAgentTriggerDeliveries {
    type Output = Vec<crate::domains::agent_triggers::record::AgentTriggerDelivery>;

    async fn execute(
        self,
        ctx: &Ctx,
    ) -> Result<Vec<crate::domains::agent_triggers::record::AgentTriggerDelivery>, CommandError>
    {
        validate_window(self.from, self.to)?;
        let (_, trigger) =
            super::commands::resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id)
                .await?;
        let limit = self
            .limit
            .unwrap_or(50)
            .clamp(1, super::events::DELIVERY_PAGE_LIMIT);
        let rows = ctx
            .db
            .database()
            .agent_trigger_delivery_history(
                ctx.org_id(),
                trigger.id,
                limit,
                self.from,
                self.to,
                self.before,
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |row| crate::domains::agent_triggers::record::AgentTriggerDelivery {
                    input_message_id: row
                        .input_message_id
                        .map(everruns_contracts::typed_id::MessageId::from_uuid),
                    requested_by: row.requested_by,
                    id: row.id,
                    source: if row.source_version == 0 && row.source == "schedule" {
                        "unknown".to_owned()
                    } else {
                        row.source
                    },
                    event_id: row.event_id,
                    event_type: row.event_type,
                    subject: row.subject,
                    status: row.status.parse().unwrap_or(
                        crate::domains::agent_triggers::record::TriggerDeliveryStatus::Failed,
                    ),
                    reason: row.reason,
                    session_id: row.session_id.map(SessionId::from_uuid),
                    created_at: row.created_at,
                },
            )
            .collect())
    }
}

pub(crate) fn validate_window(
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), CommandError> {
    if from.zip(to).is_some_and(|(from, to)| from >= to) {
        return Err(CommandError::bad_request("from must be before to"));
    }
    Ok(())
}
