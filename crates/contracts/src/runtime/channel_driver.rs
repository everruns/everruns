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
    ChannelDeliveryAdapter, DeliveryContext, DeliveryTarget, ExternalActor, InboundChannelEvent,
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
    /// A platform action on the conversation's session, not a message (Slack's
    /// assistant pane stop button, a renamed thread).
    Control(Box<InboundControl>),
    /// A person answered an approval the agent asked for, from a platform
    /// button.
    Decision(Box<InboundDecision>),
}

/// Where a conversation is on the platform: what a session binding is derived
/// from, without a message.
#[derive(Debug, Clone)]
pub struct InboundRoute {
    pub actor: ExternalActor,
    pub thread_ref: Option<String>,
    pub routing_metadata: std::collections::HashMap<String, String>,
    pub reply_to: DeliveryTarget,
}

impl InboundMessage {
    /// Where this message's conversation is.
    pub fn route(&self) -> InboundRoute {
        InboundRoute {
            actor: self.event.actor.clone(),
            thread_ref: self.event.thread_ref.clone(),
            routing_metadata: self.event.routing_metadata.clone(),
            reply_to: self.reply_to.clone(),
        }
    }
}

/// An action on the session bound to a conversation. A conversation with no
/// session yet ignores it.
#[derive(Debug, Clone)]
pub struct InboundControl {
    pub route: InboundRoute,
    pub action: ControlAction,
}

/// What a control event asks of the session.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlAction {
    /// Stop the running turn.
    Cancel,
    /// The conversation was renamed on the platform.
    Rename { title: String },
    /// The platform reported context the agent may use (Slack's open view),
    /// kept on the session under `key`.
    Context { key: String, value: Value },
}

/// An approval answered on the platform. The driver put the session in the
/// prompt it rendered; the host's port checks that the session belongs to the
/// channel and that the decider may decide.
#[derive(Debug, Clone)]
pub struct InboundDecision {
    pub session_id: String,
    /// The parked tool call the approval belongs to.
    pub call_id: String,
    pub approved: bool,
    pub reason: Option<String>,
    pub actor: ExternalActor,
    /// Where the resumed turn's replies go.
    pub reply_to: DeliveryTarget,
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
    fn delivery_context(&self, target: &DeliveryTarget) -> DeliveryContext {
        target.context(String::new())
    }

    /// Whether the agent takes this message, asked after the duplicate check
    /// and before a session is created. `session_id` is the conversation's
    /// session when it has one. Slack's relevance policy answers here; the
    /// default takes every message.
    async fn admit(
        &self,
        _message: &InboundMessage,
        _session_id: Option<&str>,
    ) -> Result<bool, ChannelError> {
        Ok(true)
    }

    /// Earlier messages of the conversation, oldest first, to seed a session
    /// created for a thread already under way. The default has none.
    async fn backfill(
        &self,
        _message: &InboundMessage,
    ) -> Result<Vec<InboundChannelEvent>, ChannelError> {
        Ok(Vec::new())
    }

    /// Called after a decision was applied, so the driver can update the
    /// prompt it rendered. The default does nothing.
    async fn decided(&self, _decision: &InboundDecision, _context: &DeliveryContext) {}
}
