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
    description = "List recent events delivered to an agent trigger: dispatched, filtered, duplicate or failed.",
    method = "GET",
    path = "/v1/agents/{agent_id}/triggers/{trigger_id}/deliveries",
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
        let (_, trigger) =
            super::commands::resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id)
                .await?;
        let limit = self
            .limit
            .unwrap_or(50)
            .clamp(1, super::events::DELIVERY_HISTORY_LIMIT);
        let rows = ctx
            .db
            .list_agent_trigger_deliveries(ctx.org_id(), trigger.id, limit)
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |row| crate::domains::agent_triggers::record::AgentTriggerDelivery {
                    id: row.id,
                    source: row.source,
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
