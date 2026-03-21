// Execution metadata helpers for event provenance.
//
// Design Decision:
// - Keep provenance in event.metadata so it does not change message payload shape.
// - Record both initiator and acting_principal, but keep external_actor separate.

use everruns_core::typed_id::{AgentIdentityId, AppId, ScheduleId};
use serde_json::{Value, json};
use uuid::Uuid;

pub fn interactive_user_metadata(user_id: Option<Uuid>) -> Option<Value> {
    user_id.map(|user_id| {
        json!({
            "initiator": { "type": "user", "user_id": user_id },
            "acting_principal": { "type": "user", "user_id": user_id },
        })
    })
}

pub fn scheduled_run_metadata(
    schedule_id: ScheduleId,
    agent_identity_id: Option<AgentIdentityId>,
) -> Value {
    let acting_principal = agent_identity_id
        .map(|identity_id| json!({ "type": "agent_identity", "agent_identity_id": identity_id }))
        .unwrap_or_else(|| json!({ "type": "schedule" }));
    json!({
        "initiator": { "type": "schedule", "schedule_id": schedule_id },
        "acting_principal": acting_principal,
    })
}

pub fn app_message_metadata(app_id: AppId, agent_identity_id: Option<AgentIdentityId>) -> Value {
    let acting_principal = agent_identity_id
        .map(|identity_id| json!({ "type": "agent_identity", "agent_identity_id": identity_id }))
        .unwrap_or_else(|| json!({ "type": "app", "app_id": app_id }));
    json!({
        "initiator": { "type": "app", "app_id": app_id },
        "acting_principal": acting_principal,
    })
}
