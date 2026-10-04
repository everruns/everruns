//! Webhook trigger invocation: a token-authenticated HTTP request becomes a
//! trigger event. Authentication and rate limiting happen in
//! `api::channel_webhooks`; everything after normalization is the shared pipeline
//! in [`super::events`].

use super::commands::{WebhookCompatibilityContext, parse_agent_id};
use super::events;
use super::queries as q;
use crate::domains::agent_channels::invocation::render_message_template;
use crate::domains::common::{CommandError, classify_anyhow};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::storage::StorageBackend;
use chrono::Utc;
use everruns_platform::AgentTriggerType;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct WebhookTriggerInvocationRequest {
    pub ingress_id: String,
    pub body: String,
    pub json_payload: Option<Value>,
    pub headers: HashMap<String, String>,
}

pub async fn invoke_webhook_agent_trigger(
    db: &Arc<StorageBackend>,
    encryption: Option<&Arc<crate::storage::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: WebhookTriggerInvocationRequest,
    request_id: Option<String>,
) -> Result<events::TriggerEventOutcome, CommandError> {
    let trigger_row = db
        .get_agent_trigger_by_ingress_id_unscoped(&req.ingress_id)
        .await
        .map_err(classify_anyhow)?
        .filter(|row| row.trigger_type == AgentTriggerType::Webhook.to_string())
        .ok_or_else(|| CommandError::not_found("Agent trigger"))?;
    if !trigger_row.enabled {
        return Err(CommandError::forbidden(
            "Agent trigger is disabled".to_string(),
        ));
    }
    let agent = db
        .get_agent(trigger_row.org_id, trigger_row.agent_id)
        .await
        .map_err(classify_anyhow)?
        .filter(|agent| agent.status == "active" && !agent.exposures_suspended)
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    let trigger = q::row_to_trigger(
        trigger_row.clone(),
        parse_agent_id(&agent.public_id)?,
        encryption,
    );
    let config = trigger
        .webhook_config()
        .map_err(|_| CommandError::bad_request("Invalid webhook trigger configuration"))?;

    let webhook_context = if trigger_row.execution_app_id.is_some() {
        Some(WebhookCompatibilityContext {
            app_public_id: trigger_row
                .legacy_alias_id
                .clone()
                .ok_or_else(|| CommandError::not_found("App channel"))?,
            app_name: trigger_row
                .legacy_alias_name
                .clone()
                .ok_or_else(|| CommandError::not_found("App channel"))?,
            ingress_id: req.ingress_id.clone(),
        })
    } else {
        None
    };
    let legacy_app = webhook_context
        .as_ref()
        .map(|context| {
            json!({
                "id": context.app_public_id,
                "name": context.app_name,
            })
        })
        .unwrap_or_else(|| json!({"id": "", "name": ""}));
    let context = json!({
        "agent": {
            "id": agent.public_id,
            "name": agent.name,
        },
        "trigger": {
            "id": trigger.id.to_string(),
            "type": "webhook",
        },
        "endpoint": {
            "id": req.ingress_id,
            "type": "webhook",
        },
        "app": legacy_app,
        "channel": {
            "id": req.ingress_id,
            "type": "webhook",
        },
        "invocation": {
            "source": "webhook",
            "triggered_at": Utc::now().to_rfc3339(),
        },
        "payload": req
            .json_payload
            .clone()
            .unwrap_or_else(|| Value::String(req.body.clone())),
        "webhook": {
            "body": req.body,
            "json": req.json_payload,
            "headers": req.headers,
        },
    });
    let render_optional = |template: &Option<String>| {
        template
            .as_deref()
            .map(|template| render_message_template(template, &context))
    };
    let event = events::TriggerEvent {
        source: "webhook",
        event_id: render_optional(&config.event_id_template),
        event_type: None,
        subject: render_optional(&config.subject_template),
        context: context.clone(),
    };
    events::dispatch_trigger_event(
        db,
        session_service,
        message_service,
        events::TriggerEventRoute {
            trigger: &trigger_row,
            agent: &agent,
            message_template: &config.message,
            session_mode: config.session_mode,
            filter: config.filter.as_ref(),
            session_source: everruns_platform::SessionSource::Webhook,
            webhook_compat: webhook_context.as_ref(),
        },
        event,
        request_id,
    )
    .await
}
