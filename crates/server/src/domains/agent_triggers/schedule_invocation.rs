//! Schedule trigger invocation: a durable schedule tick (or a manual fire)
//! becomes a trigger event. Everything after the context is the shared
//! pipeline in [`super::events`].

use super::commands::parse_agent_id;
use super::events;
use super::manual::ManualFire;
use super::queries as q;
use crate::domains::common::CommandError;
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::storage::{AgentRow, AgentTriggerRow, StorageBackend};
use chrono::Utc;
use serde_json::{Value, json};
use std::sync::Arc;

/// Run one schedule trigger. A manual fire with an event passes `manual` and
/// the event's payload, which templates read as `{{payload.*}}`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_schedule(
    db: &Arc<StorageBackend>,
    session_service: &SessionService,
    message_service: &MessageService,
    channels: Option<&crate::channels::host::ServerChannels>,
    encryption: Option<&Arc<crate::storage::EncryptionService>>,
    trigger_row: &AgentTriggerRow,
    agent: &AgentRow,
    manual: Option<(&ManualFire, Value)>,
) -> Result<events::TriggerEventOutcome, CommandError> {
    let trigger = q::row_to_trigger(trigger_row.clone(), parse_agent_id(&agent.public_id)?, None);
    let config = trigger
        .schedule_config()
        .map_err(|_| CommandError::bad_request("Invalid schedule trigger configuration"))?;
    let fire = manual.as_ref().map(|(fire, _)| *fire);
    let mut context = json!({
        "agent": {
            "id": agent.public_id,
            "name": agent.name.clone(),
        },
        "trigger": {
            "id": trigger_row.id.to_string(),
            "type": trigger.trigger_type.to_string(),
        },
        "invocation": {
            "source": if fire.is_some() { super::manual::SOURCE } else { "agent_trigger" },
            "triggered_at": Utc::now().to_rfc3339(),
        },
    });
    if let Some((_, payload)) = manual {
        context["payload"] = payload;
    }
    events::dispatch_trigger_event(
        db,
        session_service,
        message_service,
        events::TriggerEventRoute {
            trigger: trigger_row,
            agent,
            message_template: &config.message,
            session_mode: config.session_mode,
            filter: None,
            session_source: crate::domains::sessions::record::SessionSource::Schedule,
            webhook_compat: None,
            script: config.script.as_ref(),
            channel: config.target.as_ref().map(|target| events::ChannelRun {
                target,
                channels,
                encryption,
            }),
        },
        events::TriggerEvent {
            source: super::manual::source_name(fire, "schedule"),
            // Only an explicit id dedupes: two manual test fires both run.
            event_id: fire
                .filter(|fire| fire.explicit_id)
                .map(|fire| fire.event_id.clone()),
            event_type: None,
            subject: None,
            context,
        },
        None,
    )
    .await
}
