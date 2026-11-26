// Execution metadata helpers for event provenance.
//
// Design Decision:
// - Keep provenance in event.metadata so it does not change message payload shape.
// - Record both initiator and acting_principal, but keep external_actor separate.

use everruns_contracts::typed_id::{
    AgentChannelId, PrincipalId, ScheduleId, TriggerId, VirtualUserId,
};
use serde_json::{Value, json};
use uuid::Uuid;

pub fn interactive_user_metadata(
    user_id: Option<Uuid>,
    principal_id: Option<PrincipalId>,
) -> Option<Value> {
    user_id.map(|user_id| {
        json!({
            "initiator": { "type": "user", "user_id": user_id },
            "acting_principal": { "type": "user", "user_id": user_id },
            "initiator_principal_id": principal_id,
            "acting_principal_id": principal_id,
        })
    })
}

pub fn scheduled_run_metadata(
    schedule_id: ScheduleId,
    owner_principal_id: PrincipalId,
    virtual_user_id: Option<VirtualUserId>,
) -> Value {
    let acting_principal = virtual_user_id
        .map(|identity_id| json!({ "type": "virtual_user", "virtual_user_id": identity_id }))
        .unwrap_or_else(|| json!({ "type": "schedule" }));
    json!({
        "initiator": { "type": "schedule", "schedule_id": schedule_id },
        "acting_principal": acting_principal,
        "initiator_principal_id": owner_principal_id,
        "acting_principal_id": owner_principal_id,
    })
}

/// Provenance for a message injected by an agent's own schedule trigger
/// (EVE-757). Mirrors [`channel_message_metadata`] but keyed on the trigger; the
/// acting principal is the agent-owned session owner (no virtual-user layer).
pub fn agent_trigger_message_metadata(
    trigger_id: TriggerId,
    owner_principal_id: PrincipalId,
) -> Value {
    json!({
        "initiator": { "type": "agent_trigger", "trigger_id": trigger_id },
        "acting_principal": { "type": "agent_trigger", "trigger_id": trigger_id },
        "initiator_principal_id": owner_principal_id,
        "acting_principal_id": owner_principal_id,
    })
}

/// Metadata for a message arriving through an agent channel.
///
/// `channel_id` is the stable public channel identifier the caller addressed.
pub fn channel_message_metadata(
    channel_id: AgentChannelId,
    owner_principal_id: PrincipalId,
    virtual_user_id: Option<VirtualUserId>,
) -> Value {
    let acting_principal = virtual_user_id
        .map(|identity_id| json!({ "type": "virtual_user", "virtual_user_id": identity_id }))
        .unwrap_or_else(|| json!({ "type": "channel", "channel_id": channel_id }));
    json!({
        "initiator": { "type": "channel", "channel_id": channel_id },
        "acting_principal": acting_principal,
        "initiator_principal_id": owner_principal_id,
        "acting_principal_id": owner_principal_id,
    })
}
