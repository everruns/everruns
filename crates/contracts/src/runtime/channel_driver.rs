//! Channel platform drivers: the contract a platform integration implements.
//!
//! Decisions:
//! - A driver is one platform: it parses and verifies a request, and it is the
//!   [`ChannelDeliveryAdapter`] that posts replies. Splitting the two halves
//!   into separate values only made hosts pair them by hand.
//! - Requests and responses are plain values, not an HTTP framework's types, so
//!   no runtime crate carries a web stack and any host (axum, a Lambda, a test) can adapt.
//! - The reply target a driver returns never holds credentials. The driver
//!   re-derives the [`DeliveryContext`] when posting, so a pending delivery can
//!   be persisted and recovered without storing a token.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::channel::{
    ChannelDeliveryAdapter, ChannelReplyMode, DeliveryContext, DeliveryTarget, InboundChannelEvent,
};

/// An inbound request to a channel.
#[derive(Debug, Clone, Default)]
pub struct ChannelRequest {
    /// Path below the channel's route, empty for the channel root.
    pub path: String,
    /// Header names and values. Lookups ignore ASCII case.
    pub headers: Vec<(String, String)>,
    /// Raw body; drivers that verify signatures need the exact bytes.
    pub body: Vec<u8>,
}

impl ChannelRequest {
    /// A JSON request to the channel root.
    pub fn json(body: &Value) -> Self {
        Self {
            path: String::new(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.to_string().into_bytes(),
        }
    }

    /// Add a header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// The first value of a header.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The body as JSON.
    pub fn body_json(&self) -> Result<Value, ChannelError> {
        serde_json::from_slice(&self.body)
            .map_err(|err| ChannelError::BadRequest(format!("body is not JSON: {err}")))
    }
}

/// The answer to a channel request.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelResponse {
    pub status: u16,
    pub body: Value,
}

impl ChannelResponse {
    /// `200` with a JSON body.
    pub fn ok(body: Value) -> Self {
        Self { status: 200, body }
    }

    /// The acknowledgement for an accepted message.
    pub fn accepted(session_id: &str) -> Self {
        Self::ok(json!({ "ok": true, "session_id": session_id }))
    }

    /// The acknowledgement for a request that needs no action.
    pub fn ignored() -> Self {
        Self::ok(json!({ "ok": true }))
    }
}

/// A message for the agent, with where its replies go.
#[derive(Debug, Clone)]
pub struct InboundMessage {
    pub event: InboundChannelEvent,
    pub reply_to: DeliveryTarget,
}

/// What a request means.
#[derive(Debug, Clone)]
pub enum Inbound {
    /// Answer the request directly (Slack's URL verification).
    Respond(ChannelResponse),
    /// Acknowledge and do nothing (a bot's own message, an unsubscribed event).
    Ignore,
    /// A message for the agent.
    Message(Box<InboundMessage>),
}

/// Why a channel request failed. Each maps to one HTTP status.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("session error: {0}")]
    Session(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl ChannelError {
    /// The HTTP status a host answers with.
    pub fn status(&self) -> u16 {
        match self {
            Self::Unauthorized(_) => 401,
            Self::BadRequest(_) => 400,
            Self::NotFound(_) => 404,
            Self::Session(_) | Self::Other(_) => 500,
        }
    }

    /// The error as a response. Server-side failures carry no detail: channel
    /// requests come from outside, and the detail is in the host's logs.
    pub fn to_response(&self) -> ChannelResponse {
        let message = match self {
            Self::Session(_) | Self::Other(_) => "internal error".to_string(),
            other => other.to_string(),
        };
        ChannelResponse {
            status: self.status(),
            body: json!({ "ok": false, "error": message }),
        }
    }
}

/// One platform: parses its requests and delivers its replies.
#[async_trait]
pub trait ChannelDriver: ChannelDeliveryAdapter + 'static {
    /// Secret names this driver reads, for manifests and deploy checks.
    fn secrets(&self) -> Vec<String> {
        Vec::new()
    }

    /// Interpret one request: verify it, then turn it into a message, a direct
    /// answer, or nothing.
    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError>;

    /// The delivery context for a target. Drivers holding credentials put
    /// their token here; the default has none.
    fn delivery_context(
        &self,
        target: &DeliveryTarget,
        reply_mode: ChannelReplyMode,
    ) -> DeliveryContext {
        target.context(String::new(), reply_mode)
    }
}
