// Slack delivery dispatcher — event-driven Slack message posting
//
// Design Decision: Replaces the 120s polling deadline in wait_and_post_response()
// with an event-driven approach. Subscribes to EventNotificationBroadcaster and
// dispatches Slack messages as output.message.completed events arrive.
//
// Design Decision: No durable queue for the Slack HTTP call. Events are already
// durably stored — if the server restarts, startup recovery re-registers active
// Slack sessions and replays from the last known event. Retry on transient Slack
// API failures uses simple exponential backoff in-process.
//
// Design Decision: Delivery context is keyed by (session_id, input_message_id)
// to support concurrent turns in the same session.

use async_trait::async_trait;
use everruns_core::channel::{
    ChannelDeliveryAdapter, ChannelStreamDelivery, DeliveryContext as ChannelDeliveryContext,
    DeliveryResult as ChannelDeliveryResult, OutboundChannelMessage,
};
use everruns_core::progress_reporting::{
    ProgressReportPayload, REPORT_PROGRESS_TOOL_NAME, format_progress_report_for_slack,
};
use everruns_platform::SlackReplyMode;
use everruns_provider::typed_id::{EventId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::event_notifications::EventNotificationPayload;
use crate::services::run_summary::is_terminal_turn_event;
use crate::storage::StorageBackend;

/// Which Slack surface a turn belongs to.
///
/// One app serves both at once: enabling Slack's Agents feature adds an assistant
/// container, it does not replace the channel bot. Which surface an event belongs
/// to is therefore read from the event at runtime, not from config (EVE-973).
/// Delivery style branches on this in the tickets that follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlackSurface {
    /// An ordinary channel or group thread — the classic channel bot.
    #[default]
    Channel,
    /// The agent pane: a DM to an app with the agent surface enabled.
    Pane,
}

/// Pane or channel, from the only two signals Slack gives us.
///
/// A live event carries `channel_type` (`"im"` for the agent pane). Recovery has
/// no event to read — only the channel id stored in the turn's metadata — and
/// Slack DM channel ids start with `D`. Both paths go through here so the two
/// cannot drift into disagreeing about the same session.
///
/// With the surface disabled the app is a channel bot everywhere, DMs included,
/// which is exactly how it behaves today.
pub fn classify_surface(
    agent_surface_enabled: bool,
    channel_type: Option<&str>,
    channel_id: &str,
) -> SlackSurface {
    if !agent_surface_enabled {
        return SlackSurface::Channel;
    }

    let is_dm = match channel_type {
        Some(kind) => kind == "im",
        None => channel_id.starts_with('D'),
    };

    if is_dm {
        SlackSurface::Pane
    } else {
        SlackSurface::Channel
    }
}

/// How often open streams are flushed to the platform.
///
/// Measured against a real workspace rather than guessed (EVE-974). Appending
/// serially, Slack sustained 300 appends in 86s — 209/min — with no rate-limit
/// push-back at all, so the documented "Tier 2: 20+ per minute" is a floor and
/// not the ceiling. The real limiter is call latency: a median `chat.appendStream`
/// took 291ms, so a flush interval below ~300ms cannot actually deliver more, it
/// just queues behind calls already in flight.
///
/// 500ms therefore sits above measured call latency and yields 120/min — a ~1.7x
/// margin under the rate we sustained without push-back.
const STREAM_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Flush early once this much text is waiting, so a fast burst surfaces without
/// waiting out the timer. Well under Slack's 12,000-character append cap.
const STREAM_FLUSH_CHARS: usize = 2_000;

/// An open stream for one output message.
///
/// Deltas are not accumulated locally: `output.message.delta` carries the full
/// text so far in `accumulated`, so a dropped notification self-heals on the next
/// one rather than leaving a permanent hole in the reply.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamState {
    /// Platform handle — Slack's stream `ts`.
    handle: String,
    /// Full text seen so far.
    accumulated: String,
    /// How many bytes of `accumulated` have reached the platform. Always a
    /// previous `accumulated.len()`, so slicing at it stays on a char boundary.
    sent: usize,
}

impl StreamState {
    /// Text produced since the last flush.
    fn pending(&self) -> &str {
        &self.accumulated[self.sent.min(self.accumulated.len())..]
    }
}

/// Everything needed to start watching one turn's delivery.
///
/// A struct rather than a parameter list: the turn's identity, its Slack
/// destination, and its delivery style are three different things, and passing
/// seven positional arguments made them easy to transpose.
pub struct DeliveryRegistration {
    pub session_id: Uuid,
    pub input_message_id: String,
    pub bot_token: String,
    pub channel: String,
    pub thread_ts: String,
    pub reply_mode: SlackReplyMode,
    pub surface: SlackSurface,
    /// Slack user and team the reply is for. `chat.startStream` requires both
    /// when streaming into a channel (EVE-974).
    pub recipient_user_id: Option<String>,
    pub recipient_team_id: Option<String>,
}

/// Context needed to deliver Slack messages for a turn.
#[derive(Debug, Clone)]
struct DeliveryContext {
    bot_token: String,
    channel: String,
    thread_ts: String,
    input_message_id: String,
    reply_mode: SlackReplyMode,
    /// Surface this turn arrived on. Carried so delivery can branch on it.
    surface: SlackSurface,
    /// Recipient identity, required by `chat.startStream`.
    recipient_user_id: Option<String>,
    recipient_team_id: Option<String>,
    /// Open streams for this turn, keyed by output message id. A turn with three
    /// output messages is three streams, not one concatenated blob.
    streams: HashMap<String, StreamState>,
    /// Last event ID we've processed (for cursor-based pagination).
    since_event_id: Option<EventId>,
    /// Whether a reply has already reached Slack for this turn. Terminal states
    /// only announce themselves when the user got nothing (EVE-966).
    delivered: bool,
}

/// Key for delivery context — a turn within a session.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct DeliveryKey {
    session_id: Uuid,
    input_message_id: String,
}

/// Event-driven Slack delivery dispatcher.
///
/// Subscribes to the event notification broadcaster and delivers agent output
/// messages to Slack as they arrive, with no fixed deadline.
pub struct SlackDeliveryDispatcher {
    /// Active deliveries: (session_id, input_message_id) → context
    deliveries: Arc<RwLock<HashMap<DeliveryKey, DeliveryContext>>>,
    /// Set of session_ids that have at least one active delivery (for fast filtering)
    active_sessions: Arc<RwLock<std::collections::HashSet<Uuid>>>,
    db: Arc<StorageBackend>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// UI base used to build the session link carried by terminal notices.
    /// Empty when unconfigured, in which case notices ship without a link.
    frontend_url: String,
    /// Platform delivery adapter. Every outbound message goes through it, so the
    /// path a second platform inherits is the one Slack actually exercises.
    adapter: Arc<dyn ChannelDeliveryAdapter>,
}

impl SlackDeliveryDispatcher {
    /// Create and start the dispatcher.
    ///
    /// `event_rx` is a broadcast receiver from EventNotificationBroadcaster.
    /// The dispatcher runs a background task that listens for event notifications.
    pub fn start(
        db: Arc<StorageBackend>,
        event_rx: broadcast::Receiver<EventNotificationPayload>,
        frontend_url: String,
    ) -> Arc<Self> {
        Self::start_with_adapter(
            db,
            event_rx,
            frontend_url,
            Arc::new(SlackDeliveryAdapter::new()),
        )
    }

    /// `start`, with the Slack API base injected. Tests point this at a mock.
    pub fn start_with_adapter(
        db: Arc<StorageBackend>,
        event_rx: broadcast::Receiver<EventNotificationPayload>,
        frontend_url: String,
        adapter: Arc<dyn ChannelDeliveryAdapter>,
    ) -> Arc<Self> {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let dispatcher = Arc::new(Self {
            deliveries: Arc::new(RwLock::new(HashMap::new())),
            active_sessions: Arc::new(RwLock::new(std::collections::HashSet::new())),
            db,
            shutdown_tx,
            frontend_url,
            adapter,
        });

        // Spawn the event processing loop
        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            dispatcher_clone.event_loop(event_rx, shutdown_rx).await;
        });

        dispatcher
    }

    /// Register a new delivery for a session turn.
    ///
    /// Called when a Slack message creates a new agent turn. The dispatcher will
    /// watch for output events and post them to Slack.
    pub async fn register(&self, registration: DeliveryRegistration) {
        let DeliveryRegistration {
            session_id,
            input_message_id,
            bot_token,
            channel,
            thread_ts,
            reply_mode,
            surface,
            recipient_user_id,
            recipient_team_id,
        } = registration;

        let key = DeliveryKey {
            session_id,
            input_message_id: input_message_id.clone(),
        };

        let ctx = DeliveryContext {
            bot_token,
            channel,
            thread_ts,
            input_message_id,
            reply_mode,
            surface,
            recipient_user_id,
            recipient_team_id,
            streams: HashMap::new(),
            since_event_id: None,
            delivered: false,
        };

        info!(
            %session_id,
            input_message_id = %ctx.input_message_id,
            ?surface,
            "Registered Slack delivery"
        );

        self.active_sessions.write().await.insert(session_id);
        self.deliveries.write().await.insert(key, ctx);
    }

    /// Shutdown the dispatcher.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Main event loop: listens for event notifications and processes them.
    async fn event_loop(
        &self,
        mut event_rx: broadcast::Receiver<EventNotificationPayload>,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        info!("Slack delivery dispatcher started");

        // Streaming needs a cadence the notification stream cannot provide: deltas
        // arrive per token, and `chat.appendStream` will not take them at that rate.
        // The loop therefore selects over {notification, flush tick} rather than
        // notifications alone (EVE-974).
        let mut flush = tokio::time::interval(STREAM_FLUSH_INTERVAL);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = flush.tick() => {
                    self.flush_open_streams().await;
                }
                result = event_rx.recv() => {
                    match result {
                        Ok(payload) => {
                            // Fast path: skip if no deliveries for this session
                            if !self.active_sessions.read().await.contains(&payload.session_id) {
                                continue;
                            }
                            self.process_session_events(payload.session_id).await;
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(
                                skipped = n,
                                "Slack delivery dispatcher lagged, processing all active sessions"
                            );
                            // On lag, process all active sessions to catch up
                            let sessions: Vec<Uuid> =
                                self.active_sessions.read().await.iter().copied().collect();
                            for session_id in sessions {
                                self.process_session_events(session_id).await;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("Event broadcaster closed, stopping Slack delivery dispatcher");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!("Slack delivery dispatcher shutting down");
                    break;
                }
            }
        }
    }

    /// Process new events for a session, delivering messages to Slack.
    async fn process_session_events(&self, session_id: Uuid) {
        let session_id_typed = SessionId::from_uuid(session_id);
        let empty: Vec<String> = vec![];

        // Collect keys for this session
        let keys: Vec<DeliveryKey> = {
            let deliveries = self.deliveries.read().await;
            deliveries
                .keys()
                .filter(|k| k.session_id == session_id)
                .cloned()
                .collect()
        };

        for key in keys {
            // Get current context (clone to release lock)
            let ctx = {
                let deliveries = self.deliveries.read().await;
                match deliveries.get(&key) {
                    Some(ctx) => ctx.clone(),
                    None => continue,
                }
            };

            // Fetch events since last processed
            let events = match self
                .db
                .list_events(
                    session_id_typed,
                    None,
                    ctx.since_event_id,
                    &empty,
                    &empty,
                    None,
                    None,
                )
                .await
            {
                Ok(events) => events,
                Err(e) => {
                    error!(%session_id, error = %e, "Failed to list events for Slack delivery");
                    continue;
                }
            };

            let mut new_since_id = ctx.since_event_id;
            let mut delivered = ctx.delivered;
            let mut terminal_event: Option<String> = None;
            let streams_supported = self.streaming_for(&ctx).is_some();

            for event in &events {
                new_since_id = Some(event.id);

                // Only consider events for our turn.
                //
                // `turn.cancelled` is the exception. Both cancel paths mint a fresh
                // `input_message_id` for the synthetic event because neither knows the
                // in-flight turn's id, so a per-turn match would never fire — which is
                // exactly the registration leak EVE-966 describes. Cancellation is
                // session-scoped anyway (`cancel_run` takes a session), so every
                // delivery on this session is terminal once it arrives.
                let event_input_msg = event
                    .context
                    .get("input_message_id")
                    .and_then(|v| v.as_str());
                let is_our_turn = event_input_msg == Some(&ctx.input_message_id)
                    || event.event_type == "turn.cancelled";

                if !is_our_turn {
                    continue;
                }

                // Progressive delivery for the pane. Deltas never reach the
                // discrete path below: they are accumulated per output message and
                // flushed on a cadence Slack can absorb.
                if streams_supported && ctx.reply_mode == SlackReplyMode::AllMessages {
                    if event.event_type == "output.message.delta" {
                        if let (Some(message_id), Some(accumulated)) = (
                            event.data.get("message_id").and_then(|v| v.as_str()),
                            event.data.get("accumulated").and_then(|v| v.as_str()),
                        ) && self.record_delta(&key, &ctx, message_id, accumulated).await
                        {
                            // Flush early on a burst so a long answer does not sit
                            // behind the timer.
                            let burst = self
                                .deliveries
                                .read()
                                .await
                                .get(&key)
                                .and_then(|c| c.streams.get(message_id))
                                .is_some_and(|s| s.pending().chars().count() >= STREAM_FLUSH_CHARS);
                            if burst {
                                self.flush_stream(&key, &ctx, message_id).await;
                            }
                        }
                        continue;
                    }

                    // A streamed message is finished by closing its stream, not by
                    // posting it again.
                    if event.event_type == "output.message.completed"
                        && let Some(message_id) = event
                            .data
                            .get("message")
                            .and_then(|m| m.get("id"))
                            .and_then(|v| v.as_str())
                    {
                        // The completed event is authoritative for the final text.
                        // Closing on the last delta alone truncates the reply by
                        // whatever arrived after it — and the last chunk is exactly
                        // what tends to arrive between the final delta and
                        // completion.
                        let final_text = extract_response_text(&event.data);
                        let streamed = match final_text {
                            Some(text) => self.set_accumulated(&key, message_id, text).await,
                            None => self
                                .open_stream_ids(&key)
                                .await
                                .iter()
                                .any(|id| id == message_id),
                        };
                        if streamed {
                            self.close_stream(&key, &ctx, message_id).await;
                            delivered = true;
                            continue;
                        }
                    }
                }

                // Post output messages to Slack
                if let Some(text) =
                    extract_delivery_text(&event.event_type, ctx.reply_mode, &event.data)
                {
                    // In report-progress-only mode the only text that reaches here
                    // is a progress report; in all-messages mode it is the answer.
                    let is_progress_report = ctx.reply_mode == SlackReplyMode::ReportProgressOnly;
                    match self.post(&ctx, session_id, text, is_progress_report).await {
                        // Only a reply Slack accepted counts as delivered. A send that
                        // exhausted its retries or hit a permanent error leaves this
                        // false, so the notice below tells the user the answer was
                        // produced and lost rather than leaving the thread silent.
                        ChannelDeliveryResult::Ok => delivered = true,
                        ChannelDeliveryResult::TransientError(e)
                        | ChannelDeliveryResult::PermanentError(e) => error!(
                            %session_id,
                            error = %e,
                            "Failed to post message to Slack after retries"
                        ),
                    }
                }

                // Stop watching when turn ends
                if is_terminal_turn_event(&event.event_type) {
                    debug!(
                        %session_id,
                        event_type = %event.event_type,
                        "Turn ended, unregistering Slack delivery"
                    );
                    terminal_event = Some(event.event_type.clone());
                    break;
                }
            }

            // Every terminal state stops the stream. An unstopped stream is a
            // message left spinning in the client forever, which is strictly worse
            // than the silence EVE-966 fixed.
            if terminal_event.is_some() {
                for message_id in self.open_stream_ids(&key).await {
                    self.close_stream(&key, &ctx, &message_id).await;
                    delivered = true;
                }
            }

            // Update cursor or unregister
            if let Some(event_type) = terminal_event {
                // A turn that ended without a delivered reply is silence in the Slack
                // thread. Post exactly one status line so the user knows the request
                // is over, and unregister either way.
                if !delivered {
                    let notice = self.terminal_notice(&event_type, session_id);
                    match self.post(&ctx, session_id, notice, false).await {
                        ChannelDeliveryResult::Ok => {}
                        ChannelDeliveryResult::TransientError(e)
                        | ChannelDeliveryResult::PermanentError(e) => warn!(
                            %session_id,
                            event_type = %event_type,
                            error = %e,
                            "Failed to post terminal-state notice to Slack"
                        ),
                    }
                }
                self.unregister(&key).await;
            } else if new_since_id != ctx.since_event_id || delivered != ctx.delivered {
                // Update the cursor
                // Stream state lives in the shared map and is never written back
                // from a clone, so only the cursor and delivered flag move here.
                let mut deliveries = self.deliveries.write().await;
                if let Some(live) = deliveries.get_mut(&key) {
                    live.since_event_id = new_since_id;
                    live.delivered = delivered;
                }
            }
        }
    }

    /// The streaming interface to use for this delivery, if any.
    ///
    /// Streaming is a pane behaviour: token-by-token into a shared channel is not
    /// wanted, and a platform without streaming returns `None` here (EVE-973/974).
    fn streaming_for(&self, ctx: &DeliveryContext) -> Option<&dyn ChannelStreamDelivery> {
        if ctx.surface != SlackSurface::Pane {
            return None;
        }
        self.adapter.streaming()
    }

    /// Record the text produced so far for one output message, opening its stream
    /// on first sight. Returns false when no stream could be opened, so the caller
    /// falls back to a discrete reply.
    async fn record_delta(
        &self,
        key: &DeliveryKey,
        ctx: &DeliveryContext,
        message_id: &str,
        accumulated: &str,
    ) -> bool {
        {
            let mut deliveries = self.deliveries.write().await;
            let Some(live) = deliveries.get_mut(key) else {
                return false;
            };
            if let Some(state) = live.streams.get_mut(message_id) {
                state.accumulated = accumulated.to_string();
                return true;
            }
        }

        let Some(stream) = self.streaming_for(ctx) else {
            return false;
        };
        let handle = match stream.start(&self.delivery_context(ctx)).await {
            Ok(handle) => handle,
            Err(e) => {
                warn!(error = %e, "Could not start Slack stream, falling back to a discrete reply");
                return false;
            }
        };

        let mut deliveries = self.deliveries.write().await;
        let Some(live) = deliveries.get_mut(key) else {
            return false;
        };
        live.streams
            .entry(message_id.to_string())
            .or_insert(StreamState {
                handle,
                accumulated: accumulated.to_string(),
                sent: 0,
            })
            .accumulated = accumulated.to_string();
        true
    }

    /// Replace the accumulated text for an open stream, if it is still open.
    async fn set_accumulated(&self, key: &DeliveryKey, message_id: &str, text: String) -> bool {
        let mut deliveries = self.deliveries.write().await;
        match deliveries
            .get_mut(key)
            .and_then(|c| c.streams.get_mut(message_id))
        {
            Some(state) => {
                state.accumulated = text;
                true
            }
            None => false,
        }
    }

    /// Output messages with a stream still open on this delivery.
    async fn open_stream_ids(&self, key: &DeliveryKey) -> Vec<String> {
        self.deliveries
            .read()
            .await
            .get(key)
            .map(|ctx| ctx.streams.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Take the text waiting on one stream, advancing `sent` under the lock.
    ///
    /// Claiming before the network call is what makes concurrent flushes safe:
    /// two flushers cannot both read the same `sent` and transmit the same bytes.
    /// Returns the handle, the claimed text, and the offset to restore if the send
    /// fails.
    async fn claim_pending(
        &self,
        key: &DeliveryKey,
        message_id: &str,
    ) -> Option<(String, String, usize)> {
        let mut deliveries = self.deliveries.write().await;
        let state = deliveries.get_mut(key)?.streams.get_mut(message_id)?;

        let pending = state.pending().to_string();
        if pending.is_empty() {
            return None;
        }

        let claimed_from = state.sent;
        state.sent = state.accumulated.len();
        Some((state.handle.clone(), pending, claimed_from))
    }

    /// Give a claim back after a failed send, so the text is retried rather than
    /// silently dropped from the middle of a reply.
    async fn release_claim(&self, key: &DeliveryKey, message_id: &str, claimed_from: usize) {
        let mut deliveries = self.deliveries.write().await;
        if let Some(ctx) = deliveries.get_mut(key)
            && let Some(state) = ctx.streams.get_mut(message_id)
        {
            state.sent = state.sent.min(claimed_from);
        }
    }

    /// Send whatever has accumulated on one stream since the last flush.
    async fn flush_stream(&self, key: &DeliveryKey, ctx: &DeliveryContext, message_id: &str) {
        let Some(stream) = self.streaming_for(ctx) else {
            return;
        };
        let Some((handle, pending, claimed_from)) = self.claim_pending(key, message_id).await
        else {
            return;
        };

        if let ChannelDeliveryResult::TransientError(e) | ChannelDeliveryResult::PermanentError(e) =
            stream
                .append(&handle, &pending, &self.delivery_context(ctx))
                .await
        {
            warn!(error = %e, "Failed to append to Slack stream");
            self.release_claim(key, message_id, claimed_from).await;
        }
    }

    /// Flush the tail and close the stream, then forget it.
    ///
    /// Always closes, even when the append failed — a stream left open spins in
    /// the client forever, which is worse than a truncated reply.
    async fn close_stream(&self, key: &DeliveryKey, ctx: &DeliveryContext, message_id: &str) {
        self.flush_stream(key, ctx, message_id).await;

        let handle = {
            let mut deliveries = self.deliveries.write().await;
            match deliveries
                .get_mut(key)
                .and_then(|c| c.streams.remove(message_id))
            {
                Some(state) => state.handle,
                None => return,
            }
        };

        let Some(stream) = self.streaming_for(ctx) else {
            return;
        };
        if let ChannelDeliveryResult::TransientError(e) | ChannelDeliveryResult::PermanentError(e) =
            stream.stop(&handle, &self.delivery_context(ctx)).await
        {
            warn!(error = %e, "Failed to stop Slack stream");
        }
    }

    /// Flush every open stream across every delivery. Driven by the timer.
    async fn flush_open_streams(&self) {
        let pending: Vec<(DeliveryKey, DeliveryContext, Vec<String>)> = {
            let deliveries = self.deliveries.read().await;
            deliveries
                .iter()
                .filter_map(|(key, ctx)| {
                    let ids: Vec<String> = ctx
                        .streams
                        .iter()
                        .filter(|(_, s)| !s.pending().is_empty())
                        .map(|(id, _)| id.clone())
                        .collect();
                    (!ids.is_empty()).then(|| (key.clone(), ctx.clone(), ids))
                })
                .collect()
        };

        for (key, ctx, message_ids) in pending {
            for message_id in message_ids {
                self.flush_stream(&key, &ctx, &message_id).await;
            }
        }
    }

    /// Post one message through the platform adapter.
    ///
    /// The dispatcher deliberately does not interpret the failure variant.
    /// Transient-vs-permanent is the adapter's judgement (EVE-972); the only
    /// thing the dispatcher decides is whether the user saw the message.
    async fn post(
        &self,
        ctx: &DeliveryContext,
        session_id: Uuid,
        text: String,
        is_progress_report: bool,
    ) -> ChannelDeliveryResult {
        let message = OutboundChannelMessage {
            session_id: SessionId::from_uuid(session_id),
            text,
            thread_ref: ctx.thread_ts.clone(),
            is_progress_report,
        };
        self.adapter
            .deliver(&message, &self.delivery_context(ctx))
            .await
    }

    /// The platform-facing view of a delivery.
    fn delivery_context(&self, ctx: &DeliveryContext) -> ChannelDeliveryContext {
        let mut extra = HashMap::new();
        if let Some(user) = &ctx.recipient_user_id {
            extra.insert(SLACK_RECIPIENT_USER_ID.to_string(), user.clone());
        }
        if let Some(team) = &ctx.recipient_team_id {
            extra.insert(SLACK_RECIPIENT_TEAM_ID.to_string(), team.clone());
        }

        ChannelDeliveryContext {
            auth_token: ctx.bot_token.clone(),
            channel_id: ctx.channel.clone(),
            thread_ref: ctx.thread_ts.clone(),
            reply_mode: ctx.reply_mode.into(),
            extra,
        }
    }

    /// One terse status line for a turn that ended without a reply.
    ///
    /// Deliberately carries no error detail: Slack channels are frequently public
    /// and the failure text is server-internal. The session link is the escape
    /// hatch for anyone who needs the real reason.
    fn terminal_notice(&self, event_type: &str, session_id: Uuid) -> String {
        let headline = match event_type {
            "turn.failed" => "The agent could not finish this request.",
            "turn.cancelled" => "This request was cancelled.",
            _ => "The agent finished without a reply.",
        };

        match self.session_link(session_id) {
            Some(link) => format!("{headline} <{link}|View the session>"),
            None => headline.to_string(),
        }
    }

    /// Absolute UI link to the session, when a frontend URL is configured.
    fn session_link(&self, session_id: Uuid) -> Option<String> {
        let base = self.frontend_url.trim_end_matches('/');
        if base.is_empty() {
            return None;
        }
        Some(format!(
            "{base}/sessions/{}/chat",
            SessionId::from_uuid(session_id)
        ))
    }

    /// Remove a delivery registration.
    async fn unregister(&self, key: &DeliveryKey) {
        let session_id = key.session_id;
        self.deliveries.write().await.remove(key);

        // Check if any other deliveries exist for this session
        let has_others = self
            .deliveries
            .read()
            .await
            .keys()
            .any(|k| k.session_id == session_id);

        if !has_others {
            self.active_sessions.write().await.remove(&session_id);
        }

        info!(
            %session_id,
            input_message_id = %key.input_message_id,
            "Unregistered Slack delivery"
        );
    }

    /// Get the number of active deliveries (for monitoring).
    pub async fn active_delivery_count(&self) -> usize {
        self.deliveries.read().await.len()
    }

    /// Recover active Slack deliveries after server restart.
    ///
    /// Finds app-owned sessions in 'active' status for Slack apps, looks up
    /// the corresponding app for bot_token, and re-registers deliveries for
    /// any turns that haven't completed yet.
    pub async fn recover(
        &self,
        encryption: Option<&std::sync::Arc<crate::storage::EncryptionService>>,
    ) {
        let db = self.db.as_ref();
        let sessions = match self.db.find_active_slack_sessions().await {
            Ok(sessions) => sessions,
            Err(e) => {
                error!(error = %e, "Failed to query active Slack sessions for recovery");
                return;
            }
        };

        if sessions.is_empty() {
            debug!("No active Slack sessions to recover");
            return;
        }

        info!(count = sessions.len(), "Recovering active Slack sessions");

        for session in &sessions {
            let app_internal_id = match session.app_id {
                Some(app_id) => app_id,
                None => continue,
            };

            // Look up the app through the server-owned session FK.
            // This keeps recovery independent of mutable routing tags.
            let app = match crate::domains::apps::queries::get_by_internal_id(
                db,
                encryption,
                session.org_id,
                app_internal_id,
            )
            .await
            {
                Ok(Some(app)) => app,
                Ok(None) => {
                    warn!(
                        app_id = %app_internal_id,
                        session_id = %session.id,
                        org_id = session.org_id,
                        "App not found in session org during Slack recovery"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(app_id = %app_internal_id, error = %e, "Failed to load app for Slack recovery");
                    continue;
                }
            };

            let slack_channel = match app.slack_channel() {
                Some(ch) => ch,
                None => continue,
            };
            let slack_config = match slack_channel.slack_config() {
                Some(cfg) => cfg,
                None => continue,
            };

            // Find the last input.message event to determine delivery context
            let empty: Vec<String> = vec![];
            let filter_types = vec!["input.message".to_string()];
            let events = match self
                .db
                .list_events(session.id, None, None, &filter_types, &empty, None, None)
                .await
            {
                Ok(events) => events,
                Err(e) => {
                    warn!(
                        session_id = %session.id,
                        error = %e,
                        "Failed to list events for Slack recovery"
                    );
                    continue;
                }
            };

            // Get the last input message
            let last_input = match events.last() {
                Some(e) => e,
                None => continue,
            };

            let input_message_id = last_input
                .data
                .get("message")
                .and_then(|m| m.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            if input_message_id.is_empty() {
                continue;
            }

            // Extract channel and thread_ts from message metadata
            let metadata = last_input
                .data
                .get("message")
                .and_then(|m| m.get("metadata"));

            let channel = metadata
                .and_then(|m| m.get("slack_channel"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let thread_ts = metadata
                .and_then(|m| m.get("slack_ts"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            // Persisted at input.message time so a restart can still name the
            // stream recipient (EVE-974). Absent on turns registered before that
            // field existed, which simply means no streaming for those.
            let recipient_user_id = metadata
                .and_then(|m| m.get("slack_user"))
                .and_then(|v| v.as_str())
                .map(str::to_string);

            // Check if this turn already completed
            let turn_events = vec!["turn.completed".to_string(), "turn.failed".to_string()];
            let completion_events = self
                .db
                .list_events(
                    session.id,
                    None,
                    Some(last_input.id),
                    &turn_events,
                    &empty,
                    None,
                    None,
                )
                .await
                .unwrap_or_default();

            let turn_done = completion_events.iter().any(|e| {
                e.context.get("input_message_id").and_then(|v| v.as_str())
                    == Some(&input_message_id)
            });

            if turn_done {
                debug!(
                    session_id = %session.id,
                    "Slack session turn already completed, skipping recovery"
                );
                continue;
            }

            if channel.is_empty() {
                warn!(
                    session_id = %session.id,
                    "Missing channel in Slack session recovery metadata"
                );
                continue;
            }

            info!(
                session_id = %session.id,
                input_message_id = %input_message_id,
                "Recovering Slack delivery"
            );

            // No event to read on this path, so the channel id is the only signal.
            let surface = classify_surface(slack_config.agent_surface_enabled, None, &channel);

            self.register(DeliveryRegistration {
                session_id: session.id.uuid(),
                input_message_id,
                bot_token: slack_config.bot_token.clone(),
                channel,
                thread_ts,
                reply_mode: slack_config.reply_mode,
                surface,
                recipient_user_id: recipient_user_id.filter(|u| !u.is_empty()),
                recipient_team_id: slack_config.team_id.clone(),
            })
            .await;
        }

        let count = self.deliveries.read().await.len();
        info!(recovered = count, "Slack delivery recovery complete");
    }
}

// ============================================
// ChannelDeliveryAdapter implementation for Slack
// ============================================

/// Slack implementation of the generic `ChannelDeliveryAdapter` trait.
///
/// Translates `OutboundChannelMessage` into Slack `chat.postMessage` API calls.
/// Used by the `SlackDeliveryDispatcher` and available for future generic
/// delivery dispatchers.
pub struct SlackDeliveryAdapter {
    /// Slack API base. Tests point this at a mock server.
    api_base: String,
}

impl SlackDeliveryAdapter {
    pub fn new() -> Self {
        Self {
            api_base: SLACK_API_BASE.to_string(),
        }
    }

    /// Adapter aimed at a different Slack API base — used by tests.
    pub fn with_api_base(api_base: String) -> Self {
        Self { api_base }
    }
}

impl Default for SlackDeliveryAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// Slack API error codes that retrying cannot fix.
///
/// The single source of truth for transient-vs-permanent (EVE-972), matched as
/// exact codes rather than substrings of a formatted message (EVE-968).
const PERMANENT_SLACK_ERRORS: &[&str] = &[
    "channel_not_found",
    "not_authed",
    "invalid_auth",
    "token_revoked",
    "account_inactive",
    "no_text",
];

/// Ceiling on an honoured `Retry-After`.
///
/// Slack's advice is normally seconds, but a delivery task must not be pinned by
/// a pathological value. Worst case is `max_attempts` waits at this cap.
const MAX_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// A failed Slack API call, typed so the retry loop can act on the reason.
///
/// Before EVE-968 every failure was flattened into a string and re-examined by
/// substring match, which threw away the one thing a 429 actually tells us:
/// how long to wait.
#[derive(Debug)]
pub(crate) enum SlackApiError {
    /// Slack asked us to slow down, and said for how long when it could.
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },
    /// Retrying cannot help: bad token, missing channel, empty message.
    Permanent(String),
    /// Network trouble, a 5xx, or an error code we do not recognise.
    Transient(String),
}

impl SlackApiError {
    /// Classify a Slack `error` code from an `ok: false` body.
    fn from_code(code: &str, retry_after: Option<std::time::Duration>) -> Self {
        if code == "ratelimited" {
            return Self::RateLimited { retry_after };
        }
        let message = format!("Slack API error: {code}");
        if PERMANENT_SLACK_ERRORS.contains(&code) {
            Self::Permanent(message)
        } else {
            Self::Transient(message)
        }
    }

    /// Whether retrying this failure is pointless.
    fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }
}

impl std::fmt::Display for SlackApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimited {
                retry_after: Some(d),
            } => {
                write!(f, "Slack API rate limited (retry after {}s)", d.as_secs())
            }
            Self::RateLimited { retry_after: None } => write!(f, "Slack API rate limited"),
            Self::Permanent(message) | Self::Transient(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SlackApiError {}

/// How long to wait before the next attempt.
///
/// Slack's own advice beats our guess. Retrying a 429 on a 1s backoff burns the
/// remaining attempts before the rate-limit window has even opened, which is how
/// a burst drops replies that would otherwise have gone through (EVE-968).
fn retry_wait(error: &SlackApiError, backoff: std::time::Duration) -> std::time::Duration {
    match error {
        SlackApiError::RateLimited {
            retry_after: Some(advice),
        } => (*advice).min(MAX_RETRY_AFTER),
        _ => backoff,
    }
}

/// Slack sends `Retry-After` in whole seconds.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<std::time::Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

#[async_trait]
impl ChannelDeliveryAdapter for SlackDeliveryAdapter {
    fn platform(&self) -> &str {
        "slack"
    }

    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        match post_to_slack_with_retry_base(
            &self.api_base,
            &context.auth_token,
            &context.channel_id,
            &context.thread_ref,
            &message.text,
        )
        .await
        {
            Ok(()) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }

    async fn send_ack(
        &self,
        thread_ref: &str,
        text: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        match post_to_slack_base(
            &self.api_base,
            &context.auth_token,
            &context.channel_id,
            thread_ref,
            text,
        )
        .await
        {
            Ok(()) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }

    fn format_progress_report(
        &self,
        report: &everruns_core::progress_reporting::ProgressReportPayload,
    ) -> String {
        format_progress_report_for_slack(report)
    }

    fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
        Some(self)
    }
}

/// Keys the Slack adapter reads out of `DeliveryContext::extra`.
///
/// `chat.startStream` refuses a channel stream without both and answers
/// `missing_recipient_team_id`. Neither is listed as required in Slack's
/// argument table; this was found by calling it (EVE-974).
pub(crate) const SLACK_RECIPIENT_USER_ID: &str = "slack_recipient_user_id";
pub(crate) const SLACK_RECIPIENT_TEAM_ID: &str = "slack_recipient_team_id";

/// Slack caps `markdown_text` at 12,000 characters per append.
const SLACK_APPEND_MAX_CHARS: usize = 12_000;

#[async_trait]
impl ChannelStreamDelivery for SlackDeliveryAdapter {
    async fn start(&self, context: &ChannelDeliveryContext) -> Result<String, String> {
        let mut payload = serde_json::json!({ "channel": context.channel_id });

        if !context.thread_ref.is_empty() {
            payload["thread_ts"] = serde_json::Value::String(context.thread_ref.clone());
        }
        for (extra_key, field) in [
            (SLACK_RECIPIENT_USER_ID, "recipient_user_id"),
            (SLACK_RECIPIENT_TEAM_ID, "recipient_team_id"),
        ] {
            if let Some(value) = context.extra.get(extra_key) {
                payload[field] = serde_json::Value::String(value.clone());
            }
        }

        let body = slack_api_call(
            &self.api_base,
            &context.auth_token,
            "chat.startStream",
            payload,
        )
        .await
        .map_err(|e| e.to_string())?;

        body.get("ts")
            .and_then(|ts| ts.as_str())
            .map(str::to_string)
            .ok_or_else(|| "chat.startStream returned no ts".to_string())
    }

    async fn append(
        &self,
        handle: &str,
        text: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        let payload = serde_json::json!({
            "channel": context.channel_id,
            "ts": handle,
            "markdown_text": truncate_chars(text, SLACK_APPEND_MAX_CHARS),
        });

        match slack_api_call(
            &self.api_base,
            &context.auth_token,
            "chat.appendStream",
            payload,
        )
        .await
        {
            Ok(_) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }

    async fn stop(&self, handle: &str, context: &ChannelDeliveryContext) -> ChannelDeliveryResult {
        let payload = serde_json::json!({
            "channel": context.channel_id,
            "ts": handle,
        });

        match slack_api_call(
            &self.api_base,
            &context.auth_token,
            "chat.stopStream",
            payload,
        )
        .await
        {
            Ok(_) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }
}

/// Truncate to at most `max` characters, on a char boundary.
fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((idx, _)) => s[..idx].to_string(),
        None => s.to_string(),
    }
}

/// Map a Slack transport failure onto a `ChannelDeliveryResult`.
fn classify_slack_failure(error: SlackApiError) -> ChannelDeliveryResult {
    match error {
        SlackApiError::Permanent(message) => ChannelDeliveryResult::PermanentError(message),
        other => ChannelDeliveryResult::TransientError(other.to_string()),
    }
}

/// Extract text content from an output.message.completed event's data.
pub(crate) fn extract_response_text(data: &serde_json::Value) -> Option<String> {
    let message = data.get("message")?;
    let content = message.get("content")?.as_array()?;

    let mut text_parts = Vec::new();
    for part in content {
        if part.get("type")?.as_str()? == "text"
            && let Some(text) = part.get("text").and_then(|t| t.as_str())
        {
            text_parts.push(text.to_string());
        }
    }

    if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    }
}

pub(crate) fn extract_progress_report_text(data: &serde_json::Value) -> Option<String> {
    if data.get("tool_name")?.as_str()? != REPORT_PROGRESS_TOOL_NAME {
        return None;
    }
    if !data
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    let result = data.get("result")?.as_array()?;
    let json_text = result.iter().find_map(|part| {
        if part.get("type")?.as_str()? == "text" {
            part.get("text").and_then(|text| text.as_str())
        } else {
            None
        }
    })?;

    let payload: ProgressReportPayload = serde_json::from_str(json_text).ok()?;
    Some(format_progress_report_for_slack(&payload))
}

pub(crate) fn extract_delivery_text(
    event_type: &str,
    reply_mode: SlackReplyMode,
    data: &serde_json::Value,
) -> Option<String> {
    match reply_mode {
        SlackReplyMode::AllMessages if event_type == "output.message.completed" => {
            extract_response_text(data)
        }
        SlackReplyMode::ReportProgressOnly if event_type == "tool.completed" => {
            extract_progress_report_text(data)
        }
        _ => None,
    }
}

async fn post_to_slack_with_retry_base(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> Result<(), SlackApiError> {
    let max_attempts = 3;
    let mut backoff = std::time::Duration::from_secs(1);

    for attempt in 1..=max_attempts {
        match post_to_slack_base(base_url, bot_token, channel, thread_ts, text).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if e.is_permanent() || attempt == max_attempts {
                    return Err(e);
                }

                let wait = retry_wait(&e, backoff);

                warn!(
                    attempt,
                    max_attempts,
                    wait_secs = wait.as_secs(),
                    error = %e,
                    "Slack post failed, retrying"
                );
                tokio::time::sleep(wait).await;
                backoff *= 2;
            }
        }
    }

    unreachable!()
}

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Post a message to Slack using the Bot API.
pub(crate) async fn post_to_slack(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> anyhow::Result<()> {
    post_to_slack_base(SLACK_API_BASE, bot_token, channel, thread_ts, text).await?;
    Ok(())
}

/// Post a message to Slack using the Bot API (with configurable base URL for testing).
pub(crate) async fn post_to_slack_base(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> Result<(), SlackApiError> {
    let mut payload = serde_json::json!({
        "channel": channel,
        "text": text,
    });

    if !thread_ts.is_empty() {
        payload["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
    }

    slack_api_call(base_url, bot_token, "chat.postMessage", payload).await?;
    info!(channel = channel, "Posted response to Slack");
    Ok(())
}

/// Call one Slack Web API method.
///
/// Every method shares the same envelope — `ok: false` plus an `error` code, and
/// a `Retry-After` header on a rate limit — so error handling lives here rather
/// than being rewritten per endpoint.
async fn slack_api_call(
    base_url: &str,
    bot_token: &str,
    method: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, SlackApiError> {
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/{}", base_url, method))
        .header("Authorization", format!("Bearer {}", bot_token))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| SlackApiError::Transient(e.to_string()))?;

    let status = response.status();
    // Read the header before the body is consumed: a 429 carries its advice here,
    // not in the JSON.
    let retry_after = parse_retry_after(response.headers());

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| SlackApiError::Transient(e.to_string()))?;

    if !body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown");

        let failure = SlackApiError::from_code(error, retry_after);

        // A rate limit is routine backpressure, not a fault to shout about.
        if matches!(failure, SlackApiError::RateLimited { .. }) {
            debug!(
                method = method,
                retry_after_secs = ?retry_after.map(|d| d.as_secs()),
                status = %status,
                "Slack rate limited the call"
            );
        } else {
            error!(
                method = method,
                error = error,
                status = %status,
                "Slack API call failed"
            );
        }
        return Err(failure);
    }

    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_response_text_valid() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "text", "text": "Hello from the agent!"},
                    {"type": "text", "text": "Second part."}
                ]
            }
        });
        let result = extract_response_text(&data);
        assert_eq!(
            result,
            Some("Hello from the agent!\nSecond part.".to_string())
        );
    }

    #[test]
    fn test_extract_response_text_single_part() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "text", "text": "Only one part."}
                ]
            }
        });
        assert_eq!(
            extract_response_text(&data),
            Some("Only one part.".to_string())
        );
    }

    #[test]
    fn test_extract_response_text_no_text() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "tool_use", "name": "search"}
                ]
            }
        });
        assert_eq!(extract_response_text(&data), None);
    }

    #[test]
    fn test_extract_response_text_empty_content() {
        let data = serde_json::json!({
            "message": {
                "content": []
            }
        });
        assert_eq!(extract_response_text(&data), None);
    }

    #[test]
    fn test_extract_response_text_missing_message() {
        let data = serde_json::json!({});
        assert_eq!(extract_response_text(&data), None);
    }

    #[test]
    fn test_extract_response_text_missing_content() {
        let data = serde_json::json!({
            "message": {}
        });
        assert_eq!(extract_response_text(&data), None);
    }

    #[test]
    fn test_extract_response_text_mixed_content() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "text", "text": "Part 1"},
                    {"type": "tool_use", "name": "search"},
                    {"type": "text", "text": "Part 2"}
                ]
            }
        });
        // tool_use part causes early return via `?` operator on type check
        // Only "Part 1" is extracted before the tool_use part returns None from the iterator
        let result = extract_response_text(&data);
        // The `?` in part.get("type")?.as_str()? causes the for loop to
        // short-circuit the entire function when a non-text part doesn't have
        // the expected structure. But tool_use does have "type", so it just
        // doesn't match "text" and the `&&` short-circuits. Both text parts
        // should be captured.
        assert_eq!(result, Some("Part 1\nPart 2".to_string()));
    }

    #[test]
    fn test_extract_progress_report_text_formats_payload() {
        let data = serde_json::json!({
            "tool_name": "report_progress",
            "success": true,
            "result": [
                {
                    "type": "text",
                    "text": r#"{"status":"completed","summary":"Shipped the fix","details":["updated Slack reply mode"]}"#
                }
            ]
        });

        assert_eq!(
            extract_progress_report_text(&data),
            Some("Done: Shipped the fix\n- updated Slack reply mode".to_string())
        );
    }

    #[test]
    fn test_extract_delivery_text_respects_reply_mode() {
        let output = serde_json::json!({
            "message": {
                "content": [
                    {"type": "text", "text": "Normal assistant reply"}
                ]
            }
        });
        let tool = serde_json::json!({
            "tool_name": "report_progress",
            "success": true,
            "result": [
                {
                    "type": "text",
                    "text": r#"{"status":"progress","summary":"Investigating","details":[]}"#
                }
            ]
        });

        assert_eq!(
            extract_delivery_text(
                "output.message.completed",
                SlackReplyMode::AllMessages,
                &output
            ),
            Some("Normal assistant reply".to_string())
        );
        assert_eq!(
            extract_delivery_text("tool.completed", SlackReplyMode::AllMessages, &tool),
            None
        );
        assert_eq!(
            extract_delivery_text("tool.completed", SlackReplyMode::ReportProgressOnly, &tool),
            Some("Update: Investigating".to_string())
        );
        assert_eq!(
            extract_delivery_text(
                "output.message.completed",
                SlackReplyMode::ReportProgressOnly,
                &output
            ),
            None
        );
    }

    #[test]
    fn test_delivery_key_eq() {
        let k1 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        let k2 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_delivery_key_different_session() {
        let k1 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        let k2 = DeliveryKey {
            session_id: Uuid::from_u128(1),
            input_message_id: "msg_123".to_string(),
        };
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_delivery_key_different_message() {
        let k1 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        let k2 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_456".to_string(),
        };
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_delivery_key_hash_consistency() {
        use std::collections::HashMap;
        let key = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        let mut map = HashMap::new();
        map.insert(key.clone(), "value");
        assert_eq!(map.get(&key), Some(&"value"));
    }

    // ==========================================
    // WireMock integration tests — post_to_slack
    // ==========================================

    mod wiremock_tests {
        use super::*;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        #[tokio::test]
        async fn test_post_to_slack_success() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .and(header("Authorization", "Bearer xoxb-test-token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": "1234567890.123456"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "1234567890.000000",
                "Hello from agent!",
            )
            .await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_post_to_slack_success_no_thread_ts() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": "1234567890.123456"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            // Empty thread_ts should still succeed (message not in thread)
            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_post_to_slack_channel_not_found() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "channel_not_found"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C_INVALID",
                "",
                "Hello!",
            )
            .await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("channel_not_found")
            );
        }

        #[tokio::test]
        async fn test_post_to_slack_invalid_auth() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "invalid_auth"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-bad-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("invalid_auth"));
        }

        #[tokio::test]
        async fn test_post_to_slack_rate_limited() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "ratelimited"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("rate limited"));
        }

        #[tokio::test]
        async fn test_post_to_slack_token_revoked() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "token_revoked"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-revoked", "C123", "", "Hello!").await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("token_revoked"));
        }

        #[tokio::test]
        async fn test_post_to_slack_unknown_error() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "some_unexpected_error"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("some_unexpected_error")
            );
        }

        #[tokio::test]
        async fn test_post_to_slack_malformed_json() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_server_error() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            // 500 with non-JSON body → reqwest JSON parse error
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_non_retryable_errors_fail_immediately() {
            let mock_server = MockServer::start().await;

            // Non-retryable error should only be called once (no retry)
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "channel_not_found"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C_GONE",
                "",
                "Hello!",
            )
            .await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("channel_not_found")
            );
            // .expect(1) verifies exactly one call was made (no retries)
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_success_on_first_attempt() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": "1234567890.123456"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "1234.0000",
                "Hello!",
            )
            .await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_not_authed() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "not_authed"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "",
                "Hello!",
            )
            .await;

            assert!(result.is_err());
            // Should not retry — exactly 1 call
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_account_inactive() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "account_inactive"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "",
                "Hello!",
            )
            .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_no_text() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "no_text"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "",
                "",
            )
            .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_ok_false_no_error_field() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("unknown"));
        }
    }

    // Regression tests for Slack recovery app ownership.
    //
    // Recovery must use the server-owned `sessions.app_id` FK. Mutable
    // `slack:app:*` routing tags alone must not opt a session into recovery.
    mod recovery_scoping_tests {
        use super::*;
        use crate::storage::StorageBackend;
        use crate::storage::models::{CreateAppRow, CreateSessionRow, UpdateSession};
        use everruns_provider::typed_id::PrincipalId;
        use everruns_provider::typed_id::{AgentId, HarnessId};
        use tokio::sync::broadcast;

        const ORG_APP_OWNER: i64 = 10;
        const ORG_ATTACKER: i64 = 20;
        const LEGIT_APP_PUBLIC_ID: &str = "app_legit_test";

        async fn seed_slack_app(db: &StorageBackend, org_id: i64, public_id: &str) -> uuid::Uuid {
            db.create_app(
                org_id,
                CreateAppRow {
                    public_id: public_id.to_string(),
                    name: "Legit Slack App".to_string(),
                    description: None,
                    harness_id: uuid::Uuid::nil(),
                    agent_id: Some(uuid::Uuid::nil()),
                    agent_version_policy: "default".to_string(),
                    agent_version_id: None,
                    agent_identity_id: None,
                    owner_principal_id: PrincipalId::from_seed(1),
                    resolved_owner_user_id: None,
                    channel_type: Some("slack".to_string()),
                    channel_config: serde_json::json!({
                        "signing_secret": "sig",
                        "bot_token": "xoxb-secret-bot-token",
                        "session_strategy": "per_thread",
                        "reply_mode": "all_messages",
                    }),
                    channel_config_encrypted: None,
                },
            )
            .await
            .expect("seed slack app")
            .id
        }

        async fn seed_active_slack_delivery_session(
            db: &StorageBackend,
            org_id: i64,
            app_id: Option<uuid::Uuid>,
            app_public_id: &str,
        ) -> everruns_provider::typed_id::SessionId {
            use crate::storage::models::CreateEventRow;

            let session = db
                .create_session(CreateSessionRow {
                    source: everruns_platform::SessionSource::Api,
                    workspace_id: None,
                    org_id,
                    app_id,
                    harness_id: Some(HarnessId::from_uuid(uuid::Uuid::nil())),
                    agent_id: Some(AgentId::from_uuid(uuid::Uuid::nil())),
                    agent_version_id: None,
                    agent_config_hash: None,
                    agent_identity_id: None,
                    owner_principal_id: PrincipalId::from_seed(1),
                    resolved_owner_user_id: None,
                    title: Some("recovery test".to_string()),
                    locale: None,
                    tags: vec![format!("slack:app:{app_public_id}")],
                    model_id: None,
                    capabilities: serde_json::json!([]),
                    tools: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    system_prompt: None,
                    initial_files: serde_json::Value::Array(vec![]),
                    hints: None,
                    max_iterations: None,
                    parallel_tool_calls: None,
                    blueprint_id: None,
                    blueprint_config: None,
                    network_access: None,
                    parent_session_id: None,
                    budget_root_session_id: None,
                })
                .await
                .expect("create session");

            db.update_session(
                org_id,
                session.id,
                UpdateSession {
                    status: Some("active".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("activate session");

            // Seed an input.message event so recover() has enough context to
            // reach the `register()` call under the pre-fix (unscoped) lookup.
            // Without this, recover() would exit via the "no events" branch
            // and the test couldn't distinguish pre- and post-fix behavior.
            db.create_event(CreateEventRow {
                session_id: session.id,
                event_type: "input.message".to_string(),
                ts: chrono::Utc::now(),
                context: serde_json::json!({}),
                data: serde_json::json!({
                    "message": {
                        "id": "msg_test_input",
                        "metadata": {
                            "slack_channel": "C_TEST",
                            "slack_ts": "1234.5678",
                        },
                    },
                }),
                metadata: None,
                tags: None,
            })
            .await
            .expect("seed input.message");

            session.id
        }

        fn build_dispatcher(db: Arc<StorageBackend>) -> Arc<SlackDeliveryDispatcher> {
            // `recover` is independent of the event loop; we only need the
            // dispatcher struct with a valid broadcast receiver so `start`
            // can wire the event loop.
            let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
            SlackDeliveryDispatcher::start(db, rx, String::new())
        }

        #[tokio::test]
        async fn recover_ignores_cross_org_slack_app_tag() {
            let db = Arc::new(StorageBackend::in_memory());
            seed_slack_app(&db, ORG_APP_OWNER, LEGIT_APP_PUBLIC_ID).await;
            // Attacker session sits in a different org but tags a real app's public id.
            seed_active_slack_delivery_session(&db, ORG_ATTACKER, None, LEGIT_APP_PUBLIC_ID).await;

            let dispatcher = build_dispatcher(db);
            dispatcher.recover(None).await;

            assert_eq!(
                dispatcher.active_delivery_count().await,
                0,
                "cross-org slack:app:* tag must not register a delivery during recovery"
            );
        }

        #[tokio::test]
        async fn recover_ignores_session_with_unknown_app_tag() {
            let db = Arc::new(StorageBackend::in_memory());
            // No app exists with this public id anywhere.
            seed_active_slack_delivery_session(&db, ORG_ATTACKER, None, "app_does_not_exist").await;

            let dispatcher = build_dispatcher(db);
            dispatcher.recover(None).await;

            assert_eq!(
                dispatcher.active_delivery_count().await,
                0,
                "missing slack:app:* lookup must not leak recovery to any app"
            );
        }

        #[tokio::test]
        async fn recover_uses_session_app_id_instead_of_slack_app_tag() {
            let db = Arc::new(StorageBackend::in_memory());
            let app_id = seed_slack_app(&db, ORG_APP_OWNER, LEGIT_APP_PUBLIC_ID).await;
            seed_active_slack_delivery_session(&db, ORG_APP_OWNER, Some(app_id), "app_forged_tag")
                .await;

            let dispatcher = build_dispatcher(db);
            dispatcher.recover(None).await;

            assert_eq!(
                dispatcher.active_delivery_count().await,
                1,
                "session app_id should drive Slack recovery even if routing tags drift"
            );
        }
    }

    /// EVE-973: one app serves both Slack surfaces, and which one an event
    /// belongs to is read from the event rather than from config.
    mod surface_tests {
        use super::*;

        #[test]
        fn surface_off_is_channel_everywhere() {
            // Including DMs — that is exactly today's behaviour, and enabling the
            // feature must be the only thing that changes it.
            assert_eq!(
                classify_surface(false, Some("im"), "D123"),
                SlackSurface::Channel
            );
            assert_eq!(
                classify_surface(false, Some("channel"), "C123"),
                SlackSurface::Channel
            );
        }

        #[test]
        fn live_events_classify_on_channel_type() {
            assert_eq!(
                classify_surface(true, Some("im"), "D123"),
                SlackSurface::Pane
            );
            for kind in ["channel", "group", "mpim"] {
                assert_eq!(
                    classify_surface(true, Some(kind), "C123"),
                    SlackSurface::Channel,
                    "{kind} is not the pane"
                );
            }
        }

        /// Recovery has no event to read, only the stored channel id.
        #[test]
        fn recovery_classifies_on_channel_id() {
            assert_eq!(classify_surface(true, None, "D0A1B2C3"), SlackSurface::Pane);
            assert_eq!(
                classify_surface(true, None, "C0A1B2C3"),
                SlackSurface::Channel
            );
            assert_eq!(classify_surface(true, None, ""), SlackSurface::Channel);
        }

        /// An `app_mention` delivered with an explicit channel type is channel-shaped
        /// even if the id looks like a DM: the event's own word wins.
        #[test]
        fn channel_type_wins_over_channel_id() {
            assert_eq!(
                classify_surface(true, Some("channel"), "D123"),
                SlackSurface::Channel
            );
        }
    }

    /// EVE-972: every outbound message leaves through `ChannelDeliveryAdapter`,
    /// and transient-vs-permanent is decided in exactly one place.
    mod adapter_routing_tests {
        use super::*;
        use crate::storage::StorageBackend;
        use std::sync::Mutex;
        use tokio::sync::broadcast;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// Adapter that records what it was asked to send and never touches the
        /// network. If the dispatcher still called Slack directly, the recorder
        /// would stay empty and the mock would see traffic instead.
        struct RecordingAdapter {
            sent: Arc<Mutex<Vec<(String, bool)>>>,
        }

        #[async_trait]
        impl ChannelDeliveryAdapter for RecordingAdapter {
            fn platform(&self) -> &str {
                "recording"
            }

            async fn deliver(
                &self,
                message: &OutboundChannelMessage,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
                self.sent
                    .lock()
                    .expect("recorder lock")
                    .push((message.text.clone(), message.is_progress_report));
                ChannelDeliveryResult::Ok
            }

            async fn send_ack(
                &self,
                _thread_ref: &str,
                _text: &str,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
                ChannelDeliveryResult::Ok
            }

            fn format_progress_report(&self, report: &ProgressReportPayload) -> String {
                format_progress_report_for_slack(report)
            }
        }

        #[tokio::test]
        async fn dispatcher_delivers_through_the_adapter() {
            let db = Arc::new(StorageBackend::in_memory());
            let session_id = terminal_state_tests::seed_session(&db).await;

            // A live Slack mock that must never be called: the dispatcher's only
            // route out is the adapter above.
            let unused_slack = MockServer::start().await;
            let sent = Arc::new(Mutex::new(Vec::new()));
            let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
            let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
                db.clone(),
                rx,
                "https://app.example.com".to_string(),
                Arc::new(RecordingAdapter { sent: sent.clone() }),
            );

            dispatcher
                .register(DeliveryRegistration {
                    session_id: session_id.uuid(),
                    input_message_id: "msg_turn_one".to_string(),
                    bot_token: "xoxb-test-token".to_string(),
                    channel: "C_ADAPTER".to_string(),
                    thread_ts: "1700000000.000100".to_string(),
                    reply_mode: SlackReplyMode::AllMessages,
                    surface: SlackSurface::Channel,
                    recipient_user_id: None,
                    recipient_team_id: None,
                })
                .await;

            terminal_state_tests::emit(
                &db,
                session_id,
                "output.message.completed",
                "msg_turn_one",
                serde_json::json!({
                    "message": { "content": [{ "type": "text", "text": "Routed reply." }] }
                }),
            )
            .await;
            terminal_state_tests::emit(
                &db,
                session_id,
                "turn.completed",
                "msg_turn_one",
                serde_json::json!({}),
            )
            .await;
            dispatcher.process_session_events(session_id.uuid()).await;

            let sent = sent.lock().expect("recorder lock").clone();
            assert_eq!(sent, vec![("Routed reply.".to_string(), false)]);
            assert!(
                unused_slack
                    .received_requests()
                    .await
                    .unwrap_or_default()
                    .is_empty(),
                "dispatcher must make no direct Slack API calls"
            );
        }

        async fn deliver_against(body: serde_json::Value) -> ChannelDeliveryResult {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&mock_server)
                .await;

            let adapter = SlackDeliveryAdapter::with_api_base(mock_server.uri());
            let message = OutboundChannelMessage {
                session_id: SessionId::from_uuid(uuid::Uuid::nil()),
                text: "hello".to_string(),
                thread_ref: String::new(),
                is_progress_report: false,
            };
            let ctx = ChannelDeliveryContext {
                auth_token: "xoxb-test-token".to_string(),
                channel_id: "C123".to_string(),
                thread_ref: String::new(),
                reply_mode: SlackReplyMode::AllMessages.into(),
                extra: HashMap::new(),
            };
            adapter.deliver(&message, &ctx).await
        }

        /// `account_inactive` and `no_text` were already treated as unretryable by
        /// the retry loop, but the adapter reported them as transient — the two
        /// lists had drifted apart. One list now answers both questions.
        #[tokio::test]
        async fn permanent_errors_are_reported_permanent() {
            for error in ["channel_not_found", "account_inactive", "no_text"] {
                let result =
                    deliver_against(serde_json::json!({ "ok": false, "error": error })).await;
                assert!(
                    matches!(result, ChannelDeliveryResult::PermanentError(_)),
                    "{error} must be permanent, got {result:?}"
                );
            }
        }

        #[tokio::test]
        async fn unknown_errors_stay_transient() {
            let result =
                deliver_against(serde_json::json!({ "ok": false, "error": "ratelimited" })).await;
            assert!(
                matches!(result, ChannelDeliveryResult::TransientError(_)),
                "a rate limit must stay retryable, got {result:?}"
            );
        }

        #[test]
        fn classification_lists_every_unretryable_error() {
            for error in [
                "channel_not_found",
                "not_authed",
                "invalid_auth",
                "token_revoked",
                "account_inactive",
                "no_text",
            ] {
                assert!(
                    SlackApiError::from_code(error, None).is_permanent(),
                    "{error} must be permanent"
                );
            }
            assert!(!SlackApiError::from_code("internal_error", None).is_permanent());

            // A rate limit is its own variant, never permanent.
            assert!(matches!(
                SlackApiError::from_code("ratelimited", None),
                SlackApiError::RateLimited { .. }
            ));
        }
    }

    /// EVE-974: pane replies render progressively, and every terminal state closes
    /// the stream. A stream left open spins in the client forever.
    mod streaming_tests {
        use super::*;
        use crate::storage::StorageBackend;
        use std::sync::Mutex;
        use tokio::sync::broadcast;

        /// What the dispatcher asked the platform to do, in order.
        #[derive(Debug, Clone, PartialEq, Eq)]
        enum Call {
            Start,
            Append(String, String),
            Stop(String),
            Discrete(String),
        }

        struct RecordingAdapter {
            calls: Arc<Mutex<Vec<Call>>>,
            /// When false, `start` fails so the fallback path can be exercised.
            can_start: bool,
        }

        impl RecordingAdapter {
            fn new(calls: Arc<Mutex<Vec<Call>>>) -> Self {
                Self {
                    calls,
                    can_start: true,
                }
            }

            fn push(&self, call: Call) {
                self.calls.lock().expect("recorder").push(call);
            }
        }

        #[async_trait]
        impl ChannelDeliveryAdapter for RecordingAdapter {
            fn platform(&self) -> &str {
                "recording"
            }

            async fn deliver(
                &self,
                message: &OutboundChannelMessage,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
                self.push(Call::Discrete(message.text.clone()));
                ChannelDeliveryResult::Ok
            }

            async fn send_ack(
                &self,
                _thread_ref: &str,
                _text: &str,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
                ChannelDeliveryResult::Ok
            }

            fn format_progress_report(&self, report: &ProgressReportPayload) -> String {
                format_progress_report_for_slack(report)
            }

            fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
                Some(self)
            }
        }

        #[async_trait]
        impl ChannelStreamDelivery for RecordingAdapter {
            async fn start(&self, _context: &ChannelDeliveryContext) -> Result<String, String> {
                if !self.can_start {
                    return Err("stream unavailable".to_string());
                }
                self.push(Call::Start);
                let n = self
                    .calls
                    .lock()
                    .expect("recorder")
                    .iter()
                    .filter(|c| matches!(c, Call::Start))
                    .count();
                Ok(format!("stream-{n}"))
            }

            async fn append(
                &self,
                handle: &str,
                text: &str,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
                // Yield so concurrent flushes actually interleave — a real append
                // is a ~300ms network call, which is where the duplication this
                // guards against came from.
                tokio::task::yield_now().await;
                self.push(Call::Append(handle.to_string(), text.to_string()));
                ChannelDeliveryResult::Ok
            }

            async fn stop(
                &self,
                handle: &str,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
                self.push(Call::Stop(handle.to_string()));
                ChannelDeliveryResult::Ok
            }
        }

        async fn dispatcher_with(
            db: Arc<StorageBackend>,
            adapter: RecordingAdapter,
        ) -> Arc<SlackDeliveryDispatcher> {
            let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
            SlackDeliveryDispatcher::start_with_adapter(
                db,
                rx,
                "https://app.example.com".to_string(),
                Arc::new(adapter),
            )
        }

        async fn register(
            dispatcher: &SlackDeliveryDispatcher,
            session: Uuid,
            surface: SlackSurface,
        ) {
            dispatcher
                .register(DeliveryRegistration {
                    session_id: session,
                    input_message_id: "msg_in".to_string(),
                    bot_token: "xoxb-t".to_string(),
                    channel: "D_PANE".to_string(),
                    thread_ts: "1700000000.000100".to_string(),
                    reply_mode: SlackReplyMode::AllMessages,
                    surface,
                    recipient_user_id: Some("U_HUMAN".to_string()),
                    recipient_team_id: Some("T_TEAM".to_string()),
                })
                .await;
        }

        async fn delta(
            db: &StorageBackend,
            session: SessionId,
            message_id: &str,
            accumulated: &str,
        ) {
            terminal_state_tests::emit(
                db,
                session,
                "output.message.delta",
                "msg_in",
                serde_json::json!({ "message_id": message_id, "accumulated": accumulated }),
            )
            .await;
        }

        async fn completed(db: &StorageBackend, session: SessionId, message_id: &str, text: &str) {
            terminal_state_tests::emit(
                db,
                session,
                "output.message.completed",
                "msg_in",
                serde_json::json!({
                    "message": { "id": message_id, "content": [{ "type": "text", "text": text }] }
                }),
            )
            .await;
        }

        fn recorded(calls: &Arc<Mutex<Vec<Call>>>) -> Vec<Call> {
            calls.lock().expect("recorder").clone()
        }

        #[tokio::test]
        async fn pane_reply_streams_then_stops() {
            let db = Arc::new(StorageBackend::in_memory());
            let session = terminal_state_tests::seed_session(&db).await;
            let calls = Arc::new(Mutex::new(Vec::new()));
            let dispatcher =
                dispatcher_with(db.clone(), RecordingAdapter::new(calls.clone())).await;
            register(&dispatcher, session.uuid(), SlackSurface::Pane).await;

            delta(&db, session, "m1", "Hel").await;
            delta(&db, session, "m1", "Hello wor").await;
            completed(&db, session, "m1", "Hello world").await;
            dispatcher.process_session_events(session.uuid()).await;

            let calls = recorded(&calls);
            assert_eq!(
                calls[0],
                Call::Start,
                "first delta opens the stream: {calls:?}"
            );
            assert_eq!(
                calls.last(),
                Some(&Call::Stop("stream-1".to_string())),
                "completed closes it: {calls:?}"
            );
            // The whole reply reached Slack exactly once, in order.
            let appended: String = calls
                .iter()
                .filter_map(|c| match c {
                    Call::Append(_, text) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(appended, "Hello world");
            assert!(
                !calls.iter().any(|c| matches!(c, Call::Discrete(_))),
                "a streamed message must not also be posted discretely: {calls:?}"
            );
        }

        /// Two flushes racing must not send the same text twice.
        ///
        /// Found against a real workspace, not in a unit test: the dispatcher's own
        /// 500ms tick overlapped an event-driven flush, both read the same `sent`
        /// offset, and the reader saw "StreamingStreaming works works end to end."
        /// `sent` is therefore claimed under the lock before the network call.
        #[tokio::test]
        async fn concurrent_flushes_do_not_duplicate_text() {
            let db = Arc::new(StorageBackend::in_memory());
            let session = terminal_state_tests::seed_session(&db).await;
            let calls = Arc::new(Mutex::new(Vec::new()));
            let dispatcher =
                dispatcher_with(db.clone(), RecordingAdapter::new(calls.clone())).await;
            register(&dispatcher, session.uuid(), SlackSurface::Pane).await;

            delta(&db, session, "m1", "Streaming works end to end.").await;
            dispatcher.process_session_events(session.uuid()).await;

            let (a, b) = (dispatcher.clone(), dispatcher.clone());
            tokio::join!(async move { a.flush_open_streams().await }, async move {
                b.flush_open_streams().await
            },);

            let appended: String = recorded(&calls)
                .iter()
                .filter_map(|c| match c {
                    Call::Append(_, text) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                appended, "Streaming works end to end.",
                "racing flushes must not re-send claimed text"
            );
        }

        #[tokio::test]
        async fn channel_surface_does_not_stream() {
            let db = Arc::new(StorageBackend::in_memory());
            let session = terminal_state_tests::seed_session(&db).await;
            let calls = Arc::new(Mutex::new(Vec::new()));
            let dispatcher =
                dispatcher_with(db.clone(), RecordingAdapter::new(calls.clone())).await;
            register(&dispatcher, session.uuid(), SlackSurface::Channel).await;

            delta(&db, session, "m1", "Hello").await;
            completed(&db, session, "m1", "Hello world").await;
            dispatcher.process_session_events(session.uuid()).await;

            assert_eq!(
                recorded(&calls),
                vec![Call::Discrete("Hello world".to_string())],
                "token-by-token into a shared channel is not wanted"
            );
        }

        #[tokio::test]
        async fn several_output_messages_are_several_streams() {
            let db = Arc::new(StorageBackend::in_memory());
            let session = terminal_state_tests::seed_session(&db).await;
            let calls = Arc::new(Mutex::new(Vec::new()));
            let dispatcher =
                dispatcher_with(db.clone(), RecordingAdapter::new(calls.clone())).await;
            register(&dispatcher, session.uuid(), SlackSurface::Pane).await;

            delta(&db, session, "m1", "first").await;
            completed(&db, session, "m1", "first").await;
            delta(&db, session, "m2", "second").await;
            completed(&db, session, "m2", "second").await;
            dispatcher.process_session_events(session.uuid()).await;

            let calls = recorded(&calls);
            let starts = calls.iter().filter(|c| matches!(c, Call::Start)).count();
            let stops = calls.iter().filter(|c| matches!(c, Call::Stop(_))).count();
            assert_eq!(
                (starts, stops),
                (2, 2),
                "two messages, two streams: {calls:?}"
            );
            assert!(
                calls.contains(&Call::Stop("stream-1".to_string()))
                    && calls.contains(&Call::Stop("stream-2".to_string())),
                "each stream closes on its own handle: {calls:?}"
            );
        }

        /// An unstopped stream is worse than the silence EVE-966 fixed.
        #[tokio::test]
        async fn every_terminal_state_stops_an_open_stream() {
            for terminal in ["turn.completed", "turn.failed", "turn.cancelled"] {
                let db = Arc::new(StorageBackend::in_memory());
                let session = terminal_state_tests::seed_session(&db).await;
                let calls = Arc::new(Mutex::new(Vec::new()));
                let dispatcher =
                    dispatcher_with(db.clone(), RecordingAdapter::new(calls.clone())).await;
                register(&dispatcher, session.uuid(), SlackSurface::Pane).await;

                // A stream is opened and then the turn ends without completing it.
                delta(&db, session, "m1", "half an ans").await;
                terminal_state_tests::emit(&db, session, terminal, "msg_in", serde_json::json!({}))
                    .await;
                dispatcher.process_session_events(session.uuid()).await;

                let calls = recorded(&calls);
                let stops = calls.iter().filter(|c| matches!(c, Call::Stop(_))).count();
                assert_eq!(
                    stops, 1,
                    "{terminal} must stop the stream exactly once: {calls:?}"
                );
                assert_eq!(
                    dispatcher.active_delivery_count().await,
                    0,
                    "{terminal} must still unregister"
                );
            }
        }

        /// If the platform will not open a stream, the reply must still arrive.
        #[tokio::test]
        async fn failed_start_falls_back_to_a_discrete_reply() {
            let db = Arc::new(StorageBackend::in_memory());
            let session = terminal_state_tests::seed_session(&db).await;
            let calls = Arc::new(Mutex::new(Vec::new()));
            let dispatcher = dispatcher_with(
                db.clone(),
                RecordingAdapter {
                    calls: calls.clone(),
                    can_start: false,
                },
            )
            .await;
            register(&dispatcher, session.uuid(), SlackSurface::Pane).await;

            delta(&db, session, "m1", "Hel").await;
            completed(&db, session, "m1", "Hello world").await;
            dispatcher.process_session_events(session.uuid()).await;

            assert_eq!(
                recorded(&calls),
                vec![Call::Discrete("Hello world".to_string())],
                "a stream that cannot open must not swallow the reply"
            );
        }

        #[tokio::test]
        async fn flush_sends_only_what_is_new() {
            let db = Arc::new(StorageBackend::in_memory());
            let session = terminal_state_tests::seed_session(&db).await;
            let calls = Arc::new(Mutex::new(Vec::new()));
            let dispatcher =
                dispatcher_with(db.clone(), RecordingAdapter::new(calls.clone())).await;
            register(&dispatcher, session.uuid(), SlackSurface::Pane).await;

            delta(&db, session, "m1", "one").await;
            dispatcher.process_session_events(session.uuid()).await;
            dispatcher.flush_open_streams().await;

            delta(&db, session, "m1", "one two").await;
            dispatcher.process_session_events(session.uuid()).await;
            dispatcher.flush_open_streams().await;

            let appends: Vec<String> = recorded(&calls)
                .into_iter()
                .filter_map(|c| match c {
                    Call::Append(_, text) => Some(text),
                    _ => None,
                })
                .collect();
            assert_eq!(
                appends,
                vec!["one".to_string(), " two".to_string()],
                "each flush sends the tail, never the whole accumulated text again"
            );
        }
    }

    /// EVE-968: a 429 carries Slack's own advice on how long to wait. Ignoring it
    /// is how a burst drops replies that would otherwise have gone through.
    mod rate_limit_tests {
        use super::*;
        use std::time::Duration;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// Post once against a mock returning `response`, and hand back the error.
        async fn post_against(response: ResponseTemplate) -> SlackApiError {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(response)
                .mount(&mock_server)
                .await;

            post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "hi")
                .await
                .expect_err("rate limited")
        }

        fn rate_limited_body() -> serde_json::Value {
            serde_json::json!({ "ok": false, "error": "ratelimited" })
        }

        #[tokio::test]
        async fn retry_after_header_is_propagated() {
            let error = post_against(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "30")
                    .set_body_json(rate_limited_body()),
            )
            .await;

            assert!(
                matches!(
                    error,
                    SlackApiError::RateLimited {
                        retry_after: Some(d)
                    } if d == Duration::from_secs(30)
                ),
                "expected 30s of advice, got {error:?}"
            );
        }

        #[tokio::test]
        async fn rate_limit_without_header_is_still_rate_limited() {
            let error =
                post_against(ResponseTemplate::new(200).set_body_json(rate_limited_body())).await;

            assert!(
                matches!(error, SlackApiError::RateLimited { retry_after: None }),
                "expected a rate limit with no advice, got {error:?}"
            );
        }

        #[tokio::test]
        async fn unparseable_retry_after_falls_back_to_no_advice() {
            // Slack documents whole seconds; an HTTP-date or junk value must not
            // panic or be mistaken for a duration.
            let error = post_against(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "Wed, 21 Oct 2026 07:28:00 GMT")
                    .set_body_json(rate_limited_body()),
            )
            .await;

            assert!(
                matches!(error, SlackApiError::RateLimited { retry_after: None }),
                "expected no usable advice, got {error:?}"
            );
        }

        /// End to end through the retry loop, on virtual time: the first attempt
        /// is rate limited with 30s of advice, the second succeeds, and the loop
        /// must have waited the 30s rather than its 1s backoff.
        #[tokio::test(start_paused = true)]
        async fn retry_loop_waits_the_advised_window() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(429)
                        .insert_header("Retry-After", "30")
                        .set_body_json(rate_limited_body()),
                )
                .up_to_n_times(1)
                .mount(&mock_server)
                .await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({ "ok": true, "ts": "1.2" })),
                )
                .mount(&mock_server)
                .await;

            let started = tokio::time::Instant::now();
            let result =
                post_to_slack_with_retry_base(&mock_server.uri(), "xoxb-t", "C123", "", "hi").await;
            let waited = started.elapsed();

            assert!(result.is_ok(), "second attempt should succeed: {result:?}");
            assert!(
                waited >= Duration::from_secs(30),
                "loop waited {waited:?}, expected the advised 30s"
            );
        }

        #[test]
        fn advice_wins_over_backoff() {
            let one_second = Duration::from_secs(1);
            assert_eq!(
                retry_wait(
                    &SlackApiError::RateLimited {
                        retry_after: Some(Duration::from_secs(30))
                    },
                    one_second
                ),
                Duration::from_secs(30),
                "a 429 saying 30s must not be retried after 1s"
            );
        }

        #[test]
        fn backoff_applies_without_advice() {
            let backoff = Duration::from_secs(4);
            for error in [
                SlackApiError::RateLimited { retry_after: None },
                SlackApiError::Transient("boom".to_string()),
            ] {
                assert_eq!(retry_wait(&error, backoff), backoff, "{error:?}");
            }
        }

        #[test]
        fn pathological_advice_is_capped() {
            assert_eq!(
                retry_wait(
                    &SlackApiError::RateLimited {
                        retry_after: Some(Duration::from_secs(86_400))
                    },
                    Duration::from_secs(1)
                ),
                MAX_RETRY_AFTER,
                "a delivery task must not be pinned for a day"
            );
        }
    }

    /// EVE-966: a turn that ends without a delivered reply must say so in the
    /// Slack thread exactly once, and must always release its registration.
    mod terminal_state_tests {
        use super::*;
        use crate::storage::StorageBackend;
        use crate::storage::models::{CreateEventRow, CreateSessionRow};
        use everruns_provider::typed_id::PrincipalId;
        use everruns_provider::typed_id::{AgentId, HarnessId};
        use tokio::sync::broadcast;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const ORG: i64 = 30;
        const INPUT_MSG: &str = "msg_turn_one";
        const FRONTEND: &str = "https://app.example.com";
        const CHANNEL: &str = "C_TERMINAL";
        const THREAD_TS: &str = "1700000000.000100";

        pub(super) async fn seed_session(
            db: &StorageBackend,
        ) -> everruns_provider::typed_id::SessionId {
            db.create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                workspace_id: None,
                org_id: ORG,
                app_id: None,
                harness_id: Some(HarnessId::from_uuid(uuid::Uuid::nil())),
                agent_id: Some(AgentId::from_uuid(uuid::Uuid::nil())),
                agent_version_id: None,
                agent_config_hash: None,
                agent_identity_id: None,
                owner_principal_id: PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                title: Some("terminal state test".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::Value::Array(vec![]),
                hints: None,
                max_iterations: None,
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                network_access: None,
                parent_session_id: None,
                budget_root_session_id: None,
            })
            .await
            .expect("create session")
            .id
        }

        pub(super) async fn emit(
            db: &StorageBackend,
            session_id: everruns_provider::typed_id::SessionId,
            event_type: &str,
            input_message_id: &str,
            data: serde_json::Value,
        ) {
            db.create_event(CreateEventRow {
                session_id,
                event_type: event_type.to_string(),
                ts: chrono::Utc::now(),
                context: serde_json::json!({ "input_message_id": input_message_id }),
                data,
                metadata: None,
                tags: None,
            })
            .await
            .expect("create event");
        }

        fn reply_event_data(text: &str) -> serde_json::Value {
            serde_json::json!({
                "message": { "content": [{ "type": "text", "text": text }] }
            })
        }

        /// Slack mock that accepts every post, plus a dispatcher pointed at it.
        async fn dispatcher_against_slack(
            db: Arc<StorageBackend>,
        ) -> (Arc<SlackDeliveryDispatcher>, MockServer) {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({ "ok": true, "ts": "1.2" })),
                )
                .mount(&mock_server)
                .await;

            let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
            let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
                db,
                rx,
                FRONTEND.to_string(),
                Arc::new(SlackDeliveryAdapter::with_api_base(mock_server.uri())),
            );
            (dispatcher, mock_server)
        }

        /// Text of every message the dispatcher posted, in order.
        async fn posted_texts(mock_server: &MockServer) -> Vec<String> {
            mock_server
                .received_requests()
                .await
                .unwrap_or_default()
                .iter()
                .map(|req| {
                    let body: serde_json::Value =
                        serde_json::from_slice(&req.body).expect("slack post body is json");
                    body["text"].as_str().unwrap_or_default().to_string()
                })
                .collect()
        }

        async fn register_turn(dispatcher: &SlackDeliveryDispatcher, session_id: uuid::Uuid) {
            dispatcher
                .register(DeliveryRegistration {
                    session_id,
                    input_message_id: INPUT_MSG.to_string(),
                    bot_token: "xoxb-test-token".to_string(),
                    channel: CHANNEL.to_string(),
                    thread_ts: THREAD_TS.to_string(),
                    reply_mode: SlackReplyMode::AllMessages,
                    surface: SlackSurface::Channel,
                    recipient_user_id: None,
                    recipient_team_id: None,
                })
                .await;
        }

        #[tokio::test]
        async fn turn_failed_without_reply_posts_one_notice() {
            let db = Arc::new(StorageBackend::in_memory());
            let session_id = seed_session(&db).await;
            let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
            register_turn(&dispatcher, session_id.uuid()).await;

            emit(
                &db,
                session_id,
                "turn.failed",
                INPUT_MSG,
                serde_json::json!({ "error": "provider exploded: secret-bearing detail" }),
            )
            .await;
            dispatcher.process_session_events(session_id.uuid()).await;

            let texts = posted_texts(&mock_server).await;
            assert_eq!(texts.len(), 1, "expected exactly one notice, got {texts:?}");
            assert!(
                texts[0].contains("could not finish"),
                "unexpected notice: {}",
                texts[0]
            );
            assert!(
                texts[0].contains(&format!("{FRONTEND}/sessions/{session_id}/chat")),
                "notice must link back to the session: {}",
                texts[0]
            );
            assert!(
                !texts[0].contains("secret-bearing detail"),
                "notice must not leak internal error text: {}",
                texts[0]
            );
            assert_eq!(
                dispatcher.active_delivery_count().await,
                0,
                "a failed turn must release its registration"
            );
        }

        #[tokio::test]
        async fn turn_completed_without_output_posts_notice() {
            let db = Arc::new(StorageBackend::in_memory());
            let session_id = seed_session(&db).await;
            let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
            register_turn(&dispatcher, session_id.uuid()).await;

            emit(
                &db,
                session_id,
                "turn.completed",
                INPUT_MSG,
                serde_json::json!({}),
            )
            .await;
            dispatcher.process_session_events(session_id.uuid()).await;

            let texts = posted_texts(&mock_server).await;
            assert_eq!(texts.len(), 1, "expected exactly one notice, got {texts:?}");
            assert!(
                texts[0].contains("without a reply"),
                "unexpected notice: {}",
                texts[0]
            );
        }

        #[tokio::test]
        async fn delivered_reply_suppresses_notice() {
            let db = Arc::new(StorageBackend::in_memory());
            let session_id = seed_session(&db).await;
            let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
            register_turn(&dispatcher, session_id.uuid()).await;

            emit(
                &db,
                session_id,
                "output.message.completed",
                INPUT_MSG,
                reply_event_data("Here is your answer."),
            )
            .await;
            emit(
                &db,
                session_id,
                "turn.completed",
                INPUT_MSG,
                serde_json::json!({}),
            )
            .await;
            dispatcher.process_session_events(session_id.uuid()).await;

            let texts = posted_texts(&mock_server).await;
            assert_eq!(texts, vec!["Here is your answer.".to_string()]);
            assert_eq!(dispatcher.active_delivery_count().await, 0);
        }

        /// The reply and the terminal event usually arrive in separate
        /// notifications, so `delivered` has to survive between passes or every
        /// answered turn would be chased by a spurious "no reply" notice.
        #[tokio::test]
        async fn reply_in_earlier_pass_still_suppresses_notice() {
            let db = Arc::new(StorageBackend::in_memory());
            let session_id = seed_session(&db).await;
            let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
            register_turn(&dispatcher, session_id.uuid()).await;

            emit(
                &db,
                session_id,
                "output.message.completed",
                INPUT_MSG,
                reply_event_data("Answered early."),
            )
            .await;
            dispatcher.process_session_events(session_id.uuid()).await;

            emit(
                &db,
                session_id,
                "turn.completed",
                INPUT_MSG,
                serde_json::json!({}),
            )
            .await;
            dispatcher.process_session_events(session_id.uuid()).await;

            let texts = posted_texts(&mock_server).await;
            assert_eq!(texts, vec!["Answered early.".to_string()]);
        }

        /// Both cancel paths mint a fresh `input_message_id`, so the delivery used
        /// to sit registered forever and the user was never told.
        #[tokio::test]
        async fn turn_cancelled_notifies_and_unregisters() {
            let db = Arc::new(StorageBackend::in_memory());
            let session_id = seed_session(&db).await;
            let (dispatcher, mock_server) = dispatcher_against_slack(db.clone()).await;
            register_turn(&dispatcher, session_id.uuid()).await;

            emit(
                &db,
                session_id,
                "turn.cancelled",
                "msg_freshly_minted_by_cancel",
                serde_json::json!({ "reason": "User requested cancellation" }),
            )
            .await;
            dispatcher.process_session_events(session_id.uuid()).await;

            let texts = posted_texts(&mock_server).await;
            assert_eq!(texts.len(), 1, "expected exactly one notice, got {texts:?}");
            assert!(
                texts[0].contains("cancelled"),
                "unexpected notice: {}",
                texts[0]
            );
            assert_eq!(
                dispatcher.active_delivery_count().await,
                0,
                "cancelling must not leak the delivery registration"
            );
        }

        /// A reply Slack refused is not a delivered reply: the user still needs to
        /// be told the turn is over.
        #[tokio::test]
        async fn failed_delivery_still_yields_notice() {
            let db = Arc::new(StorageBackend::in_memory());
            let session_id = seed_session(&db).await;

            // Reject every post with a permanent error so no retry budget burns.
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({ "ok": false, "error": "channel_not_found" }),
                ))
                .mount(&mock_server)
                .await;

            let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
            let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
                db.clone(),
                rx,
                FRONTEND.to_string(),
                Arc::new(SlackDeliveryAdapter::with_api_base(mock_server.uri())),
            );
            register_turn(&dispatcher, session_id.uuid()).await;

            emit(
                &db,
                session_id,
                "output.message.completed",
                INPUT_MSG,
                reply_event_data("Answer nobody will see."),
            )
            .await;
            emit(
                &db,
                session_id,
                "turn.completed",
                INPUT_MSG,
                serde_json::json!({}),
            )
            .await;
            dispatcher.process_session_events(session_id.uuid()).await;

            let texts = posted_texts(&mock_server).await;
            assert_eq!(
                texts.len(),
                2,
                "the lost reply and then the notice, got {texts:?}"
            );
            assert!(
                texts[1].contains("without a reply"),
                "unexpected notice: {}",
                texts[1]
            );
            assert_eq!(dispatcher.active_delivery_count().await, 0);
        }

        #[tokio::test]
        async fn notice_without_frontend_url_omits_link() {
            let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
            let dispatcher = SlackDeliveryDispatcher::start(
                Arc::new(StorageBackend::in_memory()),
                rx,
                String::new(),
            );

            let notice = dispatcher.terminal_notice("turn.failed", uuid::Uuid::nil());
            assert_eq!(notice, "The agent could not finish this request.");
        }
    }
}
