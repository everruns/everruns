//! Channels: where conversations come from and where replies go.
//!
//! A `#[channel]` function returns an `everruns::channels::Channel` (or just
//! a driver such as [`Slack`] or [`Webhook`]); its webhook is served at
//! `POST /v1/channels/{name}`.
//!
//! Decisions:
//! - serve runs the shared channel runtime (`everruns::channels::ChannelHost`),
//!   the one the Framework and the everruns server run: platform retries are
//!   dropped, each thread keeps one session, replies are delivered while the
//!   turn runs, and a turn cut off by a restart is finished by recovery.
//! - serve plugs in only its sessions ([`ServePort`] over the host) and its
//!   SQLite store; channel state lives next to the session catalog.
//! - Every delivery is reported on the host's notices, which is what the dev
//!   console prints and in-process evals observe.
//! - Channel and agent names share `/v1/channels/{name}`, so they must differ.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use everruns::InputMessage;
use everruns::SendDisposition;
use everruns::channels::{
    ChannelAgentSurface, ChannelDeliveryAdapter, ChannelEventStream, ChannelHost,
    ChannelSessionPort, ChannelStore, ChannelStreamDelivery, DeliveryContext, DeliveryResult,
    NewChannelSession, OutboundChannelMessage, SendOutcome, session_events,
};
use tokio::sync::broadcast;

use crate::app::App;
use crate::host::{Host, NewSession, Notice};

pub use everruns::channels::{
    Channel, ChannelDriver, ChannelError, ChannelRequest, ChannelResponse, DeliveryTarget, Inbound,
    InboundMessage, SessionBinding, Slack, Webhook,
};

/// Where a conversation started from code sends its replies: a channel of
/// this app and a target on it. `#[channel]` generates a constructor, such as
/// `slack::channel("C0123ABC")`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub channel: String,
    pub target: DeliveryTarget,
}

impl Destination {
    pub fn new(channel: impl Into<String>, target: DeliveryTarget) -> Self {
        Self {
            channel: channel.into(),
            target,
        }
    }
}

/// The channel host over `host`'s sessions and `store`.
pub(crate) fn build(
    me: Weak<Host>,
    app: &App,
    store: Arc<dyn ChannelStore>,
    notices: broadcast::Sender<Notice>,
) -> ChannelHost {
    let mut builder = ChannelHost::builder(Arc::new(ServePort { host: me })).store(store);
    for entry in &app.inner.channels {
        let (mut config, driver) = entry.channel.clone().into_parts(entry.name);
        config.agent = entry.agent.map(str::to_string);
        let driver = Arc::new(Reported {
            name: entry.name.to_string(),
            inner: driver,
            notices: notices.clone(),
        });
        builder = builder.channel(config, driver);
    }
    // AG-UI and A2A answer in the request, but bind their threads and
    // contexts through the same host and store as the messaging channels.
    for agent in app.inner.agents.iter().filter(|agent| !agent.sub) {
        for kind in STREAM_KINDS {
            let mut config =
                everruns::channels::ChannelConfig::new(stream_channel(kind, &agent.name));
            config.agent = Some(agent.name.clone());
            builder = builder.stream_channel(config, *kind);
        }
    }
    builder.build()
}

/// The streaming channel kinds this build serves.
const STREAM_KINDS: &[&str] = &[
    #[cfg(feature = "ag-ui")]
    "ag-ui",
    #[cfg(feature = "a2a")]
    "a2a",
];

/// The streaming channel of `kind` for `agent`, e.g. `ag-ui:helper`. Also the
/// key its threads are stored under.
pub(crate) fn stream_channel(kind: &str, agent: &str) -> String {
    format!("{kind}:{agent}")
}

#[cfg(any(feature = "ag-ui", feature = "a2a"))]
fn conversation_error(error: ChannelError) -> anyhow::Error {
    match error {
        ChannelError::NotFound(what) => crate::host::ApiError::NotFound(what).into(),
        ChannelError::BadRequest(why) => crate::host::ApiError::BadRequest(why).into(),
        other => anyhow::anyhow!(other.to_string()),
    }
}

/// The session behind conversation `key` of `agent`'s `kind` channel, created
/// on first use, and whether this call created it.
#[cfg(any(feature = "ag-ui", feature = "a2a"))]
pub(crate) async fn conversation(
    host: &Host,
    kind: &str,
    agent: &str,
    key: &str,
    title: &str,
) -> crate::Result<everruns::channels::Conversation> {
    host.channels()
        .conversation(&stream_channel(kind, agent), key, title)
        .await
        .map_err(conversation_error)
}

/// serve's sessions, as the channel host sees them.
struct ServePort {
    host: Weak<Host>,
}

impl ServePort {
    fn host(&self) -> Result<Arc<Host>, ChannelError> {
        self.host
            .upgrade()
            .ok_or_else(|| ChannelError::Session("host is shutting down".into()))
    }
}

fn session_error(error: anyhow::Error) -> ChannelError {
    match error.downcast_ref::<crate::host::ApiError>() {
        Some(crate::host::ApiError::NotFound(what)) => ChannelError::NotFound(what.clone()),
        Some(crate::host::ApiError::BadRequest(why)) => ChannelError::BadRequest(why.clone()),
        _ => ChannelError::Session(format!("{error:#}")),
    }
}

#[async_trait]
impl ChannelSessionPort for ServePort {
    async fn create_session(&self, request: NewChannelSession) -> Result<String, ChannelError> {
        self.host()?
            .create_session(NewSession {
                agent: request.agent,
                title: Some(request.title),
                metadata: Some(request.metadata),
                ..NewSession::default()
            })
            .await
            .map_err(session_error)
    }

    async fn send(
        &self,
        _channel: &str,
        session_id: &str,
        message: InputMessage,
    ) -> Result<SendOutcome, ChannelError> {
        let sent = self
            .host()?
            .send_message(session_id, message)
            .await
            .map_err(session_error)?;
        Ok(match sent.disposition {
            SendDisposition::Steered => SendOutcome::Steered,
            _ => SendOutcome::Started {
                input_message_id: sent.message_id,
            },
        })
    }

    async fn events(
        &self,
        _channel: &str,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<ChannelEventStream, ChannelError> {
        let session = self
            .host()?
            .session(session_id)
            .await
            .map_err(session_error)?;
        session_events(&session, after).await
    }
}

/// A driver whose deliveries are reported on the host's notices.
struct Reported {
    name: String,
    inner: Arc<dyn ChannelDriver>,
    notices: broadcast::Sender<Notice>,
}

impl Reported {
    fn report(&self, session_id: String, context: &DeliveryContext, result: &DeliveryResult) {
        let to = if context.thread_ref.is_empty() {
            format!("{}:{}", self.name, context.channel_id)
        } else {
            format!(
                "{}:{}/{}",
                self.name, context.channel_id, context.thread_ref
            )
        };
        let error = match result {
            DeliveryResult::Ok => None,
            DeliveryResult::TransientError(error) | DeliveryResult::PermanentError(error) => {
                Some(error.clone())
            }
        };
        let _ = self.notices.send(Notice::Delivered {
            session_id,
            to,
            error,
        });
    }
}

#[async_trait]
impl ChannelDeliveryAdapter for Reported {
    fn platform(&self) -> &str {
        self.inner.platform()
    }

    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        let result = self.inner.deliver(message, context).await;
        self.report(message.session_id.to_string(), context, &result);
        result
    }

    async fn send_ack(
        &self,
        thread_ref: &str,
        text: &str,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        self.inner.send_ack(thread_ref, text, context).await
    }

    fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
        self.inner.streaming()
    }

    fn agent_surface(&self) -> Option<&dyn ChannelAgentSurface> {
        self.inner.agent_surface()
    }
}

#[async_trait]
impl ChannelDriver for Reported {
    fn secrets(&self) -> Vec<String> {
        self.inner.secrets()
    }

    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        self.inner.receive(request).await
    }

    fn delivery_context(&self, target: &DeliveryTarget) -> DeliveryContext {
        self.inner.delivery_context(target)
    }
}
