//! Channels: put an agent on a messaging platform.
//!
//! A [`Channel`] is a name, a platform driver, and how conversations map to
//! sessions and replies. [`Channels`] runs them against an [`Engine`]: hand
//! it each platform request, it answers at once, starts or steers the
//! conversation's session, and posts the agent's replies back while the turn
//! runs.
//!
//! ```no_run
//! # async fn demo(engine: everruns::Engine, agent: everruns::Agent) {
//! use everruns::channels::{Channel, ChannelRequest, Channels, SessionBinding, Webhook};
//!
//! let channels = Channels::builder(&engine)
//!     .channel(
//!         Channel::new("support", Webhook::new().callback("https://example.com/replies"))
//!             .binding(SessionBinding::Thread),
//!         agent,
//!     )
//!     .build();
//!
//! // In your HTTP handler: the request in, the response out.
//! let request = ChannelRequest::json(&serde_json::json!({"text": "hello", "thread": "t1"}));
//! let response = channels.handle("support", &request).await;
//! assert_eq!(response.status, 200);
//! # }
//! ```
//!
//! Stability: experimental — outside the [`stability`](crate::stability)
//! promises.
//!
//! Decisions:
//! - The runtime is `everruns_core::channel_runtime::ChannelHost`, the one serve and
//!   the server run, so a channel behaves the same everywhere: the same
//!   duplicate check, session binding, streaming, notices and recovery.
//! - No HTTP framework here. A request is a path, headers and a body; mount
//!   [`Channels::handle`] on any server.
//! - Each channel names its agent. A session the engine no longer holds (the
//!   process restarted on a durable backend) is reattached with that agent
//!   before it is resumed.

use std::collections::HashMap;
use std::sync::Arc;

use crate::SessionId;
use async_trait::async_trait;
use everruns_core::channel_runtime::{
    ChannelConfig, ChannelEventStream, ChannelHost, ChannelSessionPort, DeliveryEvent,
    NewChannelSession, SendOutcome,
};
use futures::StreamExt;
use tracing::debug;

use crate::{Agent, Engine, InputMessage, ResumeError, SendDisposition, Session};

pub use everruns_core::channel::{
    ChannelAgentSurface, ChannelDeliveryAdapter, ChannelDriver, ChannelError, ChannelReplyMode,
    ChannelRequest, ChannelResponse, ChannelStreamDelivery, DeliveryContext, DeliveryResult,
    DeliveryTarget, ExternalActor, Inbound, InboundAttachment, InboundChannelEvent, InboundMessage,
    OutboundChannelMessage, SessionBinding,
};
pub use everruns_core::channel_runtime::{
    ChannelStore, DeliveryOptions, MemoryChannelStore, PendingDelivery,
};
pub use everruns_integrations::webhook_channel::Webhook;

/// One channel: a name, its platform driver, and how it binds sessions and
/// delivers replies.
#[derive(Clone)]
pub struct Channel {
    config: ChannelConfig,
    driver: Arc<dyn ChannelDriver>,
}

impl Channel {
    /// A channel named `name` on `driver`. Defaults: one session per platform
    /// thread, every assistant message posted, streamed where the platform
    /// can.
    pub fn new(name: impl Into<String>, driver: impl ChannelDriver) -> Self {
        Self {
            config: ChannelConfig::new(name),
            driver: Arc::new(driver),
        }
    }

    /// How conversations map to sessions.
    pub fn binding(mut self, binding: SessionBinding) -> Self {
        self.config.binding = binding;
        self
    }

    /// Post every assistant message, or only what the agent sends with the
    /// `channel_post_message` tool.
    pub fn reply_mode(mut self, mode: ChannelReplyMode) -> Self {
        self.config.delivery.reply_mode = mode;
        self
    }

    /// Stream replies while they are written, where the platform can. On by
    /// default.
    pub fn stream(mut self, stream: bool) -> Self {
        self.config.delivery.stream = stream;
        self
    }

    /// A link to the session, appended to the notice posted when a turn
    /// delivered nothing.
    pub fn session_link(mut self, link: impl Into<String>) -> Self {
        self.config.delivery.session_link = Some(link.into());
        self
    }

    /// The channel's name.
    pub fn name(&self) -> &str {
        &self.config.name
    }
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("name", &self.config.name)
            .field("platform", &self.driver.platform())
            .finish_non_exhaustive()
    }
}

/// Builds [`Channels`].
pub struct ChannelsBuilder {
    engine: Engine,
    channels: Vec<(Channel, Agent)>,
    store: Option<Arc<dyn ChannelStore>>,
}

impl ChannelsBuilder {
    /// Add a channel and the agent that answers on it. A later channel with
    /// the same name replaces it.
    pub fn channel(mut self, channel: Channel, agent: Agent) -> Self {
        self.channels.push((channel, agent));
        self
    }

    /// Keep session bindings and unfinished deliveries here. Defaults to
    /// memory, which forgets them on restart.
    pub fn store(mut self, store: Arc<dyn ChannelStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Build the channels. Nothing runs until a request is handled.
    pub fn build(self) -> Channels {
        let mut agents = HashMap::new();
        let mut configs = Vec::new();
        for (channel, agent) in self.channels {
            agents.insert(channel.config.name.clone(), agent);
            configs.push(channel);
        }
        let port = Arc::new(EnginePort {
            engine: self.engine,
            agents,
        });
        let mut builder = ChannelHost::builder(port);
        if let Some(store) = self.store {
            builder = builder.store(store);
        }
        for channel in configs {
            builder = builder.channel(channel.config, channel.driver);
        }
        Channels {
            host: builder.build(),
        }
    }
}

/// Channels running against one [`Engine`]. Cheap to clone.
#[derive(Clone)]
pub struct Channels {
    host: ChannelHost,
}

impl Channels {
    /// Start building channels that run sessions on `engine`.
    pub fn builder(engine: &Engine) -> ChannelsBuilder {
        ChannelsBuilder {
            engine: engine.clone(),
            channels: Vec::new(),
            store: None,
        }
    }

    /// Answer one platform request on `channel`. Never fails: errors become
    /// the response a platform should see (401, 400, 404, 500).
    pub async fn handle(&self, channel: &str, request: &ChannelRequest) -> ChannelResponse {
        self.host.handle(channel, request).await
    }

    /// Start a conversation from this side, such as a scheduled post: a new
    /// session on `channel` whose replies go to `target`. Returns the session
    /// id.
    pub async fn start(
        &self,
        channel: &str,
        target: DeliveryTarget,
        text: &str,
    ) -> Result<String, ChannelError> {
        self.host.start(channel, target, text).await
    }

    /// Finish the deliveries a previous process left unfinished. Needs a
    /// durable [`store`](ChannelsBuilder::store) and a durable engine
    /// backend. Returns how many were resumed.
    pub async fn recover(&self) -> Result<usize, ChannelError> {
        self.host.recover().await
    }

    /// Channel names, sorted.
    pub fn names(&self) -> Vec<String> {
        self.host.channel_names()
    }

    /// The shared runtime underneath, for hosts that need more than this
    /// surface.
    pub fn host(&self) -> &ChannelHost {
        &self.host
    }
}

impl std::fmt::Debug for Channels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channels")
            .field("channels", &self.host.channel_names())
            .finish()
    }
}

/// The session side of channels, over an [`Engine`].
struct EnginePort {
    engine: Engine,
    agents: HashMap<String, Agent>,
}

impl EnginePort {
    fn agent(&self, channel: &str) -> Result<&Agent, ChannelError> {
        self.agents
            .get(channel)
            .ok_or_else(|| ChannelError::NotFound(format!("channel {channel}")))
    }

    async fn session(&self, channel: &str, session_id: &str) -> Result<Session, ChannelError> {
        let id: SessionId = session_id
            .parse()
            .map_err(|_| ChannelError::Session(format!("invalid session id {session_id}")))?;
        match self.engine.resume(id).await {
            Err(ResumeError::SessionNotFound { .. }) => {
                self.engine
                    .attach(id, self.agent(channel)?.clone())
                    .await
                    .map_err(session_error)?;
                self.engine.resume(id).await.map_err(session_error)
            }
            result => result.map_err(session_error),
        }
    }
}

fn session_error(error: impl std::fmt::Display) -> ChannelError {
    ChannelError::Session(error.to_string())
}

#[async_trait]
impl ChannelSessionPort for EnginePort {
    async fn create_session(&self, request: NewChannelSession) -> Result<String, ChannelError> {
        let session = self.engine.create(self.agent(&request.channel)?.clone());
        Ok(session.id())
    }

    async fn send(
        &self,
        channel: &str,
        session_id: &str,
        message: InputMessage,
    ) -> Result<SendOutcome, ChannelError> {
        let session = self.session(channel, session_id).await?;
        let sent = session.send(message).await.map_err(session_error)?;
        Ok(match sent.disposition {
            SendDisposition::Steered => SendOutcome::Steered,
            _ => SendOutcome::Started {
                input_message_id: sent.message_id,
            },
        })
    }

    async fn events(
        &self,
        channel: &str,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<ChannelEventStream, ChannelError> {
        let session = self.session(channel, session_id).await?;
        let events = match after {
            None => session.events(),
            Some(after) => session
                .events_from(i32::try_from(after).unwrap_or(i32::MAX))
                .await
                .map_err(session_error)?,
        };
        // The stream holds the session so it outlives this call.
        let stream =
            futures::stream::unfold((session, events), |(session, mut events)| async move {
                loop {
                    match events.recv().await {
                        Ok(Some(event)) => {
                            if let Some(event) =
                                DeliveryEvent::from_envelope(event.canonical_json())
                            {
                                return Some((event, (session, events)));
                            }
                        }
                        // A lagging reader loses deltas only: the completed
                        // message carries the whole text.
                        Err(error) => debug!(%error, "channels: event stream lagged"),
                        Ok(None) => return None,
                    }
                }
            });
        Ok(stream.boxed())
    }
}

#[cfg(test)]
#[path = "channels_tests.rs"]
mod tests;
