//! The server's webhook channel as a host driver: a request becomes the
//! channel's message template, rendered against the request.
//!
//! Decisions:
//! - The token check and rate limit stay in the HTTP handler, in front of
//!   the host, with the other channels' caller checks; the handler also
//!   redacts credential headers before the request reaches the template.
//! - A webhook only takes messages in: its channel is configured without
//!   replies, and its turns run with nothing posted back.
//! - An `Idempotency-Key` header makes a retried delivery start nothing.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::Utc;
use everruns_contracts::runtime::channel::{
    ChannelDeliveryAdapter, ChannelDriver, ChannelError, ChannelRequest, DeliveryContext,
    DeliveryResult, DeliveryTarget, ExternalActor, Inbound, InboundChannelEvent, InboundMessage,
    OutboundChannelMessage,
};
use serde_json::{Value, json};

use super::ServerChannelScope;
use crate::domains::agent_channels::invocation::render_message_template;

/// Platform name of webhook channels on the host.
pub const WEBHOOK_PLATFORM: &str = "webhook";

/// A webhook channel row as a host driver.
pub struct WebhookDriver {
    scope: ServerChannelScope,
    template: String,
}

impl WebhookDriver {
    pub fn new(scope: ServerChannelScope, template: String) -> Self {
        Self { scope, template }
    }
}

#[async_trait]
impl ChannelDeliveryAdapter for WebhookDriver {
    fn platform(&self) -> &str {
        WEBHOOK_PLATFORM
    }

    async fn deliver(&self, _: &OutboundChannelMessage, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }

    async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelDriver for WebhookDriver {
    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        let ServerChannelScope { context, channel } = &self.scope;
        let body = String::from_utf8_lossy(&request.body).into_owned();
        let json_payload: Option<Value> = serde_json::from_slice(&request.body).ok();
        let headers: HashMap<&str, &str> = request
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let template_context = json!({
            "app": {
                "id": context.public_id.to_string(),
                "name": context.name,
            },
            "channel": {
                "id": channel.public_id.to_string(),
                "type": channel.channel_type.to_string(),
            },
            "invocation": {
                "source": "webhook",
                "triggered_at": Utc::now().to_rfc3339(),
            },
            "payload": json_payload.clone().unwrap_or_else(|| Value::String(body.clone())),
            "webhook": {
                "body": body,
                "json": json_payload,
                "headers": headers,
            },
        });
        let text = render_message_template(&self.template, &template_context);
        if text.trim().is_empty() {
            return Err(ChannelError::BadRequest(
                "Rendered invocation message is empty".into(),
            ));
        }
        Ok(Inbound::Message(Box::new(InboundMessage {
            event: InboundChannelEvent {
                actor: ExternalActor {
                    actor_id: channel.public_id.to_string(),
                    actor_name: None,
                    source: WEBHOOK_PLATFORM.into(),
                    metadata: None,
                },
                text,
                attachments: Vec::new(),
                dedup_key: request
                    .header_value("idempotency-key")
                    .unwrap_or_default()
                    .to_string(),
                thread_ref: None,
                routing_metadata: HashMap::new(),
                // The App-era provenance every invocation message carried.
                message_metadata: serde_json::Map::from_iter([
                    ("source".to_string(), json!("app_webhook")),
                    ("_app_id".to_string(), json!(context.public_id.to_string())),
                    (
                        "app_channel_id".to_string(),
                        json!(channel.public_id.to_string()),
                    ),
                    (
                        "app_channel_type".to_string(),
                        json!(channel.channel_type.to_string()),
                    ),
                ]),
            },
            reply_to: DeliveryTarget::default(),
        })))
    }
}
