//! The channel host: request in, response out.
//!
//! Decisions:
//! - The host never owns sessions. A host implements [`ChannelSessionPort`]
//!   (create, send, events) over whatever runs its agents: the Framework's
//!   engine, serve's host, the server's services. Same shape as the voice
//!   loop's `VoiceSessionPort`.
//! - The request is answered as soon as the message is accepted; replies are
//!   delivered by a task per turn. Platforms time out slow acknowledgements and
//!   then retry, which is why the duplicate check comes first.
//! - Events are subscribed before the message is sent, so the first deltas of
//!   a fast turn are never missed. A message that steers a running turn gets
//!   no delivery of its own: the running turn's delivery carries the answer.
//! - A pending delivery is saved at the turn's first durable event and cleared
//!   when it ends; [`ChannelHost::recover`] replays the rest after a restart.
//!   It is held under a lease the running delivery renews, so processes
//!   sharing a store recover each delivery once.
//! - Channels are built in or resolved per call. A host whose channels are
//!   rows plugs in a [`ChannelResolver`], or resolves and authorizes the
//!   request itself and passes the [`ResolvedChannel`]. The channel's
//!   [`ChannelScope`] carries the host's tenant data to its port.
//! - Platform intake hooks live on the driver (`admit`, `backfill`,
//!   `decided`) and in [`Inbound`] (control events, decisions), so a platform
//!   with a relevance policy, thread history or approval buttons needs no
//!   intake of its own.
//! - Streaming channels (AG-UI, A2A, voice) answer in the request itself, so
//!   they have no driver: they share the binding store and the session port
//!   through [`ChannelHost::conversation`] and [`ChannelHost::stream_turn`],
//!   and encode the turn's events in their own protocol.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::{Value, json};
use tracing::{debug, warn};

use super::{
    ChannelDeliveryAdapter, ChannelDriver, ChannelError, ChannelRequest, ChannelResponse,
    ChannelStore, ControlAction, DeliveryEvent, DeliveryOptions, DeliveryStep, DeliveryTarget,
    Inbound, InboundAttachment, InboundChannelEvent, InboundControl, InboundDecision,
    InboundMessage, InboundRoute, MemoryChannelStore, PendingDelivery, SessionBinding,
    TurnDelivery, build_session_routing_tag,
};
use everruns_contracts::runtime::events::TOOL_CALL_REQUESTED;
use everruns_contracts::runtime::message::ContentPart;
use everruns_contracts::runtime::message_retriever::InputMessage;
use everruns_contracts::typed_id::SessionId;

/// How often open streams are flushed. Measured against Slack (EVE-974):
/// appends take about 300ms, so a shorter interval only queues calls.
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

/// A stream of one session's events.
pub type ChannelEventStream = BoxStream<'static, DeliveryEvent>;

/// Host data a channel carries for its host's port: the server's tenant,
/// owner and agent for a channel row. Core never reads it.
#[derive(Clone, Default)]
pub struct ChannelScope(Option<Arc<dyn Any + Send + Sync>>);

impl ChannelScope {
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Self(Some(Arc::new(value)))
    }

    /// The scope as the host's type, when it is one.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.0.as_deref()?.downcast_ref()
    }
}

impl std::fmt::Debug for ChannelScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "ChannelScope(..)"
        } else {
            "ChannelScope(None)"
        })
    }
}

/// One channel: a name, the agent behind it, how sessions bind, how replies go.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// Stable and unique within the store: bindings and pending deliveries
    /// are kept under it. A host whose channels are rows uses the row's id.
    pub name: String,
    pub agent: Option<String>,
    pub binding: SessionBinding,
    pub delivery: DeliveryOptions,
    pub scope: ChannelScope,
}

impl ChannelConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            agent: None,
            binding: SessionBinding::Thread,
            delivery: DeliveryOptions::default(),
            scope: ChannelScope::default(),
        }
    }
}

/// How a channel answers.
#[derive(Clone)]
pub enum ChannelKind {
    /// A request is acknowledged and replies go out through the driver.
    Messaging(Arc<dyn ChannelDriver>),
    /// The request carries the answer (`ag-ui`, `a2a`, `api`, ...); the kind
    /// is recorded on the channel's sessions.
    Streaming(String),
}

/// A channel the host can run: its configuration and how it answers.
#[derive(Clone)]
pub struct ResolvedChannel {
    pub config: ChannelConfig,
    pub kind: ChannelKind,
}

impl ResolvedChannel {
    pub fn messaging(config: ChannelConfig, driver: Arc<dyn ChannelDriver>) -> Self {
        Self {
            config,
            kind: ChannelKind::Messaging(driver),
        }
    }

    pub fn streaming(config: ChannelConfig, kind: impl Into<String>) -> Self {
        Self {
            config,
            kind: ChannelKind::Streaming(kind.into()),
        }
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// The platform or streaming kind, recorded on the channel's sessions.
    pub fn platform(&self) -> &str {
        match &self.kind {
            ChannelKind::Messaging(driver) => driver.platform(),
            ChannelKind::Streaming(kind) => kind,
        }
    }

    /// The driver of a messaging channel.
    pub fn driver(&self) -> Option<&Arc<dyn ChannelDriver>> {
        match &self.kind {
            ChannelKind::Messaging(driver) => Some(driver),
            ChannelKind::Streaming(_) => None,
        }
    }

    // A channel of the other kind answers like a missing one: messaging and
    // streaming channels do not share routes.
    fn require_driver(&self) -> Result<Arc<dyn ChannelDriver>, ChannelError> {
        self.driver()
            .cloned()
            .ok_or_else(|| ChannelError::NotFound(format!("channel {}", self.name())))
    }

    fn require_streaming(&self) -> Result<(), ChannelError> {
        match self.kind {
            ChannelKind::Streaming(_) => Ok(()),
            ChannelKind::Messaging(_) => {
                Err(ChannelError::NotFound(format!("channel {}", self.name())))
            }
        }
    }
}

/// Finds channels the host was not built with, such as rows in a database.
#[async_trait]
pub trait ChannelResolver: Send + Sync {
    /// The channel named `name`. `Ok(None)` when it is gone for good (its
    /// pending deliveries are dropped); an error when it cannot be read now
    /// (they are kept for a later recovery).
    async fn resolve(&self, name: &str) -> Result<Option<ResolvedChannel>, ChannelError>;
}

/// A channel by name, or one the caller already resolved (a host that
/// authorized the request against the channel's row).
#[derive(Clone, Copy)]
pub enum ChannelRef<'a> {
    Name(&'a str),
    Resolved(&'a ResolvedChannel),
}

impl<'a> From<&'a str> for ChannelRef<'a> {
    fn from(name: &'a str) -> Self {
        Self::Name(name)
    }
}

impl<'a> From<&'a String> for ChannelRef<'a> {
    fn from(name: &'a String) -> Self {
        Self::Name(name)
    }
}

impl<'a> From<&'a ResolvedChannel> for ChannelRef<'a> {
    fn from(channel: &'a ResolvedChannel) -> Self {
        Self::Resolved(channel)
    }
}

/// What the host asks for when a conversation needs a new session.
#[derive(Debug, Clone)]
pub struct NewChannelSession {
    pub channel: ChannelConfig,
    /// The binding key the session will be found by; `None` for a one-off.
    pub binding_key: Option<String>,
    pub title: String,
    /// `{"channel": ..., "kind": ..., "binding_key": ...}`, for the host to
    /// keep with the session.
    pub metadata: Value,
}

/// What sending a message did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// A turn started for this input message.
    Started { input_message_id: String },
    /// The message steered the turn already running.
    Steered,
}

/// The session side of channels, implemented by each host. Every call names
/// the channel, so a host that lost its in-memory sessions (a restart) can
/// reopen one with the channel's agent, and a host with tenants reads them
/// from the channel's scope.
#[async_trait]
pub trait ChannelSessionPort: Send + Sync {
    /// Create a session for a channel's agent. Returns its id.
    async fn create_session(&self, request: NewChannelSession) -> Result<String, ChannelError>;

    /// Send a message: start a turn when the session is idle, steer otherwise.
    /// The sender is the message's external actor.
    async fn send(
        &self,
        channel: &ChannelConfig,
        session_id: &str,
        message: InputMessage,
    ) -> Result<SendOutcome, ChannelError>;

    /// The session's events: live from now when `after` is `None`, else the
    /// durable events after that sequence followed by live ones.
    async fn events(
        &self,
        channel: &ChannelConfig,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<ChannelEventStream, ChannelError>;

    /// Drop a session the host created and never used: it lost the race to
    /// bind its conversation. The default keeps it.
    async fn discard_session(
        &self,
        _channel: &ChannelConfig,
        _session_id: &str,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Add earlier messages of the conversation to a new session as history,
    /// without starting a turn. The default drops them.
    async fn seed(
        &self,
        _channel: &ChannelConfig,
        _session_id: &str,
        _history: Vec<InputMessage>,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Apply a platform's control event to a session. The default ignores it.
    async fn control(
        &self,
        _channel: &ChannelConfig,
        _session_id: &str,
        _action: &ControlAction,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Apply an approval answered on the platform, after checking that the
    /// session belongs to the channel and that the decider may decide.
    /// Returns what resuming did: a turn that continues reports `Started`.
    async fn decide(
        &self,
        _channel: &ChannelConfig,
        _decision: &InboundDecision,
    ) -> Result<SendOutcome, ChannelError> {
        Err(ChannelError::BadRequest(
            "this host does not take decisions from channels".into(),
        ))
    }
}

/// The session behind one conversation of a streaming channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub session_id: String,
    /// Whether this call created the session, so the caller can seed history
    /// a client sent with its first request.
    pub created: bool,
}

/// One turn of a streaming channel: what sending did, and the events to answer
/// with.
pub struct StreamTurn {
    pub outcome: SendOutcome,
    /// The turn's events, ending after its terminal event or the event that
    /// parks it on a person or the client. For a message that steered a
    /// running turn, the running turn's events.
    pub events: ChannelEventStream,
}

/// How long a process holds a pending delivery before another may take it
/// over. Renewed while the delivery runs.
pub const DEFAULT_DELIVERY_LEASE: Duration = Duration::from_secs(120);

struct Inner {
    port: Arc<dyn ChannelSessionPort>,
    store: Arc<dyn ChannelStore>,
    channels: HashMap<String, ResolvedChannel>,
    resolver: Option<Arc<dyn ChannelResolver>>,
    flush_interval: Duration,
    /// This process, as a holder of delivery leases.
    holder: String,
    lease: Duration,
}

/// Runs channels against one host's sessions. Cheap to clone.
#[derive(Clone)]
pub struct ChannelHost {
    inner: Arc<Inner>,
}

/// Builds a [`ChannelHost`].
pub struct ChannelHostBuilder {
    port: Arc<dyn ChannelSessionPort>,
    store: Option<Arc<dyn ChannelStore>>,
    channels: HashMap<String, ResolvedChannel>,
    resolver: Option<Arc<dyn ChannelResolver>>,
    flush_interval: Duration,
    lease: Duration,
}

impl ChannelHostBuilder {
    /// Persist bindings and pending deliveries here. Defaults to memory.
    pub fn store(mut self, store: Arc<dyn ChannelStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Add a channel. A later channel with the same name replaces it.
    pub fn channel(self, config: ChannelConfig, driver: Arc<dyn ChannelDriver>) -> Self {
        self.resolved(ResolvedChannel::messaging(config, driver))
    }

    /// Add a streaming channel of `kind` (`ag-ui`, `a2a`, `voice`, ...). A
    /// later one with the same name replaces it.
    pub fn stream_channel(self, config: ChannelConfig, kind: impl Into<String>) -> Self {
        self.resolved(ResolvedChannel::streaming(config, kind))
    }

    /// Add a channel of either kind. A later one with the same name replaces
    /// it.
    pub fn resolved(mut self, channel: ResolvedChannel) -> Self {
        self.channels.insert(channel.config.name.clone(), channel);
        self
    }

    /// Look up channels the host was not built with here, after the ones it
    /// was.
    pub fn resolver(mut self, resolver: Arc<dyn ChannelResolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// How often open streams are flushed.
    pub fn flush_interval(mut self, interval: Duration) -> Self {
        self.flush_interval = interval;
        self
    }

    /// How long a pending delivery is held before another process may
    /// recover it.
    pub fn delivery_lease(mut self, lease: Duration) -> Self {
        self.lease = lease;
        self
    }

    pub fn build(self) -> ChannelHost {
        ChannelHost {
            inner: Arc::new(Inner {
                port: self.port,
                store: self
                    .store
                    .unwrap_or_else(|| Arc::new(MemoryChannelStore::new())),
                channels: self.channels,
                resolver: self.resolver,
                flush_interval: self.flush_interval,
                holder: uuid::Uuid::now_v7().to_string(),
                lease: self.lease,
            }),
        }
    }
}

impl ChannelHost {
    pub fn builder(port: Arc<dyn ChannelSessionPort>) -> ChannelHostBuilder {
        ChannelHostBuilder {
            port,
            store: None,
            channels: HashMap::new(),
            resolver: None,
            flush_interval: DEFAULT_FLUSH_INTERVAL,
            lease: DEFAULT_DELIVERY_LEASE,
        }
    }

    /// Names of the messaging channels the host was built with, sorted.
    pub fn channel_names(&self) -> Vec<String> {
        self.names(|channel| channel.driver().is_some())
    }

    /// Names of the streaming channels the host was built with, sorted.
    pub fn stream_channel_names(&self) -> Vec<String> {
        self.names(|channel| channel.driver().is_none())
    }

    fn names(&self, keep: impl Fn(&ResolvedChannel) -> bool) -> Vec<String> {
        let mut names: Vec<String> = self
            .inner
            .channels
            .values()
            .filter(|channel| keep(channel))
            .map(|channel| channel.config.name.clone())
            .collect();
        names.sort();
        names
    }

    /// A built-in channel's configuration.
    pub fn config(&self, channel: &str) -> Option<&ChannelConfig> {
        self.inner.channels.get(channel).map(|entry| &entry.config)
    }

    /// A built-in messaging channel's driver.
    pub fn driver(&self, channel: &str) -> Option<Arc<dyn ChannelDriver>> {
        self.inner.channels.get(channel)?.driver().cloned()
    }

    /// The channel named `name`: a built-in one, else the resolver's.
    pub async fn resolve(&self, name: &str) -> Result<ResolvedChannel, ChannelError> {
        if let Some(channel) = self.inner.channels.get(name) {
            return Ok(channel.clone());
        }
        let found = match &self.inner.resolver {
            Some(resolver) => resolver.resolve(name).await?,
            None => None,
        };
        found.ok_or_else(|| ChannelError::NotFound(format!("channel {name}")))
    }

    async fn channel(&self, channel: ChannelRef<'_>) -> Result<ResolvedChannel, ChannelError> {
        match channel {
            ChannelRef::Name(name) => self.resolve(name).await,
            ChannelRef::Resolved(channel) => Ok(channel.clone()),
        }
    }

    /// Answer one request. Errors become responses: a channel request comes
    /// from outside and always gets an HTTP answer.
    pub async fn handle<'a>(
        &self,
        channel: impl Into<ChannelRef<'a>>,
        request: &ChannelRequest,
    ) -> ChannelResponse {
        let channel = channel.into();
        match self.try_handle(channel, request).await {
            Ok(response) => response,
            Err(error) => {
                let name = match channel {
                    ChannelRef::Name(name) => name,
                    ChannelRef::Resolved(channel) => channel.name(),
                };
                if error.status() >= 500 {
                    warn!(channel = name, %error, "channel request failed");
                } else {
                    debug!(channel = name, %error, "channel request refused");
                }
                error.to_response()
            }
        }
    }

    /// [`handle`](Self::handle), with the error kept.
    pub async fn try_handle<'a>(
        &self,
        channel: impl Into<ChannelRef<'a>>,
        request: &ChannelRequest,
    ) -> Result<ChannelResponse, ChannelError> {
        let channel = self.channel(channel.into()).await?;
        let driver = channel.require_driver()?;
        match driver.receive(request).await? {
            Inbound::Respond(response) => Ok(response),
            Inbound::Ignore => Ok(ChannelResponse::ignored()),
            Inbound::Message(message) => self.accept(&channel, &driver, *message).await,
            Inbound::Control(control) => self.control(&channel, &driver, *control).await,
            Inbound::Decision(decision) => self.decide(&channel, &driver, *decision).await,
        }
    }

    /// Start a conversation from this side: a new session whose replies go to
    /// `target`. Used by schedules and other proactive posts. Returns the
    /// session id.
    pub async fn start<'a>(
        &self,
        channel: impl Into<ChannelRef<'a>>,
        target: DeliveryTarget,
        text: &str,
    ) -> Result<String, ChannelError> {
        let channel = self.channel(channel.into()).await?;
        let driver = channel.require_driver()?;
        let session_id = self
            .inner
            .port
            .create_session(NewChannelSession {
                channel: channel.config.clone(),
                binding_key: None,
                title: title_for(text),
                metadata: json!({ "channel": channel.name(), "kind": channel.platform() }),
            })
            .await?;
        let message = InputMessage::user(text).with_metadata("channel", json!(channel.name()));
        self.send_and_deliver(&channel, &driver, &session_id, message, target)
            .await?;
        Ok(session_id)
    }

    /// Send `message` on an existing session of `channel` and deliver that
    /// turn's replies to `target`. For a host that picked the session itself
    /// (a scheduled run, an operator's post). A message that steers a running
    /// turn gets no delivery of its own.
    pub async fn send<'a>(
        &self,
        channel: impl Into<ChannelRef<'a>>,
        session_id: &str,
        message: InputMessage,
        target: DeliveryTarget,
    ) -> Result<SendOutcome, ChannelError> {
        let channel = self.channel(channel.into()).await?;
        let driver = channel.require_driver()?;
        let message = with_channel(message, channel.name());
        self.send_and_deliver(&channel, &driver, session_id, message, target)
            .await
    }

    /// The session behind conversation `key` of streaming channel `channel`,
    /// created and bound on first use. `title` names a new session.
    pub async fn conversation<'a>(
        &self,
        channel: impl Into<ChannelRef<'a>>,
        key: &str,
        title: &str,
    ) -> Result<Conversation, ChannelError> {
        let channel = self.channel(channel.into()).await?;
        channel.require_streaming()?;
        if let Some(session_id) = self.inner.store.session_for(channel.name(), key).await? {
            return Ok(Conversation {
                session_id,
                created: false,
            });
        }
        let created = self
            .inner
            .port
            .create_session(NewChannelSession {
                channel: channel.config.clone(),
                binding_key: Some(key.to_string()),
                title: title_for(title),
                metadata: json!({
                    "channel": channel.name(),
                    "kind": channel.platform(),
                    "binding_key": key,
                }),
            })
            .await?;
        self.bind_created(&channel, key, created).await
    }

    /// A new session of streaming channel `channel` that no conversation key
    /// finds again: a one-off exchange, or one the caller continues by the
    /// session id it hands out (an A2A context). `title` names it.
    pub async fn open<'a>(
        &self,
        channel: impl Into<ChannelRef<'a>>,
        title: &str,
    ) -> Result<String, ChannelError> {
        let channel = self.channel(channel.into()).await?;
        channel.require_streaming()?;
        self.inner
            .port
            .create_session(NewChannelSession {
                channel: channel.config.clone(),
                binding_key: None,
                title: title_for(title),
                metadata: json!({ "channel": channel.name(), "kind": channel.platform() }),
            })
            .await
    }

    /// The session already bound to conversation `key` of streaming channel
    /// `channel`, if any.
    pub async fn find_conversation<'a>(
        &self,
        channel: impl Into<ChannelRef<'a>>,
        key: &str,
    ) -> Result<Option<String>, ChannelError> {
        let channel = self.channel(channel.into()).await?;
        channel.require_streaming()?;
        self.inner.store.session_for(channel.name(), key).await
    }

    /// Send `message` on a session of streaming channel `channel` and return
    /// the turn's events. Subscribes before sending, so a fast turn's first
    /// deltas are not missed.
    pub async fn stream_turn<'a>(
        &self,
        channel: impl Into<ChannelRef<'a>>,
        session_id: &str,
        message: InputMessage,
    ) -> Result<StreamTurn, ChannelError> {
        let channel = self.channel(channel.into()).await?;
        channel.require_streaming()?;
        let message = with_channel(message, channel.name());
        let events = self
            .inner
            .port
            .events(&channel.config, session_id, None)
            .await?;
        let outcome = self
            .inner
            .port
            .send(&channel.config, session_id, message)
            .await?;
        let turn = match &outcome {
            SendOutcome::Started { input_message_id } => Some(input_message_id.clone()),
            SendOutcome::Steered => None,
        };
        Ok(StreamTurn {
            outcome,
            events: turn_events(events, turn),
        })
    }

    /// The events of whichever turn runs next on a session of streaming
    /// channel `channel`, without sending: for a caller that answered a
    /// question or an approval and resumes the parked turn, or that waits on
    /// a turn parked on someone else. `after` is the last durable sequence the
    /// caller saw, so nothing written since is missed; `None` follows from
    /// now. Ends like [`stream_turn`](Self::stream_turn).
    pub async fn follow<'a>(
        &self,
        channel: impl Into<ChannelRef<'a>>,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<ChannelEventStream, ChannelError> {
        let channel = self.channel(channel.into()).await?;
        channel.require_streaming()?;
        let events = self
            .inner
            .port
            .events(&channel.config, session_id, after)
            .await?;
        Ok(turn_events(events, None))
    }

    /// Resume every delivery a previous process left unfinished and no live
    /// process holds. Returns how many were resumed. Deliveries of channels
    /// gone for good are dropped; those of channels unavailable for now are
    /// given back for a later recovery.
    pub async fn recover(&self) -> Result<usize, ChannelError> {
        let mut resumed = 0;
        let claimed = self
            .inner
            .store
            .claim_pending(&self.inner.holder, self.inner.lease)
            .await?;
        for pending in claimed {
            let found = match self.inner.channels.get(&pending.channel) {
                Some(channel) => Ok(Some(channel.clone())),
                None => match &self.inner.resolver {
                    Some(resolver) => resolver.resolve(&pending.channel).await,
                    None => Ok(None),
                },
            };
            let channel = match found {
                Ok(Some(channel)) if channel.driver().is_some() => channel,
                Ok(_) => {
                    self.inner
                        .store
                        .clear_pending(&pending.session_id, &pending.input_message_id)
                        .await?;
                    continue;
                }
                Err(error) => {
                    warn!(channel = %pending.channel, %error, "channel: recovery deferred");
                    self.inner
                        .store
                        .release_pending(
                            &pending.session_id,
                            &pending.input_message_id,
                            &self.inner.holder,
                        )
                        .await?;
                    continue;
                }
            };
            let Some(driver) = channel.driver().cloned() else {
                continue;
            };
            let events = self
                .inner
                .port
                .events(
                    &channel.config,
                    &pending.session_id,
                    Some(pending.after_sequence),
                )
                .await?;
            self.spawn_delivery(
                channel,
                driver,
                pending.session_id,
                pending.input_message_id,
                pending.target,
                events,
                true,
            );
            resumed += 1;
        }
        Ok(resumed)
    }

    async fn accept(
        &self,
        channel: &ResolvedChannel,
        driver: &Arc<dyn ChannelDriver>,
        message: InboundMessage,
    ) -> Result<ChannelResponse, ChannelError> {
        let name = channel.name();
        let dedup_key = &message.event.dedup_key;
        if !dedup_key.is_empty() && !self.inner.store.first_sighting(name, dedup_key).await? {
            debug!(channel = %name, %dedup_key, "channel: dropped a platform retry");
            return Ok(ChannelResponse::ok(
                json!({ "ok": true, "duplicate": true }),
            ));
        }

        let key = binding_key(channel.platform(), channel.config.binding, &message.route());
        let bound = match &key {
            Some(key) => self.inner.store.session_for(name, key).await?,
            None => None,
        };
        if !driver.admit(&message, bound.as_deref()).await? {
            debug!(channel = %name, "channel: the driver declined a message");
            return Ok(ChannelResponse::ignored());
        }
        let (session_id, created) = match bound {
            Some(session_id) => (session_id, false),
            None => {
                let created = self.create(channel, key.clone(), &message.event).await?;
                let session_id = match &key {
                    Some(key) => self.bind_created(channel, key, created).await?,
                    None => Conversation {
                        session_id: created,
                        created: true,
                    },
                };
                if session_id.created {
                    self.backfill(channel, driver, &session_id.session_id, &message)
                        .await;
                }
                (session_id.session_id, session_id.created)
            }
        };

        let input = input_message(name, &message.event);
        self.send_and_deliver(channel, driver, &session_id, input, message.reply_to)
            .await?;
        let mut response = ChannelResponse::accepted(&session_id);
        response.body["created"] = json!(created);
        Ok(response)
    }

    /// Bind a session this host just created; when an earlier writer won the
    /// key, discard ours and use theirs.
    async fn bind_created(
        &self,
        channel: &ResolvedChannel,
        key: &str,
        created: String,
    ) -> Result<Conversation, ChannelError> {
        let bound = self.inner.store.bind(channel.name(), key, &created).await?;
        if bound == created {
            return Ok(Conversation {
                session_id: created,
                created: true,
            });
        }
        if let Err(error) = self
            .inner
            .port
            .discard_session(&channel.config, &created)
            .await
        {
            warn!(session_id = %created, %error, "channel: could not discard a session that lost its binding");
        }
        Ok(Conversation {
            session_id: bound,
            created: false,
        })
    }

    /// Seed a new session with the conversation's earlier messages. Best
    /// effort: a session without history still answers.
    async fn backfill(
        &self,
        channel: &ResolvedChannel,
        driver: &Arc<dyn ChannelDriver>,
        session_id: &str,
        message: &InboundMessage,
    ) {
        let history = match driver.backfill(message).await {
            Ok(history) if history.is_empty() => return,
            Ok(history) => history,
            Err(error) => {
                warn!(%session_id, %error, "channel: could not read a thread's history");
                return;
            }
        };
        let history = history
            .iter()
            .map(|event| input_message(channel.name(), event))
            .collect();
        if let Err(error) = self
            .inner
            .port
            .seed(&channel.config, session_id, history)
            .await
        {
            warn!(%session_id, %error, "channel: could not seed a thread's history");
        }
    }

    async fn control(
        &self,
        channel: &ResolvedChannel,
        driver: &Arc<dyn ChannelDriver>,
        control: InboundControl,
    ) -> Result<ChannelResponse, ChannelError> {
        let Some(key) = binding_key(driver.platform(), channel.config.binding, &control.route)
        else {
            return Ok(ChannelResponse::ignored());
        };
        let Some(session_id) = self.inner.store.session_for(channel.name(), &key).await? else {
            return Ok(ChannelResponse::ignored());
        };
        self.inner
            .port
            .control(&channel.config, &session_id, &control.action)
            .await?;
        Ok(ChannelResponse::accepted(&session_id))
    }

    async fn decide(
        &self,
        channel: &ResolvedChannel,
        driver: &Arc<dyn ChannelDriver>,
        decision: InboundDecision,
    ) -> Result<ChannelResponse, ChannelError> {
        // Subscribe first, as for a message: the resumed turn may be fast.
        let events = self
            .inner
            .port
            .events(&channel.config, &decision.session_id, None)
            .await?;
        let outcome = self.inner.port.decide(&channel.config, &decision).await?;
        driver
            .decided(&decision, &driver.delivery_context(&decision.reply_to))
            .await;
        if let SendOutcome::Started { input_message_id } = outcome {
            self.spawn_delivery(
                channel.clone(),
                driver.clone(),
                decision.session_id.clone(),
                input_message_id,
                decision.reply_to.clone(),
                events,
                false,
            );
        }
        Ok(ChannelResponse::accepted(&decision.session_id))
    }

    async fn create(
        &self,
        channel: &ResolvedChannel,
        key: Option<String>,
        event: &InboundChannelEvent,
    ) -> Result<String, ChannelError> {
        self.inner
            .port
            .create_session(NewChannelSession {
                channel: channel.config.clone(),
                title: title_for(&event.text),
                metadata: json!({
                    "channel": channel.name(),
                    "kind": channel.platform(),
                    "binding_key": key,
                }),
                binding_key: key,
            })
            .await
    }

    async fn send_and_deliver(
        &self,
        channel: &ResolvedChannel,
        driver: &Arc<dyn ChannelDriver>,
        session_id: &str,
        message: InputMessage,
        target: DeliveryTarget,
    ) -> Result<SendOutcome, ChannelError> {
        // Subscribe first: a fast turn's first deltas land before `send`
        // returns.
        let events = self
            .inner
            .port
            .events(&channel.config, session_id, None)
            .await?;
        let outcome = self
            .inner
            .port
            .send(&channel.config, session_id, message)
            .await?;
        if let SendOutcome::Started { input_message_id } = &outcome
            && channel.config.delivery.replies
        {
            self.spawn_delivery(
                channel.clone(),
                driver.clone(),
                session_id.to_string(),
                input_message_id.clone(),
                target,
                events,
                false,
            );
        }
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_delivery(
        &self,
        channel: ResolvedChannel,
        driver: Arc<dyn ChannelDriver>,
        session_id: String,
        input_message_id: String,
        target: DeliveryTarget,
        events: ChannelEventStream,
        pending_saved: bool,
    ) {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            run_delivery(
                inner,
                Delivery {
                    channel,
                    driver,
                    session_id,
                    input_message_id,
                    target,
                },
                events,
                pending_saved,
            )
            .await;
        });
    }
}

/// One turn's delivery: where it comes from and where it goes.
struct Delivery {
    channel: ResolvedChannel,
    driver: Arc<dyn ChannelDriver>,
    session_id: String,
    input_message_id: String,
    target: DeliveryTarget,
}

async fn run_delivery(
    inner: Arc<Inner>,
    turn: Delivery,
    mut events: ChannelEventStream,
    mut pending_saved: bool,
) {
    let Delivery {
        channel,
        driver,
        session_id,
        input_message_id,
        target,
    } = turn;
    let context = driver.delivery_context(&target);
    let adapter: Arc<dyn ChannelDeliveryAdapter> = driver.clone();
    let typed_session = session_id
        .parse::<SessionId>()
        .unwrap_or_else(|_| SessionId::new());
    let mut delivery = TurnDelivery::new(
        adapter,
        context,
        typed_session,
        input_message_id.clone(),
        channel.config.delivery.clone(),
    );
    let mut flush = tokio::time::interval(inner.flush_interval);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Renew well before the lease runs out, so a slow store round trip never
    // lets another process take a delivery this one is running.
    let mut renew = tokio::time::interval(inner.lease / 3);
    renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renew.tick().await;

    loop {
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else {
                    // The stream ended without a terminal event: the host is
                    // shutting down. Leave the pending delivery for recovery.
                    delivery.flush().await;
                    return;
                };
                if !pending_saved
                    && let Some(sequence) = event.sequence
                {
                    let pending = PendingDelivery {
                        channel: channel.config.name.clone(),
                        session_id: session_id.clone(),
                        input_message_id: input_message_id.clone(),
                        target: target.clone(),
                        after_sequence: sequence - 1,
                    };
                    if let Err(error) = inner
                        .store
                        .save_pending(&pending, &inner.holder, inner.lease)
                        .await
                    {
                        warn!(%session_id, %error, "channel: could not save a pending delivery");
                    }
                    pending_saved = true;
                }
                if let DeliveryStep::Finished(_) = delivery.observe(&event).await {
                    break;
                }
            }
            _ = flush.tick(), if delivery.has_pending_work() => delivery.flush().await,
            _ = renew.tick(), if pending_saved => {
                if let Err(error) = inner
                    .store
                    .renew_pending(&session_id, &input_message_id, &inner.holder, inner.lease)
                    .await
                {
                    warn!(%session_id, %error, "channel: could not renew a delivery lease");
                }
            }
        }
    }

    if let Err(error) = inner
        .store
        .clear_pending(&session_id, &input_message_id)
        .await
    {
        warn!(%session_id, %error, "channel: could not clear a pending delivery");
    }
}

/// `message`, tagged with the channel it came through unless it already is.
fn with_channel(message: InputMessage, channel: &str) -> InputMessage {
    match &message.metadata {
        Some(metadata) if metadata.contains_key("channel") => message,
        _ => message.with_metadata("channel", json!(channel)),
    }
}

/// One turn's events out of a session's stream: events of other turns are
/// dropped, events without a turn (ephemeral deltas) kept, and the stream ends
/// after the turn's terminal event or after the event that parks it on a
/// person or the client (`tool.call_requested`): a streaming caller answers
/// that in a later request. `turn` is `None` for a steered message: the
/// stream then follows whichever turn is running to its end.
fn turn_events(events: ChannelEventStream, turn: Option<String>) -> ChannelEventStream {
    futures::stream::unfold(Some((events, turn)), |state| async move {
        let (mut events, turn) = state?;
        loop {
            let event = events.next().await?;
            let ours = match (&turn, &event.input_message_id) {
                (Some(turn), Some(id)) => turn == id,
                _ => true,
            };
            if !ours {
                continue;
            }
            let next = if event.is_terminal() || event.event_type == TOOL_CALL_REQUESTED {
                None
            } else {
                Some((events, turn))
            };
            return Some((event, next));
        }
    })
    .boxed()
}

/// The key a message's session is found by, or `None` for a new session per
/// message.
fn binding_key(kind: &str, binding: SessionBinding, route: &InboundRoute) -> Option<String> {
    match binding {
        SessionBinding::Ephemeral => None,
        SessionBinding::Shared => Some(format!("{kind}:shared")),
        binding => {
            let mut metadata = route.routing_metadata.clone();
            if let Some(thread) = &route.thread_ref {
                metadata
                    .entry("thread_ref".into())
                    .or_insert_with(|| thread.clone());
            }
            metadata
                .entry("channel_id".into())
                .or_insert_with(|| route.reply_to.channel_id.clone());
            metadata
                .entry("user_id".into())
                .or_insert_with(|| route.actor.actor_id.clone());
            // A thread binding without a thread (a DM, a platform without
            // threads) falls back to the conversation.
            let binding =
                if binding == SessionBinding::Thread && !metadata.contains_key("thread_ref") {
                    SessionBinding::Conversation
                } else {
                    binding
                };
            build_session_routing_tag(kind, &binding, &metadata)
        }
    }
}

fn input_message(channel: &str, event: &InboundChannelEvent) -> InputMessage {
    let mut message = InputMessage::user(event.text.clone());
    for (key, value) in &event.message_metadata {
        message = message.with_metadata(key.clone(), value.clone());
    }
    let mut message = message
        .with_metadata("channel", json!(channel))
        .with_external_actor(event.actor.clone());
    for attachment in &event.attachments {
        match attachment {
            InboundAttachment::Image { url, .. } => {
                message.content.push(ContentPart::image_url(url.clone()))
            }
            InboundAttachment::FileDescription { name, mime_type } => {
                message.content.push(ContentPart::text(match mime_type {
                    Some(mime) => format!("[Attached file: {name} ({mime})]"),
                    None => format!("[Attached file: {name}]"),
                }))
            }
        }
    }
    message
}

fn title_for(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let title: String = line.chars().take(80).collect();
    if title.is_empty() {
        "Channel conversation".to_string()
    } else {
        title
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
