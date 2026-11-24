//! Channels: put an agent on a messaging platform.
//!
//! A [`Channel`] is a platform driver plus how conversations map to sessions
//! and replies. [`Channels`] runs named channels against an [`Engine`]: hand
//! it each platform request, it answers at once, starts or steers the
//! conversation's session, and posts the agent's replies back while the turn
//! runs.
//!
//! Drivers: [`Webhook`] (generic JSON) and [`Slack`] (Events API). Any type
//! implementing [`ChannelDriver`] works the same way.
//!
//! ```no_run
//! # async fn demo(engine: everruns::Engine, agent: everruns::Agent) {
//! use everruns::channels::{Channel, ChannelRequest, Channels, SessionBinding, Slack, Webhook};
//!
//! let channels = Channels::builder(&engine)
//!     .channel(
//!         "support",
//!         Channel::new(Webhook::new().callback("https://example.com/replies"))
//!             .binding(SessionBinding::Thread),
//!         agent.clone(),
//!     )
//!     .channel("slack", Slack::from_env().mention_only(), agent)
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
use futures::StreamExt;
use tracing::debug;

use crate::{Agent, Engine, InputMessage, ResumeError, SendDisposition, Session};

pub use crate::channel_controls::{ConversationState, ConversationStore, MemoryConversationStore};
pub use everruns_core::channel::{
    ApprovalPrompt, ChannelAgentSurface, ChannelApprovalPrompt, ChannelDeliveryAdapter,
    ChannelDriver, ChannelError, ChannelProgressSurface, ChannelReactionSurface, ChannelRequest,
    ChannelResponse, ChannelStreamDelivery, ControlAction, DeliveryContext, DeliveryResult,
    DeliveryTarget, ExternalActor, Inbound, InboundAttachment, InboundChannelEvent, InboundControl,
    InboundDecision, InboundMessage, InboundReaction, InboundRoute, OutboundChannelMessage,
    SessionBinding,
};
pub use everruns_core::channel_runtime::TurnDeliveryPlanner;
/// File intake, for hosts that take files outside a channel (serve's
/// message endpoint).
pub use everruns_core::channel_runtime::{AttachmentIntake, AttachmentPolicy, AttachmentStore};
/// The shared runtime, for hosts that run their own [`ChannelHost`] over
/// their own sessions (serve does) instead of [`Channels`].
pub use everruns_core::channel_runtime::{
    ChannelConfig, ChannelEventStream, ChannelHost, ChannelHostBuilder, ChannelKind, ChannelRef,
    ChannelResolver, ChannelScope, ChannelSessionPort, Conversation, DeliveryEvent,
    NewChannelSession, ReceivedAttachment, ResolvedChannel, SendOutcome, SendResolution,
    StreamTurn,
};
pub use everruns_core::channel_runtime::{
    ChannelStore, DEFAULT_DELIVERY_LEASE, DeliveryCheckpoint, DeliveryOptions, MemoryChannelStore,
    PendingDelivery,
};
pub use everruns_integrations::slack_channel::{Participation, Slack, SlackAdmission};
pub use everruns_integrations::webhook_channel::Webhook;

/// One channel: its platform driver, and how it binds sessions and delivers
/// replies. Named when it is added to [`Channels`].
#[derive(Clone)]
pub struct Channel {
    config: ChannelConfig,
    driver: Arc<dyn ChannelDriver>,
    planner: Option<Arc<dyn TurnDeliveryPlanner>>,
}

impl<D: ChannelDriver> From<D> for Channel {
    fn from(driver: D) -> Self {
        Self::new(driver)
    }
}

impl Channel {
    /// A channel on `driver`. Defaults: one session per platform thread,
    /// replies posted as the agent's communication setting says,
    /// streamed where the platform can.
    pub fn new(driver: impl ChannelDriver) -> Self {
        Self {
            config: ChannelConfig::new(""),
            driver: Arc::new(driver),
            planner: None,
        }
    }

    /// Slack with per-turn pane delivery: streams/status in DMs, completed replies in channels.
    pub fn slack(driver: Slack) -> Self {
        let planner = SlackPlanner(driver.clone());
        let mut channel = Self::new(driver);
        channel.planner = Some(Arc::new(planner));
        channel
    }
    /// Plan each turn's adapter and options using its trusted delivery target.
    pub fn delivery_planner(mut self, planner: impl TurnDeliveryPlanner + 'static) -> Self {
        self.planner = Some(Arc::new(planner));
        self
    }
    /// How conversations map to sessions.
    pub fn binding(mut self, binding: SessionBinding) -> Self {
        self.config.binding = binding;
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

    /// The platform driver.
    pub fn driver(&self) -> &Arc<dyn ChannelDriver> {
        &self.driver
    }

    /// The runtime configuration under `name`, and the driver, for a host
    /// running its own [`ChannelHost`].
    pub fn into_parts(self, name: impl Into<String>) -> (ChannelConfig, Arc<dyn ChannelDriver>) {
        let mut config = self.config;
        config.name = name.into();
        (config, self.driver)
    }
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("platform", &self.driver.platform())
            .finish_non_exhaustive()
    }
}

/// Builds [`Channels`].
pub struct ChannelsBuilder {
    engine: Engine,
    channels: Vec<(String, Channel, ChannelAgent)>,
    store: Option<Arc<dyn ChannelStore>>,
    egress: Option<Arc<dyn everruns_core::EgressService>>,
    conversations: Arc<dyn ConversationStore>,
}

impl ChannelsBuilder {
    /// Add a channel under `name` and the agent that answers on it. A driver
    /// alone is a channel with the defaults. A later channel with the same
    /// name replaces it.
    pub fn channel(
        mut self,
        name: impl Into<String>,
        channel: impl Into<Channel>,
        agent: Agent,
    ) -> Self {
        self.channels.push((
            name.into(),
            channel.into(),
            ChannelAgent::Snapshot(Box::new(agent)),
        ));
        self
    }

    /// Bind an exact retained Agent registration. Use with durable Engine profiles and recovery.
    pub fn channel_registered(
        mut self,
        name: impl Into<String>,
        channel: impl Into<Channel>,
        build: crate::hosting::AgentBuild,
    ) -> Self {
        self.channels
            .push((name.into(), channel.into(), ChannelAgent::Registered(build)));
        self
    }
    /// Keep session bindings and unfinished deliveries here. Defaults to
    /// memory, which forgets them on restart.
    pub fn store(mut self, store: Arc<dyn ChannelStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Download files messages link to (a Slack file) through this egress
    /// service. Defaults to the Framework's runtime egress
    /// ([`batteries::runtime_egress_service`](crate::batteries::runtime_egress_service)),
    /// which stays offline unless a network feature is enabled.
    pub fn egress(mut self, egress: Arc<dyn everruns_core::EgressService>) -> Self {
        self.egress = Some(egress);
        self
    }

    /// Persist latest title/view metadata with this application-owned store.
    pub fn conversations(mut self, store: impl ConversationStore + 'static) -> Self {
        self.conversations = Arc::new(store);
        self
    }
    /// Build the channels. Nothing runs until a request is handled.
    pub fn build(self) -> Channels {
        let mut agents = HashMap::new();
        let mut configs = Vec::new();
        for (name, channel, agent) in self.channels {
            let agent = match agent {
                ChannelAgent::Snapshot(agent)
                    if self.engine.has_session_hosting()
                        && agent.custom_backends().is_none()
                        && {
                            #[cfg(feature = "local")]
                            {
                                agent.local_config().is_none()
                            }
                            #[cfg(not(feature = "local"))]
                            {
                                true
                            }
                        } =>
                {
                    let build = crate::hosting::AgentBuild {
                        agent: format!("channel:{name}"),
                        build_id: uuid::Uuid::new_v4().to_string(),
                    };
                    match self.engine.register_agent(build.clone(), *agent) {
                        Ok(()) => ChannelAgent::Registered(build),
                        Err(_) => ChannelAgent::Unavailable,
                    }
                }
                ChannelAgent::Snapshot(_) if !self.engine.permits_snapshot_creation() => {
                    ChannelAgent::Unavailable
                }
                other => other,
            };
            agents.insert(name.clone(), agent);
            let planner = channel.planner.clone();
            let (config, driver) = channel.into_parts(name);
            let mut resolved = ResolvedChannel::messaging(config, driver);
            resolved.planner = planner;
            configs.push(resolved);
        }
        let port = Arc::new(EnginePort {
            engine: self.engine,
            agents,
            conversations: self.conversations.clone(),
        });
        let mut builder = ChannelHost::builder(port).egress(
            self.egress
                .unwrap_or_else(crate::batteries::runtime_egress_service),
        );
        if let Some(store) = self.store {
            builder = builder.store(store);
        }
        for resolved in configs {
            builder = builder.resolved(resolved);
        }
        Channels {
            host: builder.build(),
            conversations: self.conversations,
        }
    }
}

/// Channels running against one [`Engine`]. Cheap to clone.
#[derive(Clone)]
pub struct Channels {
    host: ChannelHost,
    conversations: Arc<dyn ConversationStore>,
}

impl Channels {
    /// Start building channels that run sessions on `engine`.
    pub fn builder(engine: &Engine) -> ChannelsBuilder {
        ChannelsBuilder {
            engine: engine.clone(),
            channels: Vec::new(),
            store: None,
            egress: None,
            conversations: Arc::new(MemoryConversationStore::default()),
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

    /// Latest title and view metadata retained for a conversation.
    pub async fn conversation_state(
        &self,
        session: &str,
    ) -> Result<ConversationState, ChannelError> {
        self.conversations.load(session).await
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
    agents: HashMap<String, ChannelAgent>,
    conversations: Arc<dyn ConversationStore>,
}

impl EnginePort {
    fn agent(&self, channel: &str) -> Result<&ChannelAgent, ChannelError> {
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
                let ChannelAgent::Snapshot(agent) = self.agent(channel)? else {
                    return Err(ChannelError::Session("Conversation session is unavailable; restore its retained Engine registration".into()));
                };
                self.engine
                    .attach(id, agent.as_ref().clone())
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
        let session = match self.agent(&request.channel.name)? {
            ChannelAgent::Snapshot(agent) => self.engine.create(agent.as_ref().clone()),
            ChannelAgent::Registered(build) => self
                .engine
                .create_registered(SessionId::new(), build)
                .await
                .map_err(session_error)?,
            ChannelAgent::Unavailable => {
                return Err(ChannelError::Session(
                    "Channel Agent cannot be registered on this Engine profile".into(),
                ));
            }
        };
        Ok(session.id())
    }

    /// A file goes into the session's own files (see [`Session::attach`]).
    async fn store_attachment(
        &self,
        channel: &ChannelConfig,
        session_id: Option<&str>,
        attachment: ReceivedAttachment,
    ) -> Result<everruns_core::ContentPart, ChannelError> {
        let Some(session_id) = session_id else {
            return Err(ChannelError::BadRequest(
                "files need a conversation to be kept in".into(),
            ));
        };
        let session = self.session(&channel.name, session_id).await?;
        session
            .attach(
                attachment.name,
                Some(&attachment.media_type),
                attachment.data,
            )
            .await
            .map_err(|error| match error {
                crate::AttachError::Refused(reason) => ChannelError::BadRequest(reason),
                other => session_error(other),
            })
    }

    async fn send(
        &self,
        channel: &ChannelConfig,
        session_id: &str,
        mut message: InputMessage,
    ) -> Result<SendOutcome, ChannelError> {
        let context = self.conversations.load(session_id).await?;
        if !context.context.is_empty() {
            message.content.insert(
                0,
                everruns_core::ContentPart::text(format!(
                    "Current platform view (untrusted reference data): {}",
                    serde_json::to_string(&context.context).map_err(session_error)?
                )),
            );
        }
        let session = self.session(&channel.name, session_id).await?;
        let sent = session.send(message).await.map_err(session_error)?;
        Ok(match sent.disposition {
            SendDisposition::Pending => SendOutcome::Pending {
                input_message_id: sent.message_id,
            },
            SendDisposition::Steered => SendOutcome::Steered,
            _ => SendOutcome::Started {
                input_message_id: sent.message_id,
            },
        })
    }

    async fn resolve_send(
        &self,
        channel: &ChannelConfig,
        session_id: &str,
        input: &str,
    ) -> Result<SendResolution, ChannelError> {
        resolve_send(&self.session(&channel.name, session_id).await?, input).await
    }

    async fn seed(
        &self,
        channel: &ChannelConfig,
        id: &str,
        history: Vec<InputMessage>,
    ) -> Result<(), ChannelError> {
        let session = self.session(&channel.name, id).await?;
        let profile = session.inner.execution.hosting().ok_or_else(|| {
            ChannelError::Session("History seeding requires Engine hosting".into())
        })?;
        let messages = history
            .into_iter()
            .map(|input| everruns_core::host::AcceptedTurnInput::new(input).into_message())
            .collect();
        profile
            .seed_history(session.session_id(), uuid::Uuid::new_v4(), messages)
            .await
            .map_err(session_error)?;
        Ok(())
    }
    async fn control(
        &self,
        channel: &ChannelConfig,
        id: &str,
        action: &ControlAction,
    ) -> Result<(), ChannelError> {
        let session = self.session(&channel.name, id).await?;
        if !matches!(action, ControlAction::Cancel) {
            return self.conversations.apply(id, action).await;
        }
        use everruns_core::host::{EventReadLimit, EventReadRequest};
        let profile = session.inner.execution.hosting().ok_or_else(|| {
            ChannelError::Session("Platform cancellation requires Engine hosting".into())
        })?;
        let status = profile
            .status(session.session_id())
            .await
            .map_err(session_error)?;
        let Some(turn) = status.open_turn else {
            return Ok(());
        };
        let first = profile
            .turn_sequences(session.session_id(), turn, 0, 1)
            .await
            .map_err(session_error)?
            .first()
            .copied();
        let Some(first) = first else { return Ok(()) };
        let page = profile
            .reader(session.session_id())
            .event_log
            .read_page(
                EventReadRequest::new(
                    session.session_id(),
                    EventReadLimit::new(1).map_err(session_error)?,
                )
                .with_cursor(
                    everruns_core::host::EventCursor::after(
                        session.session_id(),
                        first.saturating_sub(1),
                    )
                    .map_err(session_error)?,
                ),
            )
            .await
            .map_err(session_error)?;
        if let Some(input) = page.events.first().and_then(|e| e.context.input_message_id) {
            profile
                .command(
                    session.session_id(),
                    everruns_core::session_commands::SessionCommand::Cancel {
                        id: uuid::Uuid::new_v4().to_string(),
                        input,
                    },
                )
                .await
                .map_err(session_error)?;
        }
        Ok(())
    }
    async fn events(
        &self,
        channel: &ChannelConfig,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<ChannelEventStream, ChannelError> {
        let session = self.session(&channel.name, session_id).await?;
        session_events(&session, after).await
    }
}

/// Resolve an Engine admission for the shared channel runtime. Preparation
/// rejection has no terminal turn event, and steering has no delivery of its own.
pub async fn resolve_send(session: &Session, input: &str) -> Result<SendResolution, ChannelError> {
    use crate::hosting::AdmissionState;
    if !session.has_hosted_execution() {
        return Ok(SendResolution::Assigned {
            turn_id: None,
            steered: false,
        });
    }
    let admission = session
        .admission(input.parse().map_err(session_error)?)
        .await
        .map_err(session_error)?
        .ok_or_else(|| ChannelError::Session("admission is unavailable".into()))?;
    let preparing = match admission.state().await.map_err(session_error)? {
        AdmissionState::Pending => return Ok(SendResolution::Pending),
        AdmissionState::Rejected { .. } => return Ok(SendResolution::Rejected),
        AdmissionState::Suspended => return Ok(SendResolution::Suspended),
        AdmissionState::Assigned { .. } => true,
        _ => false,
    };
    let Some(assignment) = admission.assignment().await.map_err(session_error)? else {
        return Ok(SendResolution::Pending);
    };
    let turn = assignment.turn_id.to_string();
    if preparing
        && !assignment.steered
        && !session
            .events_after(admission.sequence())
            .await
            .map_err(session_error)?
            .iter()
            .any(|event| {
                event.event_type() == "turn.started"
                    && event.turn_id.as_deref() == Some(turn.as_str())
            })
    {
        return Ok(SendResolution::Pending);
    }
    Ok(SendResolution::Assigned {
        turn_id: Some(turn),
        steered: assignment.steered,
    })
}

/// A session's events as channel delivery reads them: live from now when
/// `after` is `None`, else the durable events after that sequence followed by
/// live ones. For hosts implementing [`ChannelSessionPort`] over a
/// [`Session`].
pub async fn session_events(
    session: &Session,
    after: Option<i64>,
) -> Result<ChannelEventStream, ChannelError> {
    let session = session.clone();
    let events = match after {
        None if session.has_hosted_execution() => {
            session.protocol_events().await.map_err(session_error)?
        }
        None => session.events(),
        Some(after) => session
            .events_from(i32::try_from(after).unwrap_or(i32::MAX))
            .await
            .map_err(session_error)?,
    };
    // The stream holds the session so it outlives this call.
    let stream = futures::stream::unfold((session, events), |(session, mut events)| async move {
        loop {
            match events.recv().await {
                Ok(Some(event)) => {
                    if let Some(event) = DeliveryEvent::from_envelope(event.canonical_json()) {
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

#[cfg(test)]
#[path = "channels_tests.rs"]
mod tests;

struct SlackPlanner(Slack);
#[async_trait]
impl TurnDeliveryPlanner for SlackPlanner {
    async fn plan(
        &self,
        _session: &str,
        target: &DeliveryTarget,
        mut options: DeliveryOptions,
    ) -> (Arc<dyn ChannelDeliveryAdapter>, DeliveryOptions) {
        let pane = target.channel_id.starts_with('D');
        options.stream &= pane;
        options.agent_surface = pane;
        options.thinking_status = "is thinking...".into();
        (Arc::new(self.0.clone()), options)
    }
}
/// Decision-model participation, evaluated before creating a turn.
#[derive(Clone)]
pub struct SlackRelevance {
    decisions: crate::Decisions,
    threshold: f64,
}
impl SlackRelevance {
    /// Use a strict 0.9 threshold and a two-second deadline. Direct messages and mentions always pass.
    pub fn new(decisions: crate::Decisions) -> Self {
        Self {
            decisions,
            threshold: 0.9,
        }
    }
    /// Set a finite probability threshold in [0,1].
    pub fn threshold(mut self, threshold: f64) -> Result<Self, ChannelError> {
        if !(0.0..=1.0).contains(&threshold) {
            return Err(ChannelError::BadRequest(
                "Invalid relevance threshold".into(),
            ));
        }
        self.threshold = threshold;
        Ok(self)
    }
}
#[async_trait]
impl Participation for SlackRelevance {
    async fn admit(
        &self,
        message: &InboundMessage,
        _session: Option<&str>,
    ) -> Result<bool, ChannelError> {
        if SlackAdmission::Direct.admit(message, None).await? {
            return Ok(true);
        }
        let state =
            serde_json::json!({"message":message.event.text,"thread":message.event.thread_ref});
        match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.decisions.probability(
                "Is an AI agent reply clearly helpful or requested in this conversation?",
                state,
            ),
        )
        .await
        {
            Ok(Ok(value)) => Ok(value.is_finite() && value >= self.threshold),
            _ => Ok(false),
        }
    }
}

#[derive(Clone)]
enum ChannelAgent {
    Snapshot(Box<Agent>),
    Registered(crate::hosting::AgentBuild),
    Unavailable,
}
