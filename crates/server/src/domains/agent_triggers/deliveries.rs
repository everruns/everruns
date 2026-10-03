// Delivery history for agent triggers: what each received event became.

use crate::domains::agents::AGENT_VIEW;
use crate::domains::common::*;
use crate::kernel_imports::Policy;
use everruns_contracts::typed_id::SessionId;
use serde::Deserialize;
use utoipa::ToSchema;

// ============================================================================
// ListAgentTriggerDeliveries
// ============================================================================

/// Recent events a trigger received and what happened to each.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListAgentTriggerDeliveries {
    pub agent_id: String,
    pub trigger_id: String,
    /// Maximum rows to return (1-200, default 50).
    #[serde(default)]
    pub limit: Option<i64>,
}

impl Command for ListAgentTriggerDeliveries {
    type Output = Vec<everruns_platform::AgentTriggerDelivery>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agent_trigger_deliveries",
            category: "agent_triggers",
            description: "List recent events delivered to an agent trigger: dispatched, filtered, duplicate or failed.",
            method: "GET",
            path: "/v1/agents/{agent_id}/triggers/{trigger_id}/deliveries",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(
        self,
        ctx: &Ctx,
    ) -> Result<Vec<everruns_platform::AgentTriggerDelivery>, CommandError> {
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
            .await
            .map_err(classify_anyhow)?;
        Ok(rows
            .into_iter()
            .map(|row| everruns_platform::AgentTriggerDelivery {
                id: row.id,
                source: row.source,
                event_id: row.event_id,
                event_type: row.event_type,
                subject: row.subject,
                status: row
                    .status
                    .parse()
                    .unwrap_or(everruns_platform::TriggerDeliveryStatus::Failed),
                reason: row.reason,
                session_id: row.session_id.map(SessionId::from_uuid),
                created_at: row.created_at,
            })
            .collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentTriggerDeliveries>() }
