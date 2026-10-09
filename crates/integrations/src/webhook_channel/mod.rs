//! A generic JSON webhook channel.
//!
//! In: `POST` a body `{"text": "...", "thread": "...", "user": "...", "id": "..."}`.
//! Only `text` is required. `thread` keys the session (one per thread by
//! default), `user` attributes the message, `id` makes retries idempotent.
//!
//! Out: each reply is `POST`ed as `{"thread", "text", "session_id"}` to the
//! callback URL configured on the channel, or logged when there is none.
//!
//! Decisions:
//! - The callback is channel configuration, never request input: a callback
//!   taken from the body would let any caller make the host send requests
//!   anywhere.
//! - With a secret configured, requests must carry `Authorization: Bearer
//!   <secret>`, compared in constant time.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::info;

use everruns_contracts::runtime::channel::{
    ChannelDeliveryAdapter, ChannelDriver, ChannelError, ChannelRequest, DeliveryContext,
    DeliveryResult, DeliveryTarget, ExternalActor, Inbound, InboundChannelEvent, InboundMessage,
    OutboundChannelMessage,
};

/// The generic webhook driver.
#[derive(Debug, Clone, Default)]
pub struct Webhook {
    callback: Option<String>,
    secret: Option<String>,
    client: reqwest::Client,
}

impl Webhook {
    /// A webhook channel that logs replies until a callback is set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Post replies to this URL.
    pub fn callback(mut self, url: impl Into<String>) -> Self {
        self.callback = Some(url.into());
        self
    }

    /// Require `Authorization: Bearer <secret>` on every request.
    pub fn secret(mut self, secret: impl Into<String>) -> Self {
        self.secret = Some(secret.into());
        self
    }

    fn verify(&self, request: &ChannelRequest) -> Result<(), ChannelError> {
        let Some(secret) = &self.secret else {
            return Ok(());
        };
        let presented = request
            .header_value("authorization")
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or("");
        if constant_time_eq(presented.as_bytes(), secret.as_bytes()) {
            Ok(())
        } else {
            Err(ChannelError::Unauthorized(
                "missing or wrong bearer token".into(),
            ))
        }
    }
}

#[async_trait]
impl ChannelDeliveryAdapter for Webhook {
    fn platform(&self) -> &str {
        "webhook"
    }

    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        _context: &DeliveryContext,
    ) -> DeliveryResult {
        let body = json!({
            "thread": message.thread_ref,
            "text": message.text,
            "session_id": message.session_id.to_string(),
        });
        let Some(url) = &self.callback else {
            info!(thread = %message.thread_ref, text = %message.text, "webhook reply (no callback configured)");
            return DeliveryResult::Ok;
        };
        match self.client.post(url).json(&body).send().await {
            Ok(response) if response.status().is_success() => DeliveryResult::Ok,
            Ok(response) if response.status().is_client_error() => {
                DeliveryResult::PermanentError(format!("callback answered {}", response.status()))
            }
            Ok(response) => {
                DeliveryResult::TransientError(format!("callback answered {}", response.status()))
            }
            Err(error) => DeliveryResult::TransientError(error.to_string()),
        }
    }

    async fn send_ack(
        &self,
        _thread_ref: &str,
        _text: &str,
        _context: &DeliveryContext,
    ) -> DeliveryResult {
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelDriver for Webhook {
    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        self.verify(request)?;
        let body = request.body_json()?;
        let field = |name: &str| body.get(name).and_then(Value::as_str).map(str::to_string);
        let Some(text) = field("text").filter(|text| !text.trim().is_empty()) else {
            return Err(ChannelError::BadRequest(
                "body needs a non-empty `text` string".into(),
            ));
        };
        let thread = field("thread").unwrap_or_else(|| "default".to_string());
        let user = field("user").unwrap_or_else(|| "webhook".to_string());
        Ok(Inbound::Message(Box::new(InboundMessage {
            event: InboundChannelEvent {
                actor: ExternalActor {
                    actor_id: user.clone(),
                    actor_name: field("user_name"),
                    source: "webhook".into(),
                    metadata: None,
                },
                text,
                attachments: Vec::new(),
                dedup_key: field("id").unwrap_or_default(),
                thread_ref: Some(thread.clone()),
                routing_metadata: HashMap::new(),
            },
            reply_to: DeliveryTarget::new("webhook", thread),
        })))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(body: Value) -> ChannelRequest {
        ChannelRequest::json(&body)
    }

    #[tokio::test]
    async fn a_message_becomes_an_inbound_event_on_its_thread() {
        let inbound = Webhook::new()
            .receive(&request(
                json!({"text": "hi", "thread": "t1", "user": "u1", "id": "m1"}),
            ))
            .await
            .unwrap();
        let Inbound::Message(message) = inbound else {
            panic!("expected a message")
        };
        assert_eq!(message.event.text, "hi");
        assert_eq!(message.event.thread_ref.as_deref(), Some("t1"));
        assert_eq!(message.event.actor.actor_id, "u1");
        assert_eq!(message.event.dedup_key, "m1");
        assert_eq!(message.reply_to.thread_ref, "t1");
    }

    #[tokio::test]
    async fn a_body_without_text_is_refused() {
        let error = Webhook::new()
            .receive(&request(json!({"thread": "t"})))
            .await
            .unwrap_err();
        assert_eq!(error.status(), 400);
        let error = Webhook::new()
            .receive(&request(json!({"text": "  "})))
            .await
            .unwrap_err();
        assert_eq!(error.status(), 400);
    }

    #[tokio::test]
    async fn replies_post_to_the_configured_callback_only() {
        use wiremock::matchers::{body_partial_json, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"thread": "t1", "text": "hello"})))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let hook = Webhook::new().callback(server.uri());
        let message = OutboundChannelMessage {
            session_id: everruns_contracts::typed_id::SessionId::new(),
            text: "hello".into(),
            thread_ref: "t1".into(),
            correlation_id: None,
        };
        let context = DeliveryTarget::new("webhook", "t1").context(
            String::new(),
            everruns_contracts::runtime::channel::ChannelReplyMode::AllMessages,
        );
        assert!(matches!(
            hook.deliver(&message, &context).await,
            DeliveryResult::Ok
        ));

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let gone = OutboundChannelMessage {
            text: "other".into(),
            ..message
        };
        assert!(matches!(
            hook.deliver(&gone, &context).await,
            DeliveryResult::PermanentError(_)
        ));
    }

    #[tokio::test]
    async fn a_configured_secret_is_required() {
        let hook = Webhook::new().secret("s3cret");
        let body = json!({"text": "hi"});
        assert_eq!(
            hook.receive(&request(body.clone()))
                .await
                .unwrap_err()
                .status(),
            401
        );
        let wrong = request(body.clone()).header("Authorization", "Bearer nope");
        assert_eq!(hook.receive(&wrong).await.unwrap_err().status(), 401);
        let right = request(body).header("authorization", "Bearer s3cret");
        assert!(matches!(
            hook.receive(&right).await.unwrap(),
            Inbound::Message(_)
        ));
    }
}
