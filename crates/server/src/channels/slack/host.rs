//! Slack on the shared channel host (knowledge/integrations/channels.md,
//! "Server on the host").
//!
//! The host owns intake: duplicate events, the conversation's session, the
//! turn, and its delivery with recovery. This module is what only Slack
//! knows: verifying and reading its events, whom it answers, a thread's
//! history, and how a turn's replies look in a channel thread or the agent
//! pane.
//!
//! Decisions:
//! - The binding keys are the routing tags Slack sessions carried before the
//!   host (`slack:thread:{ts}`, `slack:channel:{id}`, `slack:user:{id}`), so
//!   migration 217 binds every existing conversation under the key the host
//!   now looks up.
//! - The agent pane is one thread whatever the channel's binding says
//!   (EVE-1005), so the driver binds a pane conversation by thread.
//! - The relevance policy needs the Slack event, which the host's message
//!   does not carry. The driver puts the event's JSON in the message's
//!   routing metadata, which the host never persists.
//! - A turn's look depends on the turn: the pane streams, shows status and
//!   sets titles, a channel thread does not; approval cards bind to the
//!   person the turn answers and need the session's approval hint. The
//!   driver's [`TurnDeliveryPlanner`] reads those per turn.
//! - Slack wants an answer within three seconds. The events route verifies
//!   the request, acknowledges it, and hands it to the host in the background,
//!   so the relevance judge and session creation never hold the answer up.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::runtime::channel::{
    CHANNEL_MESSAGE_KEY, ChannelAgentSurface, ChannelDeliveryAdapter, ChannelError,
    ChannelReactionSurface, ChannelStreamDelivery, ChannelViewContext, ControlAction,
    DeliveryContext, DeliveryResult, DeliveryTarget, ExternalActor, Inbound, InboundAttachment,
    InboundChannelEvent, InboundControl, InboundMessage, InboundReaction, InboundRoute,
    OutboundChannelMessage, SessionBinding,
};
use everruns_contracts::runtime::channel_driver::{ChannelDriver, ChannelRequest, ChannelResponse};
use everruns_core::channel_runtime::{DeliveryOptions, TurnDeliveryPlanner};
use serde_json::{Value, json};

use super::approvals::approvals_enabled;
use super::delivery::{
    SLACK_RECIPIENT_TEAM_ID, SLACK_RECIPIENT_USER_ID, SlackSurface, classify_surface,
    emoji_for_slack_reaction, slack_turn_options, turn_adapter::SlackTurnAdapter,
};
use super::events::{
    SlackEvent, SlackState, build_attachment_content_parts, event_matches_slack_scope,
    is_supported_slack_message_subtype, parse_slack_inbound_event, should_process_message,
    should_skip_thread_reply, slack_message_metadata, truncation_notice,
};
use crate::channels::host::{HISTORY_NOTICE, THREAD_VIEW_CONTEXT};
use crate::domains::agent_channels::ingress::{IngressChannel, IngressContext};
use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use crate::domains::agent_channels::slack_evidence::{self, DeliveryEvidence};

pub(crate) const SLACK_PLATFORM: &str = "slack";

/// Routing metadata holding the Slack event's JSON, for the relevance policy.
const SLACK_EVENT: &str = "slack_event";
/// Routing metadata naming the surface: `pane` or `channel`.
const SLACK_SURFACE: &str = "slack_surface";
/// Delivery target extra naming the surface a turn answers in.
pub(crate) const TARGET_SURFACE: &str = "slack_surface";

/// The message metadata key holding a Slack message's thread.
pub(crate) const SLACK_THREAD_KEY: &str = "slack_thread_ts";
/// Input metadata: answer in the DM itself, not in a thread under the input.
pub(crate) const SLACK_REPLY_INLINE: &str = "slack_reply_inline";

/// One Slack channel row as a host driver.
pub(crate) struct SlackHostDriver {
    state: SlackState,
    context: IngressContext,
    channel: IngressChannel,
    config: SlackChannelConfig,
    files: everruns_integrations::slack_channel::Slack,
}

impl SlackHostDriver {
    pub(crate) fn new(
        state: SlackState,
        context: IngressContext,
        channel: IngressChannel,
        config: SlackChannelConfig,
    ) -> Self {
        let mut files = everruns_integrations::slack_channel::Slack::new(
            &config.bot_token,
            &config.signing_secret,
        )
        .installation(channel.public_id.to_string());
        if let Some(team) = &config.team_id {
            files = files.workspace(team);
        }
        Self {
            state,
            context,
            channel,
            config,
            files,
        }
    }

    fn surface(&self, channel_type: Option<&str>, channel: &str) -> SlackSurface {
        classify_surface(self.config.agent_surface_enabled, channel_type, channel)
    }

    async fn evidence(&self, evidence: DeliveryEvidence) {
        if let Err(error) = slack_evidence::record(
            &self.state.db,
            self.state.encryption.as_ref(),
            self.channel.internal_id,
            &self.channel.public_id.to_string(),
            &self.config.signing_secret,
            evidence,
        )
        .await
        {
            tracing::warn!(channel_id = %self.channel.public_id, %error, "Failed to record Slack delivery evidence");
        }
    }

    /// A pane event's conversation: the pane reports its thread under
    /// `assistant_thread`, not at the event root.
    fn pane_route(&self, event: &SlackEvent) -> Option<InboundRoute> {
        let thread = event.assistant_thread.as_ref()?;
        let channel = thread.channel_id.clone().or(event.channel.clone())?;
        let thread_ts = thread.thread_ts.clone().or(event.thread_ts.clone());
        let mut routing_metadata = HashMap::from([
            ("channel_id".to_string(), channel.clone()),
            (SLACK_SURFACE.to_string(), "pane".to_string()),
        ]);
        if let Some(user) = &event.user {
            routing_metadata.insert("user_id".into(), user.clone());
        }
        Some(InboundRoute {
            actor: ExternalActor {
                actor_id: event.user.clone().unwrap_or_default(),
                actor_name: None,
                source: SLACK_PLATFORM.into(),
                metadata: None,
            },
            thread_ref: Some(thread_ts.clone().unwrap_or_else(|| "unknown".into())),
            routing_metadata,
            reply_to: DeliveryTarget::new(channel, thread_ts.unwrap_or_default()),
        })
    }

    fn control(&self, event: &SlackEvent, action: ControlAction) -> Inbound {
        match self.pane_route(event) {
            Some(route) => Inbound::Control(Box::new(InboundControl { route, action })),
            None => Inbound::Ignore,
        }
    }

    /// A person's reaction on a Slack message. The host finds the message it
    /// linked to that key, or ignores it.
    async fn reaction(
        &self,
        event: SlackEvent,
        egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
    ) -> Inbound {
        let (Some(name), Some(item), Some(user)) = (&event.reaction, &event.item, &event.user)
        else {
            return Inbound::Ignore;
        };
        let (Some(channel), Some(ts)) = (&item.channel, &item.ts) else {
            return Inbound::Ignore;
        };
        if item.item_type.as_deref() != Some("message") {
            return Inbound::Ignore;
        }
        // THREAT[TM-SLACK-002]: the bot's own reactions are the agent's,
        // already stored; hearing them back would echo. When Slack cannot
        // say who the bot is, the reaction is dropped rather than risk it.
        let bot = super::provider::bot_user(
            &self.state,
            &self.channel.public_id.to_string(),
            &self.config,
            egress.clone(),
        )
        .await;
        if bot.is_none() || bot.as_deref() == Some(user.as_str()) {
            return Inbound::Ignore;
        }
        let actor_name = super::provider::user_name(
            &self.state,
            &self.channel.public_id.to_string(),
            &self.config,
            user,
            egress,
        )
        .await;
        Inbound::Reaction(Box::new(InboundReaction {
            message_key: format!("{channel}:{ts}"),
            emoji: emoji_for_slack_reaction(name),
            added: event.event_type == "reaction_added",
            actor: ExternalActor {
                actor_id: user.clone(),
                actor_name,
                source: SLACK_PLATFORM.into(),
                metadata: None,
            },
        }))
    }

    async fn message(
        &self,
        raw_event: &Value,
        event: SlackEvent,
        team: &str,
        egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
    ) -> Inbound {
        if event.bot_id.is_some() {
            // THREAT[TM-SLACK-002]: never answer a bot, ours included.
            return Inbound::Ignore;
        }
        if !is_supported_slack_message_subtype(event.subtype.as_deref())
            || !matches!(event.event_type.as_str(), "message" | "app_mention")
        {
            return Inbound::Ignore;
        }
        let text = event.text.clone().unwrap_or_default();
        if text.is_empty() && event.files.is_empty() && event.attachments.is_empty() {
            return Inbound::Ignore;
        }
        let channel = event.channel.clone().unwrap_or_default();
        let surface = self.surface(event.channel_type.as_deref(), &channel);
        // THREAT[TM-SLACK-006]: a shared or per-message binding stored before
        // writes rejected it would hand one person's session to everyone in
        // the channel. The pane is always a thread; elsewhere, fail closed.
        if surface == SlackSurface::Channel
            && !crate::domains::agent_channels::record::ChannelType::Slack
                .allows_binding(self.config.session_strategy)
        {
            tracing::warn!(
                channel_id = %self.channel.public_id,
                binding = ?self.config.session_strategy,
                "Ignoring Slack message: the channel's binding is not offered on Slack"
            );
            return Inbound::Ignore;
        }
        tracing::info!(
            channel_id = %self.channel.public_id,
            event_type = %event.event_type,
            slack_channel = ?event.channel,
            thread_ts = ?event.thread_ts,
            "Slack message received"
        );
        if self.config.first_message_received_at.is_none() {
            self.evidence(DeliveryEvidence::FirstMessage).await;
        }

        let user = event.user.clone().unwrap_or_default();
        let display_name = super::provider::user_name(
            &self.state,
            &self.channel.public_id.to_string(),
            &self.config,
            &user,
            egress,
        )
        .await;
        let mut inbound = parse_slack_inbound_event(
            &event,
            &self.config,
            display_name,
            &self.channel.public_id.to_string(),
            team,
        );
        // Link unfurls and app attachments have no attachment kind of their
        // own; their summaries read as part of what was posted.
        for part in build_attachment_content_parts(&event.attachments) {
            if let crate::api::messages::InputContentPart::Text(part) = part {
                inbound.text.push_str("\n\n");
                inbound.text.push_str(&part.text);
            }
        }
        inbound.message_metadata = slack_message_metadata(&self.context, &self.channel, &event)
            .into_iter()
            .collect();
        if let Some(ts) = &event.ts {
            // A reaction on this message names it the same way.
            inbound.message_metadata.insert(
                CHANNEL_MESSAGE_KEY.into(),
                Value::String(format!("{channel}:{ts}")),
            );
        }
        inbound
            .routing_metadata
            .insert(SLACK_EVENT.into(), raw_event.to_string());
        inbound
            .routing_metadata
            .insert(SLACK_SURFACE.into(), surface_name(surface).into());

        // A DM is already one conversation: answer a top-level DM message in
        // the DM itself rather than opening a thread under every question. A
        // message written inside a thread, a thread-bound session (whose key
        // is the thread) and the pane keep threading.
        let reply_inline = surface == SlackSurface::Channel
            && event.thread_ts.is_none()
            && self.config.session_strategy != SessionBinding::Thread
            && match event.channel_type.as_deref() {
                Some(kind) => kind == "im",
                None => channel.starts_with('D'),
            };
        if reply_inline {
            inbound
                .message_metadata
                .insert(SLACK_REPLY_INLINE.into(), Value::Bool(true));
        }
        let mut reply_to = DeliveryTarget::new(
            channel,
            match reply_inline {
                true => String::new(),
                false => event
                    .thread_ts
                    .clone()
                    .or(event.ts.clone())
                    .unwrap_or_default(),
            },
        );
        reply_to
            .extra
            .insert(TARGET_SURFACE.into(), surface_name(surface).into());
        if !user.is_empty() {
            reply_to.extra.insert(SLACK_RECIPIENT_USER_ID.into(), user);
        }
        reply_to
            .extra
            .insert(SLACK_RECIPIENT_TEAM_ID.into(), team.into());
        reply_to.extra.insert(
            everruns_integrations::slack_channel::bridge::TEAM_ID.into(),
            team.into(),
        );
        reply_to.extra.insert(
            everruns_integrations::slack_channel::bridge::INSTALLATION_ID.into(),
            self.channel.public_id.to_string(),
        );
        Inbound::Message(Box::new(InboundMessage {
            event: inbound,
            reply_to,
        }))
    }
}

fn surface_name(surface: SlackSurface) -> &'static str {
    match surface {
        SlackSurface::Pane => "pane",
        SlackSurface::Channel => "channel",
    }
}

fn slack_event(message: &InboundMessage) -> Option<SlackEvent> {
    serde_json::from_str(message.event.routing_metadata.get(SLACK_EVENT)?).ok()
}

#[async_trait]
impl ChannelDeliveryAdapter for SlackHostDriver {
    fn platform(&self) -> &str {
        SLACK_PLATFORM
    }

    fn requires_checkpoint(&self) -> bool {
        true
    }

    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        self.state.delivery_adapter.deliver(message, context).await
    }

    async fn send_ack(
        &self,
        thread_ref: &str,
        text: &str,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        self.state
            .delivery_adapter
            .send_ack(thread_ref, text, context)
            .await
    }

    fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
        self.state.delivery_adapter.streaming()
    }

    fn agent_surface(&self) -> Option<&dyn ChannelAgentSurface> {
        self.state.delivery_adapter.agent_surface()
    }

    fn reactions(&self) -> Option<&dyn ChannelReactionSurface> {
        self.state.delivery_adapter.reactions()
    }
}

#[async_trait]
impl ChannelDriver for SlackHostDriver {
    fn attachment_resolver(
        &self,
    ) -> Option<&dyn everruns_contracts::runtime::channel::ChannelAttachmentResolver> {
        Some(&self.files)
    }

    fn secrets(&self) -> Vec<String> {
        vec![
            "signing_secret".into(),
            "bot_token".into(),
            "app_token".into(),
        ]
    }

    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        self.receive_with_egress(
            request,
            super::provider::egress(&self.state, self.context.org_id),
        )
        .await
    }

    async fn receive_with_egress(
        &self,
        request: &ChannelRequest,
        egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
    ) -> Result<Inbound, ChannelError> {
        use everruns_integrations::slack_channel::ingress::VerifiedHttpIngress;
        let received = super::provider::receive(
            &self.channel.public_id.to_string(),
            &self.config,
            request,
            egress.clone(),
        )
        .await?;
        let (team, mut ingress) = match received {
            VerifiedHttpIngress::Challenge(challenge) => {
                if self.config.webhook_verified_at.is_none() {
                    self.evidence(DeliveryEvidence::WebhookVerified).await;
                }
                return Ok(Inbound::Respond(ChannelResponse::ok(
                    json!({"challenge": challenge}),
                )));
            }
            VerifiedHttpIngress::Events { team, ingress } => (team, ingress),
        };
        let Some(normalized) = ingress.ingress.events.drain(..).next() else {
            return Ok(Inbound::Ignore);
        };
        let Some((raw, event)) = super::provider::policy_event(normalized.kind, &ingress.context)
        else {
            return Ok(Inbound::Ignore);
        };
        if !event_matches_slack_scope(&self.config, Some(&team), &event) {
            return Ok(Inbound::Ignore);
        }
        Ok(match event.event_type.as_str() {
            // Where the user is looking (EVE-977). Kept on the session's
            // thread context, read at prompt assembly, never a message: the
            // pane reports on every navigation and only the latest matters.
            "app_context_changed" => {
                let thread = event.assistant_thread.as_ref();
                let view = thread
                    .and_then(|thread| thread.context.as_ref())
                    .map(|context| ChannelViewContext {
                        channel_id: context.channel_id.clone(),
                        team_id: context.team_id.clone(),
                        observed_at: Some(chrono::Utc::now()),
                    })
                    .unwrap_or_default();
                let thread_ref = thread
                    .and_then(|thread| thread.thread_ts.clone())
                    .or(event.thread_ts.clone());
                self.control(
                    &event,
                    ControlAction::Context {
                        key: THREAD_VIEW_CONTEXT.into(),
                        value: json!({
                            "thread_ref": thread_ref,
                            "platform": SLACK_PLATFORM,
                            "view": view,
                        }),
                    },
                )
            }
            // The stop button (EVE-976): cancels the running turn of this
            // channel's own session and nothing else. A finished turn is not
            // flipped, and the cancelled turn's delivery posts the notice.
            "agent_session_stopped" => self.control(&event, ControlAction::Cancel),
            // A rename in the pane is written back, so the agent's next
            // retitle does not silently revert it (EVE-975).
            "agent_session_title_changed" => {
                let title = event
                    .assistant_thread
                    .as_ref()
                    .and_then(|thread| thread.title.as_deref())
                    .or(event.title.as_deref())
                    .map(str::trim)
                    .filter(|title| !title.is_empty());
                match title {
                    Some(title) => self.control(
                        &event,
                        ControlAction::Rename {
                            title: title.to_string(),
                        },
                    ),
                    None => Inbound::Ignore,
                }
            }
            "app_home_opened" => Inbound::Ignore,
            "reaction_added" | "reaction_removed" => self.reaction(event, egress).await,
            _ => self.message(&raw, event, &team, egress).await,
        })
    }

    fn delivery_context(&self, target: &DeliveryTarget) -> DeliveryContext {
        let mut context = target.context(self.config.bot_token.clone());
        context.extra.remove(TARGET_SURFACE);
        context
            .extra
            .entry(everruns_integrations::slack_channel::bridge::INSTALLATION_ID.into())
            .or_insert_with(|| self.channel.public_id.to_string());
        if let Some(team) = &self.config.team_id {
            context
                .extra
                .entry(everruns_integrations::slack_channel::bridge::TEAM_ID.into())
                .or_insert_with(|| team.clone());
        }
        context
    }

    fn binding(&self, route: &InboundRoute, configured: SessionBinding) -> SessionBinding {
        match route
            .routing_metadata
            .get(SLACK_SURFACE)
            .map(String::as_str)
        {
            Some("pane") => SessionBinding::Thread,
            _ => configured,
        }
    }

    async fn admit(
        &self,
        message: &InboundMessage,
        session_id: Option<&str>,
    ) -> Result<bool, ChannelError> {
        let Some(event) = slack_event(message) else {
            return Ok(false);
        };
        let session = session_id.and_then(|id| id.parse().ok());
        Ok(should_process_message(
            &self.state,
            &self.context,
            &self.channel,
            &self.config,
            &event,
            session,
        )
        .await)
    }

    /// A thread the agent joins mid-way. Only for a thread-bound channel: a
    /// channel- or user-bound session outlives any one thread and keeps its
    /// own history, so seeding it would repeat a whole thread every time it
    /// touched a new one (EVE-969).
    async fn backfill(
        &self,
        message: &InboundMessage,
    ) -> Result<Vec<InboundChannelEvent>, ChannelError> {
        self.backfill_with_context(message, &self.delivery_context(&message.reply_to))
            .await
    }

    async fn backfill_with_context(
        &self,
        message: &InboundMessage,
        context: &DeliveryContext,
    ) -> Result<Vec<InboundChannelEvent>, ChannelError> {
        let Some(event) = slack_event(message) else {
            return Ok(Vec::new());
        };
        let (Some(_thread_ts), SessionBinding::Thread) =
            (event.thread_ts.as_deref(), self.config.session_strategy)
        else {
            return Ok(Vec::new());
        };
        let factory = everruns_integrations::slack_channel::bridge::direct_factory()
            .map_err(super::provider::error)?;
        let client = factory
            .for_context(context, super::events::SLACK_API_BASE)
            .await
            .map_err(super::provider::error)?;
        let target = everruns_integrations::slack_channel::bridge::target(&client, context)
            .map_err(super::provider::error)?;
        let backfill = super::events::fetch_thread_replies_client(&client, &target).await;
        let mut history = Vec::new();
        // Say up front that this is a window, not the whole thread.
        if backfill.is_truncated() && !backfill.messages.is_empty() {
            history.push(history_event(
                ExternalActor {
                    actor_id: String::new(),
                    actor_name: None,
                    source: SLACK_PLATFORM.into(),
                    metadata: None,
                },
                truncation_notice(&backfill),
                [(HISTORY_NOTICE.to_string(), Value::Bool(true))]
                    .into_iter()
                    .collect(),
            ));
        }
        for reply in &backfill.messages {
            if should_skip_thread_reply(reply, event.ts.as_deref()) {
                continue;
            }
            let user = reply.user.clone().unwrap_or_default();
            let actor_name = super::provider::user_name(
                &self.state,
                &self.channel.public_id.to_string(),
                &self.config,
                &user,
                context.egress.clone(),
            )
            .await;
            let metadata = reply
                .ts
                .iter()
                .map(|ts| ("slack_ts".to_string(), Value::String(ts.clone())))
                .collect();
            history.push(history_event(
                ExternalActor {
                    actor_id: user,
                    actor_name,
                    source: SLACK_PLATFORM.into(),
                    metadata: None,
                },
                reply.text.clone().unwrap_or_default(),
                metadata,
            ));
        }
        Ok(history)
    }
}

fn history_event(
    actor: ExternalActor,
    text: String,
    message_metadata: serde_json::Map<String, Value>,
) -> InboundChannelEvent {
    InboundChannelEvent {
        actor,
        text,
        attachments: Vec::<InboundAttachment>::new(),
        dedup_key: String::new(),
        thread_ref: None,
        routing_metadata: HashMap::new(),
        message_metadata,
    }
}

/// Plans each Slack turn: its surface, its approval cards, its session link.
pub(crate) struct SlackTurnPlanner {
    state: SlackState,
    org_id: i64,
    config: SlackChannelConfig,
}

impl SlackTurnPlanner {
    pub(crate) fn new(state: SlackState, org_id: i64, config: SlackChannelConfig) -> Self {
        Self {
            state,
            org_id,
            config,
        }
    }
}

#[async_trait]
impl TurnDeliveryPlanner for SlackTurnPlanner {
    async fn plan(
        &self,
        session_id: &str,
        target: &DeliveryTarget,
        _options: DeliveryOptions,
    ) -> (Arc<dyn ChannelDeliveryAdapter>, DeliveryOptions) {
        let surface = match target.extra.get(TARGET_SURFACE).map(String::as_str) {
            Some("pane") => SlackSurface::Pane,
            Some(_) => SlackSurface::Channel,
            None => classify_surface(self.config.agent_surface_enabled, None, &target.channel_id),
        };
        let session = session_id.parse().ok();
        // A session without the approval hint gets the ask in prose: the
        // documented degradation, not a gap (EVE-1025).
        let hints = match session {
            Some(session) => match self.state.db.get_session(self.org_id, session).await {
                Ok(row) => row.and_then(|row| row.hints),
                Err(error) => {
                    tracing::warn!(%session_id, %error, "Slack delivery: could not read the session's hints");
                    None
                }
            },
            None => None,
        };
        let adapter = Arc::new(SlackTurnAdapter {
            inner: self.state.delivery_adapter.clone(),
            approval_cards: approvals_enabled(hints.as_ref()),
            private_approvals: self.config.private_approvals,
            requester: target.extra.get(SLACK_RECIPIENT_USER_ID).cloned(),
            app_views: self.state.mcp_app_snapshots.clone().map(|snapshots| {
                crate::channels::slack::delivery::turn_adapter::SlackAppViews {
                    snapshots,
                    org_id: self.org_id,
                    session_link: session.and_then(|session| self.state.session_link(session)),
                }
            }),
        });
        let options = slack_turn_options(
            surface,
            &self.config,
            session.and_then(|session| self.state.session_link(session)),
        );
        (adapter, options)
    }
}
