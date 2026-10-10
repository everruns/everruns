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
//! - Streaming channels (AG-UI, A2A, voice) answer in the request itself, so
//!   they have no driver: they share the binding store and the session port
//!   through [`ChannelHost::conversation`] and [`ChannelHost::stream_turn`],
//!   and encode the turn's events in their own protocol.

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
    ChannelStore, DeliveryEvent, DeliveryOptions, DeliveryStep, DeliveryTarget, Inbound,
    InboundAttachment, InboundChannelEvent, InboundMessage, MemoryChannelStore, PendingDelivery,
    SessionBinding, TurnDelivery, build_session_routing_tag,
};
use everruns_contracts::runtime::message::ContentPart;
use everruns_contracts::runtime::message_retriever::InputMessage;
use everruns_contracts::typed_id::SessionId;

/// How often open streams are flushed. Measured against Slack (EVE-974):
/// appends take about 300ms, so a shorter interval only queues calls.
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

/// A stream of one session's events.
pub type ChannelEventStream = BoxStream<'static, DeliveryEvent>;

/// What the host asks for when a conversation needs a new session.
#[derive(Debug, Clone)]
pub struct NewChannelSession {
    pub channel: String,
    /// The agent the channel fronts, when the host runs several.
    pub agent: Option<String>,
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

/// The session side of channels, implemented by each host.
#[async_trait]
pub trait ChannelSessionPort: Send + Sync {
    /// Create a session for a channel's agent. Returns its id.
    async fn create_session(&self, request: NewChannelSession) -> Result<String, ChannelError>;

    /// Send a message: start a turn when the session is idle, steer otherwise.
    ///
    /// `channel` names the channel the session belongs to, so a host that
    /// lost its in-memory sessions (a restart) can reopen this one with the
    /// channel's agent.
    async fn send(
        &self,
        channel: &str,
        session_id: &str,
        message: InputMessage,
    ) -> Result<SendOutcome, ChannelError>;

    /// The session's events: live from now when `after` is `None`, else the
    /// durable events after that sequence followed by live ones.
    async fn events(
        &self,
        channel: &str,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<ChannelEventStream, ChannelError>;
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
    /// The turn's events, ending after its terminal event. For a message that
    /// steered a running turn, the running turn's events.
    pub events: ChannelEventStream,
}

/// One channel: a name, the agent behind it, how sessions bind, how replies go.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    pub name: String,
    pub agent: Option<String>,
    pub binding: SessionBinding,
    pub delivery: DeliveryOptions,
}

impl ChannelConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            agent: None,
            binding: SessionBinding::Thread,
            delivery: DeliveryOptions::default(),
        }
    }
}

#[derive(Clone)]
struct Entry {
    config: ChannelConfig,
    driver: Arc<dyn ChannelDriver>,
}

/// A streaming channel: no driver, a kind recorded on its sessions.
#[derive(Clone)]
struct StreamEntry {
    config: ChannelConfig,
    kind: String,
}

struct Inner {
    port: Arc<dyn ChannelSessionPort>,
    store: Arc<dyn ChannelStore>,
    channels: HashMap<String, Entry>,
    streams: HashMap<String, StreamEntry>,
    flush_interval: Duration,
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
    channels: HashMap<String, Entry>,
    streams: HashMap<String, StreamEntry>,
    flush_interval: Duration,
}

impl ChannelHostBuilder {
    /// Persist bindings and pending deliveries here. Defaults to memory.
    pub fn store(mut self, store: Arc<dyn ChannelStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Add a channel. A later channel with the same name replaces it.
    pub fn channel(mut self, config: ChannelConfig, driver: Arc<dyn ChannelDriver>) -> Self {
        self.channels
            .insert(config.name.clone(), Entry { config, driver });
        self
    }

    /// Add a streaming channel of `kind` (`ag-ui`, `a2a`, `voice`, ...). A
    /// later one with the same name replaces it.
    pub fn stream_channel(mut self, config: ChannelConfig, kind: impl Into<String>) -> Self {
        self.streams.insert(
            config.name.clone(),
            StreamEntry {
                config,
                kind: kind.into(),
            },
        );
        self
    }

    /// How often open streams are flushed.
    pub fn flush_interval(mut self, interval: Duration) -> Self {
        self.flush_interval = interval;
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
                streams: self.streams,
                flush_interval: self.flush_interval,
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
            streams: HashMap::new(),
            flush_interval: DEFAULT_FLUSH_INTERVAL,
        }
    }

    /// Channel names, sorted.
    pub fn channel_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.inner.channels.keys().cloned().collect();
        names.sort();
        names
    }

    /// A channel's configuration.
    pub fn config(&self, channel: &str) -> Option<&ChannelConfig> {
        self.inner.channels.get(channel).map(|entry| &entry.config)
    }

    /// A channel's driver.
    pub fn driver(&self, channel: &str) -> Option<Arc<dyn ChannelDriver>> {
        self.inner
            .channels
            .get(channel)
            .map(|entry| entry.driver.clone())
    }

    /// Answer one request. Errors become responses: a channel request comes
    /// from outside and always gets an HTTP answer.
    pub async fn handle(&self, channel: &str, request: &ChannelRequest) -> ChannelResponse {
        match self.try_handle(channel, request).await {
            Ok(response) => response,
            Err(error) => {
                if error.status() >= 500 {
                    warn!(channel, %error, "channel request failed");
                } else {
                    debug!(channel, %error, "channel request refused");
                }
                error.to_response()
            }
        }
    }

    /// [`handle`](Self::handle), with the error kept.
    pub async fn try_handle(
        &self,
        channel: &str,
        request: &ChannelRequest,
    ) -> Result<ChannelResponse, ChannelError> {
        let entry = self.entry(channel)?;
        match entry.driver.receive(request).await? {
            Inbound::Respond(response) => Ok(response),
            Inbound::Ignore => Ok(ChannelResponse::ignored()),
            Inbound::Message(message) => self.accept(&entry, *message).await,
        }
    }

    /// Start a conversation from this side: a new session whose replies go to
    /// `target`. Used by schedules and other proactive posts. Returns the
    /// session id.
    pub async fn start(
        &self,
        channel: &str,
        target: DeliveryTarget,
        text: &str,
    ) -> Result<String, ChannelError> {
        let entry = self.entry(channel)?;
        let session_id = self
            .inner
            .port
            .create_session(NewChannelSession {
                channel: channel.to_string(),
                agent: entry.config.agent.clone(),
                binding_key: None,
                title: title_for(text),
                metadata: json!({ "channel": channel, "kind": entry.driver.platform() }),
            })
            .await?;
        let message = InputMessage::user(text).with_metadata("channel", json!(channel));
        self.send_and_deliver(&entry, &session_id, message, target)
            .await?;
        Ok(session_id)
    }

    /// Send `message` on an existing session of `channel` and deliver that
    /// turn's replies to `target`. For a host that picked the session itself
    /// (a scheduled run, an operator's post). A message that steers a running
    /// turn gets no delivery of its own.
    pub async fn send(
        &self,
        channel: &str,
        session_id: &str,
        message: InputMessage,
        target: DeliveryTarget,
    ) -> Result<SendOutcome, ChannelError> {
        let entry = self.entry(channel)?;
        let message = match &message.metadata {
            Some(metadata) if metadata.contains_key("channel") => message,
            _ => message.with_metadata("channel", json!(channel)),
        };
        self.send_and_deliver(&entry, session_id, message, target)
            .await
    }

    /// The session behind conversation `key` of streaming channel `channel`,
    /// created and bound on first use. `title` names a new session.
    pub async fn conversation(
        &self,
        channel: &str,
        key: &str,
        title: &str,
    ) -> Result<Conversation, ChannelError> {
        let entry = self.stream_entry(channel)?;
        if let Some(session_id) = self.inner.store.session_for(channel, key).await? {
            return Ok(Conversation {
                session_id,
                created: false,
            });
        }
        let session_id = self
            .inner
            .port
            .create_session(NewChannelSession {
                channel: channel.to_string(),
                agent: entry.config.agent.clone(),
                binding_key: Some(key.to_string()),
                title: title_for(title),
                metadata: json!({ "channel": channel, "kind": entry.kind, "binding_key": key }),
            })
            .await?;
        self.inner.store.bind(channel, key, &session_id).await?;
        Ok(Conversation {
            session_id,
            created: true,
        })
    }

    /// The session already bound to conversation `key` of streaming channel
    /// `channel`, if any.
    pub async fn find_conversation(
        &self,
        channel: &str,
        key: &str,
    ) -> Result<Option<String>, ChannelError> {
        self.stream_entry(channel)?;
        self.inner.store.session_for(channel, key).await
    }

    /// Send `message` on a session of streaming channel `channel` and return
    /// the turn's events. Subscribes before sending, so a fast turn's first
    /// deltas are not missed.
    pub async fn stream_turn(
        &self,
        channel: &str,
        session_id: &str,
        message: InputMessage,
    ) -> Result<StreamTurn, ChannelError> {
        self.stream_entry(channel)?;
        let message = match &message.metadata {
            Some(metadata) if metadata.contains_key("channel") => message,
            _ => message.with_metadata("channel", json!(channel)),
        };
        let events = self.inner.port.events(channel, session_id, None).await?;
        let outcome = self.inner.port.send(channel, session_id, message).await?;
        let turn = match &outcome {
            SendOutcome::Started { input_message_id } => Some(input_message_id.clone()),
            SendOutcome::Steered => None,
        };
        Ok(StreamTurn {
            outcome,
            events: turn_events(events, turn),
        })
    }

    /// Streaming channel names, sorted.
    pub fn stream_channel_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.inner.streams.keys().cloned().collect();
        names.sort();
        names
    }

    /// Resume every delivery a previous process left unfinished. Returns how
    /// many were resumed. Deliveries of channels no longer configured are
    /// dropped.
    pub async fn recover(&self) -> Result<usize, ChannelError> {
        let mut resumed = 0;
        for pending in self.inner.store.pending().await? {
            let Some(entry) = self.inner.channels.get(&pending.channel).cloned() else {
                self.inner
                    .store
                    .clear_pending(&pending.session_id, &pending.input_message_id)
                    .await?;
                continue;
            };
            let events = self
                .inner
                .port
                .events(
                    &pending.channel,
                    &pending.session_id,
                    Some(pending.after_sequence),
                )
                .await?;
            self.spawn_delivery(
                entry,
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

    fn entry(&self, channel: &str) -> Result<Entry, ChannelError> {
        self.inner
            .channels
            .get(channel)
            .cloned()
            .ok_or_else(|| ChannelError::NotFound(format!("channel {channel}")))
    }

    fn stream_entry(&self, channel: &str) -> Result<StreamEntry, ChannelError> {
        self.inner
            .streams
            .get(channel)
            .cloned()
            .ok_or_else(|| ChannelError::NotFound(format!("channel {channel}")))
    }

    async fn accept(
        &self,
        entry: &Entry,
        message: InboundMessage,
    ) -> Result<ChannelResponse, ChannelError> {
        let name = &entry.config.name;
        let InboundMessage { event, reply_to } = message;
        if !event.dedup_key.is_empty()
            && !self
                .inner
                .store
                .first_sighting(name, &event.dedup_key)
                .await?
        {
            debug!(channel = %name, dedup_key = %event.dedup_key, "channel: dropped a platform retry");
            return Ok(ChannelResponse::ok(
                json!({ "ok": true, "duplicate": true }),
            ));
        }

        let kind = entry.driver.platform().to_string();
        let key = binding_key(&kind, entry.config.binding, &event, &reply_to);
        let session_id = match &key {
            Some(key) => match self.inner.store.session_for(name, key).await? {
                Some(session_id) => session_id,
                None => {
                    let session_id = self.create(entry, Some(key.clone()), &event).await?;
                    self.inner.store.bind(name, key, &session_id).await?;
                    session_id
                }
            },
            None => self.create(entry, None, &event).await?,
        };

        let message = input_message(name, &event);
        self.send_and_deliver(entry, &session_id, message, reply_to)
            .await?;
        Ok(ChannelResponse::accepted(&session_id))
    }

    async fn create(
        &self,
        entry: &Entry,
        key: Option<String>,
        event: &InboundChannelEvent,
    ) -> Result<String, ChannelError> {
        self.inner
            .port
            .create_session(NewChannelSession {
                channel: entry.config.name.clone(),
                agent: entry.config.agent.clone(),
                title: title_for(&event.text),
                metadata: json!({
                    "channel": entry.config.name,
                    "kind": entry.driver.platform(),
                    "binding_key": key,
                }),
                binding_key: key,
            })
            .await
    }

    async fn send_and_deliver(
        &self,
        entry: &Entry,
        session_id: &str,
        message: InputMessage,
        target: DeliveryTarget,
    ) -> Result<SendOutcome, ChannelError> {
        // Subscribe first: a fast turn's first deltas land before `send`
        // returns.
        let channel = &entry.config.name;
        let events = self.inner.port.events(channel, session_id, None).await?;
        let outcome = self.inner.port.send(channel, session_id, message).await?;
        if let SendOutcome::Started { input_message_id } = &outcome {
            self.spawn_delivery(
                entry.clone(),
                session_id.to_string(),
                input_message_id.clone(),
                target,
                events,
                false,
            );
        }
        Ok(outcome)
    }

    fn spawn_delivery(
        &self,
        entry: Entry,
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
                entry,
                session_id,
                input_message_id,
                target,
                events,
                pending_saved,
            )
            .await;
        });
    }
}

async fn run_delivery(
    inner: Arc<Inner>,
    entry: Entry,
    session_id: String,
    input_message_id: String,
    target: DeliveryTarget,
    mut events: ChannelEventStream,
    mut pending_saved: bool,
) {
    let context = entry.driver.delivery_context(&target);
    let adapter: Arc<dyn ChannelDeliveryAdapter> = entry.driver.clone();
    let typed_session = session_id
        .parse::<SessionId>()
        .unwrap_or_else(|_| SessionId::new());
    let mut delivery = TurnDelivery::new(
        adapter,
        context,
        typed_session,
        input_message_id.clone(),
        entry.config.delivery.clone(),
    );
    let mut flush = tokio::time::interval(inner.flush_interval);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

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
                        channel: entry.config.name.clone(),
                        session_id: session_id.clone(),
                        input_message_id: input_message_id.clone(),
                        target: target.clone(),
                        after_sequence: sequence - 1,
                    };
                    if let Err(error) = inner.store.save_pending(&pending).await {
                        warn!(%session_id, %error, "channel: could not save a pending delivery");
                    }
                    pending_saved = true;
                }
                if let DeliveryStep::Finished(_) = delivery.observe(&event).await {
                    break;
                }
            }
            _ = flush.tick(), if delivery.has_pending_work() => delivery.flush().await,
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

/// One turn's events out of a session's stream: events of other turns are
/// dropped, events without a turn (ephemeral deltas) kept, and the stream ends
/// after the turn's terminal event. `turn` is `None` for a steered message:
/// the stream then follows whichever turn is running to its end.
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
            let next = if event.is_terminal() {
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
fn binding_key(
    kind: &str,
    binding: SessionBinding,
    event: &InboundChannelEvent,
    reply_to: &DeliveryTarget,
) -> Option<String> {
    match binding {
        SessionBinding::Ephemeral => None,
        SessionBinding::Shared => Some(format!("{kind}:shared")),
        binding => {
            let mut metadata = event.routing_metadata.clone();
            if let Some(thread) = &event.thread_ref {
                metadata
                    .entry("thread_ref".into())
                    .or_insert_with(|| thread.clone());
            }
            metadata
                .entry("channel_id".into())
                .or_insert_with(|| reply_to.channel_id.clone());
            metadata
                .entry("user_id".into())
                .or_insert_with(|| event.actor.actor_id.clone());
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
    let mut message = InputMessage::user(event.text.clone())
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
