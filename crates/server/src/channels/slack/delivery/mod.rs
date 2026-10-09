// Slack delivery dispatcher — event-driven Slack message posting
//
// Design Decision: No delivery deadline. Woken per session by the PostgreSQL
// event listener, or by polling active sessions where none runs (`wake.rs`).
//
// Design Decision: No durable queue for the Slack HTTP call. Events are already
// durably stored — if the server restarts, startup recovery re-registers active
// Slack sessions and replays from the last known event. Retry on transient Slack
// API failures uses simple exponential backoff in-process.
//
// Design Decision: Delivery context is keyed by (session_id, input_message_id)
// to support concurrent turns in the same session.

use async_trait::async_trait;
use everruns_contracts::typed_id::{EventId, SessionId};
use everruns_core::channel::{
    ChannelAgentSurface, ChannelDeliveryAdapter, ChannelStreamDelivery,
    DeliveryContext as ChannelDeliveryContext, DeliveryResult as ChannelDeliveryResult,
    OutboundChannelMessage,
};
mod live_deltas;
mod message_receipts;
mod recovery_endpoint;
mod wake;
use crate::domains::agent_channels::record::{SlackReplyMode, exposure};
use everruns_core::events;
use exposure::{PublicToolVisibility, public_tool_activity_text};
use message_receipts::channel_message_was_delivered;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info, warn};
use uuid::Uuid;
pub use wake::DeliveryWake;

use crate::channels::slack::api::{
    SLACK_API_BASE, post_slack_blocks, post_slack_message_returning_ts, slack_api_call,
    update_slack_message_text,
};
use crate::channels::slack::api_error::{SlackApiError, parse_retry_after, retry_wait};
use crate::listeners::run_summary::is_terminal_turn_event;
use crate::storage::StorageBackend;

mod session_scheduler;
use session_scheduler::SessionDeliveryScheduler;

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

/// Turn-level status: the agent is working and no tool is running.
///
/// Reveals that the turn is alive and nothing else, which is the point — most of
/// the "is it dead?" ambiguity goes away without narrating anything (EVE-975).
const SLACK_THINKING_STATUS: &str = "is thinking...";

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
    /// How much of a running tool the pane's status line may reveal (EVE-975).
    pub tool_visibility: PublicToolVisibility,
    /// Status text for a running tool under `Generic` visibility.
    pub generic_tool_text: String,
    /// Whether this session declared the approval hint (EVE-1025).
    ///
    /// Resolved at registration rather than read per event: a card that cannot
    /// be drawn must degrade to the ask in prose, and deciding that once keeps
    /// the two paths from disagreeing mid-turn.
    pub approvals_enabled: bool,
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
    /// Tool-activity policy for the pane status line.
    tool_visibility: PublicToolVisibility,
    generic_tool_text: String,
    /// Whether this session can render an approval card (EVE-1025).
    approvals_enabled: bool,
    /// Live task fan-out for this turn, rendered into one status message that is
    /// updated in place (EVE-1026). Empty for a turn that delegates nothing,
    /// which is how such a turn gains no status message at all.
    task_progress: crate::channels::slack::task_progress::TaskProgress,
    /// Tools running right now. The status line reverts to the thinking text
    /// when this returns to zero, so two overlapping tools do not clear it early.
    active_tool_count: usize,
    /// Last status pushed to Slack. Slack takes a `setStatus` per call and the
    /// same string twice is a wasted round trip on a rate-limited API.
    last_status: Option<String>,
    /// Open streams for this turn, keyed by output message id. A turn with three
    /// output messages is three streams, not one concatenated blob.
    streams: HashMap<String, StreamState>,
    /// Messages whose open stream was replaced and closed by an output
    /// guardrail. Their subsequent completed event must not post a duplicate.
    replaced_messages: std::collections::HashSet<String>,
    /// Last event ID we've processed (for cursor-based pagination).
    since_event_id: Option<EventId>,
    /// Whether replay has reached this turn. Session-wide cancellation only
    /// applies after this boundary, so an older cancellation cannot terminate a
    /// newly registered follow-up in a reused session.
    turn_boundary_reached: bool,
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

/// Delivers agent output to Slack from event broadcaster notifications, with no
/// fixed deadline.
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
    /// Deltas from the delivery bus where PostgreSQL never sees them (EVE-1211).
    live: live_deltas::LiveDeltas,
}

impl SlackDeliveryDispatcher {
    /// Create and start the dispatcher, woken by `wake` (see [`DeliveryWake`]).
    pub fn start(
        db: Arc<StorageBackend>,
        wake: impl Into<DeliveryWake>,
        frontend_url: String,
    ) -> Arc<Self> {
        Self::start_with_adapter(
            db,
            wake,
            frontend_url,
            Arc::new(SlackDeliveryAdapter::new()),
        )
    }

    /// `start`, with the Slack API base injected. Tests point this at a mock.
    pub fn start_with_adapter(
        db: Arc<StorageBackend>,
        wake: impl Into<DeliveryWake>,
        frontend_url: String,
        adapter: Arc<dyn ChannelDeliveryAdapter>,
    ) -> Arc<Self> {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let dispatcher = Arc::new_cyclic(|weak| Self {
            deliveries: Arc::new(RwLock::new(HashMap::new())),
            active_sessions: Arc::new(RwLock::new(std::collections::HashSet::new())),
            db,
            shutdown_tx,
            frontend_url,
            adapter,
            live: live_deltas::LiveDeltas::new(weak.clone()),
        });

        let (dispatcher_clone, wake) = (dispatcher.clone(), wake.into());
        tokio::spawn(async move {
            dispatcher_clone.event_loop(wake, shutdown_rx).await;
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
            tool_visibility,
            generic_tool_text,
            approvals_enabled,
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
            tool_visibility,
            generic_tool_text,
            approvals_enabled,
            task_progress: Default::default(),
            active_tool_count: 0,
            last_status: None,
            streams: HashMap::new(),
            replaced_messages: std::collections::HashSet::new(),
            since_event_id: None,
            turn_boundary_reached: false,
            delivered: false,
        };

        info!(
            %session_id,
            input_message_id = %ctx.input_message_id,
            ?surface,
            "Registered Slack delivery"
        );

        self.active_sessions.write().await.insert(session_id);
        let mut deliveries = self.deliveries.write().await;
        deliveries.insert(key, ctx);
        self.live.ensure(session_id);
    }

    /// Shutdown the dispatcher.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    async fn event_loop(
        self: Arc<Self>,
        mut wake: DeliveryWake,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        info!(polling = wake.polls(), "Slack delivery dispatcher started");

        let mut flush = tokio::time::interval(STREAM_FLUSH_INTERVAL);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut scheduler = SessionDeliveryScheduler::new();

        loop {
            tokio::select! {
                _ = flush.tick() => {
                    self.flush_open_streams().await;
                    // Same cadence as the stream flush, deliberately: a task
                    // summary and an open reply stream are two things writing to
                    // one thread, and one rhythm keeps their ordering
                    // predictable (EVE-1026).
                    self.flush_task_progress_all().await;
                    if wake.polls() {
                        self.schedule_all_active(&mut scheduler).await;
                    }
                }
                result = wake.recv() => {
                    match result {
                        Ok(session_id) => {
                            if !self.active_sessions.read().await.contains(&session_id) {
                                continue;
                            }
                            scheduler.schedule(self.clone(), session_id);
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(
                                skipped = n,
                                "Slack delivery dispatcher lagged, processing all active sessions"
                            );
                            self.schedule_all_active(&mut scheduler).await;
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("Event broadcaster closed, stopping Slack delivery dispatcher");
                            break;
                        }
                    }
                }
                _ = scheduler.join_next(self.clone()), if !scheduler.is_empty() => {}
                _ = shutdown_rx.changed() => {
                    scheduler.abort_all();
                    break;
                }
            }
        }
        // Either exit leaves nothing to deliver into.
        self.live.drop_all();
    }

    /// Process new events for a session, delivering messages to Slack.
    async fn process_session_events(&self, session_id: Uuid) {
        let session_id_typed = SessionId::from_uuid(session_id);
        let empty: Vec<String> = vec![];
        let _live_order = self.live.order(session_id).await;

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
            let mut turn_boundary_reached = ctx.turn_boundary_reached;
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
                // session-scoped anyway (`TurnBackend::cancel` takes a session), so every
                // delivery on this session is terminal once it arrives. The turn
                // boundary prevents a cancellation from an earlier, persisted turn
                // from matching a later registration that reuses the session.
                let event_input_msg = event
                    .context
                    .get("input_message_id")
                    .and_then(|v| v.as_str());
                let matches_turn = event_input_msg == Some(&ctx.input_message_id);
                turn_boundary_reached |= matches_turn;
                let is_our_turn =
                    matches_turn || (event.event_type == "turn.cancelled" && turn_boundary_reached);

                if !is_our_turn {
                    continue;
                }

                // Progressive delivery for the pane. Deltas never reach the
                // discrete path below: they are accumulated per output message and
                // flushed on a cadence Slack can absorb.
                if streams_supported && ctx.reply_mode == SlackReplyMode::AllMessages {
                    self.live
                        .observe(session_id, &event.event_type, &event.data);
                    if event.event_type == events::OUTPUT_MESSAGE_REPLACED
                        && let (Some(message_id), Some(replacement)) = (
                            event.data.get("message_id").and_then(|v| v.as_str()),
                            event.data.get("replacement").and_then(|v| v.as_str()),
                        )
                        && self
                            .replace_stream(&key, &ctx, message_id, replacement)
                            .await
                    {
                        delivered = true;
                        continue;
                    }

                    if event.event_type == "output.message.delta" {
                        self.stream_delta(&key, &ctx, &event.data).await;
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
                        if self
                            .deliveries
                            .write()
                            .await
                            .get_mut(&key)
                            .is_some_and(|live| live.replaced_messages.remove(message_id))
                        {
                            delivered = true;
                            continue;
                        }

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
                    match self
                        .post(&ctx, session_id, Some(key.input_message_id.clone()), text)
                        .await
                    {
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

                // The tool already sent its message. Observe the receipt for
                // terminal-state bookkeeping without forwarding the content twice.
                if event.event_type == events::TOOL_COMPLETED
                    && channel_message_was_delivered(&event.data)
                {
                    delivered = true;
                }

                // Pane status line. Tool lifecycle is counted rather than
                // toggled: two overlapping tools must not have the first one to
                // finish clear the status while the second is still running.
                match event.event_type.as_str() {
                    events::TURN_STARTED => {
                        self.set_status(&key, &ctx, SLACK_THINKING_STATUS).await;
                    }
                    // `tool.completed` is emitted for a failed call too — there is
                    // no `tool.failed` — so the counter cannot strand above zero.
                    events::TOOL_STARTED | events::TOOL_COMPLETED => {
                        // EVE-1025: a `request_approval` that is actually
                        // waiting becomes buttons in the thread. Posted here
                        // rather than at turn end because the pause *is* the
                        // end of the turn, and the card should land with the
                        // ask rather than after the terminal notice.
                        if event.event_type == events::TOOL_COMPLETED
                            && self.post_approval_card(&ctx, session_id, &event.data).await
                        {
                            delivered = true;
                        }
                        if let Some(live) = self.deliveries.write().await.get_mut(&key) {
                            live.active_tool_count = if event.event_type == events::TOOL_STARTED {
                                live.active_tool_count + 1
                            } else {
                                live.active_tool_count.saturating_sub(1)
                            };
                        }
                        let status = self.current_status(&key).await;
                        self.set_status(&key, &ctx, &status).await;
                    }
                    // The agent names the session once it knows what the thread is
                    // about. That title is worth showing; the synthetic seed title
                    // (`Slack thread <ts> in <channel>`) is the thread's own
                    // coordinates and would tell the reader nothing.
                    // EVE-1026: fold the fan-out in, but do not push. The flush
                    // tick decides when, so twenty workers settling at once cost
                    // one `chat.update`, not twenty.
                    events::TASK_CREATED | events::TASK_UPDATED => {
                        if let Some(task) = event.data.get("task")
                            && let (Some(id), Some(name), Some(state)) = (
                                task.get("id").and_then(|v| v.as_str()),
                                task.get("display_name").and_then(|v| v.as_str()),
                                task.get("state").and_then(|v| v.as_str()),
                            )
                            && let Some(state) =
                                everruns_core::session_task::SessionTaskState::parse(state)
                            && let Some(live) = self.deliveries.write().await.get_mut(&key)
                        {
                            live.task_progress.observe(id, name, state);
                        }
                    }
                    events::SESSION_TITLE_UPDATED => {
                        if let Some(title) = event.data.get("title").and_then(|v| v.as_str())
                            && !title.trim().is_empty()
                        {
                            self.set_title(&key, &ctx, title).await;
                        }
                    }
                    _ => {}
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

            // Every terminal state clears the status line. A status left set is
            // the same failure mode as an unstopped stream: the pane keeps saying
            // the agent is working long after it stopped.
            if terminal_event.is_some() {
                self.set_status(&key, &ctx, "").await;
                // A summary frozen mid-flight is worse than none: it reads as
                // live forever. The final push says where the fan-out actually
                // got to (EVE-1026).
                if self.flush_task_progress(&key, true).await {
                    delivered = true;
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
                if !delivered
                    || (ctx.reply_mode == SlackReplyMode::ToolOnly
                        && matches!(event_type.as_str(), "turn.failed" | "turn.cancelled"))
                {
                    let notice = self.terminal_notice(&event_type, session_id);
                    match self
                        .post(&ctx, session_id, Some(key.input_message_id.clone()), notice)
                        .await
                    {
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
            } else if new_since_id != ctx.since_event_id
                || delivered != ctx.delivered
                || turn_boundary_reached != ctx.turn_boundary_reached
            {
                // Update the cursor
                // Stream state lives in the shared map and is never written back
                // from a clone, so only scalar replay state moves here.
                let mut deliveries = self.deliveries.write().await;
                if let Some(live) = deliveries.get_mut(&key) {
                    live.since_event_id = new_since_id;
                    live.turn_boundary_reached = turn_boundary_reached;
                    live.delivered = delivered;
                }
            }
        }
    }

    /// The agent-surface interface to use for this delivery, if any.
    ///
    /// Pane-only, for the same reason streaming is: a channel thread has no
    /// status line or title to set, and pushing one would be a no-op call per
    /// tool on a rate-limited API.
    fn agent_surface_for(&self, ctx: &DeliveryContext) -> Option<&dyn ChannelAgentSurface> {
        if ctx.surface != SlackSurface::Pane {
            return None;
        }
        self.adapter.agent_surface()
    }

    /// Push a status line, skipping the call when it would not change anything.
    ///
    /// Advisory: a failed status is logged and swallowed. The reply is the
    /// product; a decoration must never take a turn down with it.
    async fn set_status(&self, key: &DeliveryKey, ctx: &DeliveryContext, status: &str) {
        let Some(surface) = self.agent_surface_for(ctx) else {
            return;
        };
        {
            let deliveries = self.deliveries.read().await;
            let live = match deliveries.get(key) {
                Some(live) => live,
                None => return,
            };
            if live.last_status.as_deref() == Some(status) {
                return;
            }
        }
        if let ChannelDeliveryResult::TransientError(e) | ChannelDeliveryResult::PermanentError(e) =
            surface
                .set_status(status, &self.delivery_context(ctx))
                .await
        {
            warn!(session_id = %key.session_id, error = %e, "Failed to set Slack thread status");
            return;
        }
        if let Some(live) = self.deliveries.write().await.get_mut(key) {
            live.last_status = Some(status.to_string());
        }
    }

    /// Push a thread title. Advisory, like `set_status`.
    async fn set_title(&self, key: &DeliveryKey, ctx: &DeliveryContext, title: &str) {
        let Some(surface) = self.agent_surface_for(ctx) else {
            return;
        };
        if let ChannelDeliveryResult::TransientError(e) | ChannelDeliveryResult::PermanentError(e) =
            surface.set_title(title, &self.delivery_context(ctx)).await
        {
            warn!(session_id = %key.session_id, error = %e, "Failed to set Slack thread title");
        }
    }

    /// The status a delivery should be showing right now.
    ///
    /// A running tool narrates only as far as the channel's visibility allows;
    /// `None` visibility falls back to the turn-level thinking text, which
    /// reveals that work is happening but nothing about what.
    async fn current_status(&self, key: &DeliveryKey) -> String {
        let deliveries = self.deliveries.read().await;
        let Some(live) = deliveries.get(key) else {
            return SLACK_THINKING_STATUS.to_string();
        };
        if live.active_tool_count == 0 {
            return SLACK_THINKING_STATUS.to_string();
        }
        public_tool_activity_text(live.tool_visibility, &live.generic_tool_text)
            .unwrap_or(SLACK_THINKING_STATUS)
            .to_string()
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

    /// Flush every open stream across every delivery. Driven by the timer.
    /// Push one delivery's task summary, posting it the first time and updating
    /// in place after that.
    ///
    /// Returns whether Slack accepted something, so a turn whose only output was
    /// its fan-out is not also given the "nothing came back" notice.
    async fn flush_task_progress(&self, key: &DeliveryKey, final_state: bool) -> bool {
        let Some((ctx, text, existing_ts)) = ({
            let deliveries = self.deliveries.read().await;
            deliveries.get(key).and_then(|ctx| {
                // Nothing delegated, or nothing changed since the last push.
                if ctx.task_progress.is_empty() || !(ctx.task_progress.is_dirty() || final_state) {
                    return None;
                }
                Some((
                    ctx.clone(),
                    ctx.task_progress.render(final_state),
                    ctx.task_progress.message_ts().map(str::to_string),
                ))
            })
        }) else {
            return false;
        };

        match existing_ts {
            Some(ts) => {
                match update_slack_message_text(&ctx.bot_token, &ctx.channel, &ts, &text).await {
                    Ok(()) => {
                        if let Some(live) = self.deliveries.write().await.get_mut(key) {
                            live.task_progress.mark_updated();
                        }
                        true
                    }
                    Err(error) => {
                        warn!(%error, "Failed to update the Slack task summary");
                        false
                    }
                }
            }
            None => {
                match post_slack_message_returning_ts(
                    &ctx.bot_token,
                    &ctx.channel,
                    &ctx.thread_ts,
                    &text,
                )
                .await
                {
                    Ok(ts) => {
                        if let Some(live) = self.deliveries.write().await.get_mut(key) {
                            live.task_progress.mark_posted(ts);
                        }
                        true
                    }
                    Err(error) => {
                        warn!(%error, "Failed to post the Slack task summary");
                        false
                    }
                }
            }
        }
    }

    /// Push every delivery whose task summary changed since the last tick.
    async fn flush_task_progress_all(&self) {
        let keys: Vec<DeliveryKey> = {
            let deliveries = self.deliveries.read().await;
            deliveries
                .iter()
                .filter(|(_, ctx)| !ctx.task_progress.is_empty() && ctx.task_progress.is_dirty())
                .map(|(key, _)| key.clone())
                .collect()
        };
        for key in keys {
            self.flush_task_progress(&key, false).await;
        }
    }

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
    /// Post an approval card, if this event is one and this thread can draw it.
    ///
    /// Returns whether something was delivered, so a turn that ends on a pause
    /// is not also given the "nothing came back" notice.
    ///
    /// Without the hint, or without a requester to bind the card to, the ask is
    /// posted as prose instead. That is the documented degradation rather than
    /// a failure: the model asked, the thread shows the question, and the human
    /// answers by replying — which is exactly what happens today.
    async fn post_approval_card(
        &self,
        ctx: &DeliveryContext,
        session_id: Uuid,
        data: &serde_json::Value,
    ) -> bool {
        use crate::channels::slack::approvals::{
            ApprovalBinding, approval_fallback_text, approval_turn_id, build_approval_blocks,
            extract_approval_request,
        };

        let Some(request) = extract_approval_request(data) else {
            return false;
        };

        let blocks = if ctx.approvals_enabled {
            ctx.recipient_user_id.as_ref().and_then(|requester| {
                build_approval_blocks(
                    &request,
                    &ApprovalBinding::for_new_card(
                        session_id,
                        requester,
                        approval_turn_id(data),
                        request.action.clone(),
                    ),
                )
            })
        } else {
            None
        };

        let text = approval_fallback_text(&request);
        let result = match blocks {
            Some(blocks) => {
                post_slack_blocks(&ctx.bot_token, &ctx.channel, &ctx.thread_ts, &text, &blocks)
                    .await
            }
            None => post_to_slack(&ctx.bot_token, &ctx.channel, &ctx.thread_ts, &text)
                .await
                .map_err(|error| SlackApiError::Transient(error.to_string())),
        };

        match result {
            Ok(()) => true,
            Err(error) => {
                error!(%session_id, %error, "Failed to post the Slack approval ask");
                false
            }
        }
    }

    async fn post(
        &self,
        ctx: &DeliveryContext,
        session_id: Uuid,
        input_message_id: Option<String>,
        text: String,
    ) -> ChannelDeliveryResult {
        let message = OutboundChannelMessage {
            session_id: SessionId::from_uuid(session_id),
            text,
            thread_ref: ctx.thread_ts.clone(),
            correlation_id: input_message_id,
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
        let mut deliveries = self.deliveries.write().await;
        deliveries.remove(key);
        let has_others = deliveries.keys().any(|k| k.session_id == session_id);
        if !has_others {
            self.live.drop_session(session_id);
        }
        drop(deliveries);

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
    /// Finds active Slack sessions through their endpoint or archival App,
    /// checks the current configuration, and re-registers unfinished turns.
    pub async fn recover(
        &self,
        encryption: Option<&std::sync::Arc<crate::storage::EncryptionService>>,
    ) {
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
            let slack_config = match recovery_endpoint::configuration(&self.db, encryption, session)
                .await
            {
                Ok(Some(config)) => config,
                Ok(None) => continue,
                Err(error) => {
                    warn!(%error, session_id = %session.id, "Failed to load Slack endpoint for recovery");
                    continue;
                }
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
                .and_then(|m| m.get("slack_thread_ts").or_else(|| m.get("slack_ts")))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            // Persisted at input.message time so a restart can still name the
            // stream recipient (EVE-974). Absent on turns registered before that
            // field existed, which simply means no streaming for those.
            // Native recovery shares posting's signed-input authorization;
            // caller-authored API metadata cannot redirect lifecycle feedback.
            let (channel, thread_ts) = if session.app_id.is_none() {
                match recovery_endpoint::trusted_native_route(
                    &self.db,
                    encryption,
                    session,
                    &input_message_id,
                )
                .await
                {
                    Ok(route) => route,
                    Err(error) => {
                        warn!(%error, session_id = %session.id, "Slack recovery input has no trusted route");
                        continue;
                    }
                }
            } else {
                (channel, thread_ts)
            };

            let recipient_user_id = metadata
                .and_then(|m| m.get("slack_user"))
                .and_then(|v| v.as_str())
                .map(str::to_string);

            // Check if this turn already reached a terminal state.
            //
            // EVE-988: terminal state is interpreted here and in
            // `process_session_events`, and only the live path learned about
            // cancellation. Share the list rather than keep a second copy that
            // drifts again.
            let turn_events: Vec<String> = crate::listeners::run_summary::TERMINAL_TURN_EVENTS
                .iter()
                .map(|event_type| (*event_type).to_string())
                .collect();
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
                // `turn.cancelled` is session-scoped, not turn-scoped: both cancel
                // paths mint a fresh `input_message_id` for the synthetic event
                // because neither knows the in-flight turn's id, so a per-turn
                // match would never fire. Same rule the live path applies.
                e.event_type == "turn.cancelled"
                    || e.context.get("input_message_id").and_then(|v| v.as_str())
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
                approvals_enabled: crate::channels::slack::approvals::approvals_enabled(
                    session.hints.as_ref(),
                ),
                bot_token: slack_config.bot_token.clone(),
                channel,
                thread_ts,
                reply_mode: slack_config.reply_mode,
                surface,
                recipient_user_id: recipient_user_id.filter(|u| !u.is_empty()),
                recipient_team_id: slack_config.team_id.clone(),
                tool_visibility: slack_config.tool_visibility,
                generic_tool_text: slack_config.generic_tool_text.clone(),
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
        // Correlation comes from the message rather than the context: the
        // context is per-registration, the ids identify this one reply.
        let correlation =
            message
                .correlation_id
                .as_ref()
                .map(|input_message_id| SlackCorrelation {
                    session_id: message.session_id.to_string(),
                    input_message_id: input_message_id.clone(),
                });

        match post_to_slack_with_retry_base(
            &self.api_base,
            &context.auth_token,
            &context.channel_id,
            &context.thread_ref,
            &message.text,
            correlation.as_ref(),
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

    fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
        Some(self)
    }

    fn agent_surface(&self) -> Option<&dyn ChannelAgentSurface> {
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

mod streams;

/// Map a Slack transport failure onto a `ChannelDeliveryResult`.
/// Slack's agent-surface methods, in one place.
///
/// Slack documents `assistant.threads.setStatus` / `setTitle` as compatibility
/// bridges over these, so both name sets work today and one of them will not.
/// Pinning the `agents.sessions.*` set here — and nowhere else — makes a move a
/// two-line change rather than a search (EVE-975).
const SLACK_SET_STATUS_METHOD: &str = "agents.sessions.setStatus";
const SLACK_SET_TITLE_METHOD: &str = "agents.sessions.rename";

/// Slack rejects a title longer than this.
const SLACK_TITLE_MAX_CHARS: usize = 250;

#[async_trait]
impl ChannelAgentSurface for SlackDeliveryAdapter {
    async fn set_status(
        &self,
        status: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        // The pane keys status off the thread, so a thread_ts is required; a
        // channel post has none and has no status line to set either.
        if context.thread_ref.is_empty() {
            return ChannelDeliveryResult::Ok;
        }
        let payload = serde_json::json!({
            "channel_id": context.channel_id,
            "thread_ts": context.thread_ref,
            "status": status,
        });
        match slack_api_call(
            &self.api_base,
            &context.auth_token,
            SLACK_SET_STATUS_METHOD,
            payload,
        )
        .await
        {
            Ok(_) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }

    async fn set_title(
        &self,
        title: &str,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        if context.thread_ref.is_empty() {
            return ChannelDeliveryResult::Ok;
        }
        let payload = serde_json::json!({
            "channel_id": context.channel_id,
            "thread_ts": context.thread_ref,
            "title": truncate_chars(title, SLACK_TITLE_MAX_CHARS),
        });
        match slack_api_call(
            &self.api_base,
            &context.auth_token,
            SLACK_SET_TITLE_METHOD,
            payload,
        )
        .await
        {
            Ok(_) => ChannelDeliveryResult::Ok,
            Err(e) => classify_slack_failure(e),
        }
    }
}

fn classify_slack_failure(error: SlackApiError) -> ChannelDeliveryResult {
    match error {
        SlackApiError::Permanent(message) => ChannelDeliveryResult::PermanentError(message),
        other => ChannelDeliveryResult::TransientError(other.to_string()),
    }
}

/// Extract text content from an output.message.completed event's data.
///
/// Blank parts are not reply text: a tool-calling step can carry an empty text
/// part, and posting it makes Slack refuse the message with `no_text`.
pub(crate) fn extract_response_text(data: &serde_json::Value) -> Option<String> {
    let message = data.get("message")?;
    let content = message.get("content")?.as_array()?;

    let mut text_parts = Vec::new();
    for part in content {
        if part.get("type")?.as_str()? == "text"
            && let Some(text) = part.get("text").and_then(|t| t.as_str())
            && !text.trim().is_empty()
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

pub(crate) fn extract_delivery_text(
    event_type: &str,
    reply_mode: SlackReplyMode,
    data: &serde_json::Value,
) -> Option<String> {
    match reply_mode {
        SlackReplyMode::AllMessages if event_type == "output.message.completed" => {
            extract_response_text(data)
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
    correlation: Option<&SlackCorrelation>,
) -> Result<(), SlackApiError> {
    let max_attempts = 3;
    let mut backoff = std::time::Duration::from_secs(1);

    for attempt in 1..=max_attempts {
        match post_slack_message(base_url, bot_token, channel, thread_ts, text, correlation).await {
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

mod markdown_split;
use markdown_split::{
    SLACK_MARKDOWN_BLOCK_LIMIT, SLACK_MAX_BLOCKS_PER_MESSAGE, SLACK_MAX_OUTBOUND_REPLY_CHARS,
    split_markdown_for_blocks, truncate_chars,
};

/// Cap on the `text` notification fallback.
///
/// `text` is not rendered when `blocks` are present — it is what Slack shows in
/// push notifications and the channel list — so it only needs enough to be
/// recognisable, and Slack rejects the whole post if it runs long.
const SLACK_TEXT_FALLBACK_LIMIT: usize = 3_000;

/// Correlation stamped onto a posted message via `chat.postMessage`'s
/// `metadata`, giving a durable key from a Slack message back to the run that
/// produced it — no tag-string heuristics required.
#[derive(Debug, Clone)]
pub(crate) struct SlackCorrelation {
    pub session_id: String,
    pub input_message_id: String,
}

impl SlackCorrelation {
    fn to_metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "event_type": "everruns_agent_reply",
            "event_payload": {
                "session_id": self.session_id,
                "input_message_id": self.input_message_id,
            }
        })
    }
}

/// Build the `chat.postMessage` payloads for one reply.
///
/// Normally one payload. Replies are bounded to one full Slack message's worth
/// of source text before splitting, which keeps synchronous payload allocation
/// finite while still allowing fence continuations to spill into another API
/// call when their added markers cross the 50-block boundary.
pub(crate) fn build_post_payloads(
    channel: &str,
    thread_ts: &str,
    text: &str,
    correlation: Option<&SlackCorrelation>,
) -> Vec<serde_json::Value> {
    let text = truncate_chars(text, SLACK_MAX_OUTBOUND_REPLY_CHARS);
    let chunks = split_markdown_for_blocks(text, SLACK_MARKDOWN_BLOCK_LIMIT);

    chunks
        .chunks(SLACK_MAX_BLOCKS_PER_MESSAGE)
        .map(|group| {
            let blocks: Vec<serde_json::Value> = group
                .iter()
                .map(|chunk| serde_json::json!({ "type": "markdown", "text": chunk }))
                .collect();

            let mut payload = serde_json::json!({
                "channel": channel,
                // Notification fallback only; `blocks` is what renders.
                "text": truncate_chars(text, SLACK_TEXT_FALLBACK_LIMIT),
                "blocks": blocks,
            });

            if !thread_ts.is_empty() {
                payload["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
            }
            if let Some(correlation) = correlation {
                payload["metadata"] = correlation.to_metadata();
            }
            payload
        })
        .collect()
}

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
    post_slack_message(base_url, bot_token, channel, thread_ts, text, None).await
}

/// Post one reply, as one or more `chat.postMessage` calls.
///
/// Every call renders through `markdown` blocks, so agent output reaches Slack
/// as the Markdown it actually is rather than being reinterpreted as the much
/// smaller `mrkdwn` dialect.
pub(crate) async fn post_slack_message(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
    correlation: Option<&SlackCorrelation>,
) -> Result<(), SlackApiError> {
    let client = reqwest::Client::new();
    let payloads = build_post_payloads(channel, thread_ts, text, correlation);
    let total = payloads.len();

    for (index, payload) in payloads.into_iter().enumerate() {
        let response = client
            .post(format!("{}/chat.postMessage", base_url))
            .header("Authorization", format!("Bearer {}", bot_token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| SlackApiError::Transient(e.to_string()))?;

        let status = response.status();
        // Read the header before the body is consumed: a 429 carries its advice
        // here, not in the JSON.
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
                    channel = channel,
                    retry_after_secs = ?retry_after.map(|d| d.as_secs()),
                    status = %status,
                    part = index + 1,
                    parts = total,
                    "Slack rate limited the post"
                );
            } else {
                error!(
                    channel = channel,
                    error = error,
                    status = %status,
                    part = index + 1,
                    parts = total,
                    "Failed to post message to Slack"
                );
            }
            return Err(failure);
        }
    }

    info!(channel = channel, parts = total, "Posted response to Slack");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::domains::agent_channels::record::DEFAULT_AG_UI_GENERIC_TOOL_TEXT;
    use crate::live_updates::event_notifications::EventNotificationPayload;
    mod concurrency_tests;
    mod live_delta_tests;
    mod polling_wake_tests;
    mod response_text_tests;

    use super::*;

    #[test]
    fn explicit_message_receipts_are_observed_but_never_forwarded() {
        let mut receipt = serde_json::json!({
            "tool_name": "channel_post_message", "success": true,
            "result": [{"type":"text", "text": r#"{"delivered":true,"platform":"slack","channel":"C1","message_ref":"2.3"}"#}]
        });
        assert!(channel_message_was_delivered(&receipt));
        for mode in [SlackReplyMode::AllMessages, SlackReplyMode::ToolOnly] {
            assert_eq!(
                extract_delivery_text("tool.completed", mode, &receipt),
                None
            );
        }
        receipt["success"] = serde_json::json!(false);
        assert!(!channel_message_was_delivered(&receipt));
        receipt["success"] = serde_json::json!(true);
        receipt["result"][0]["text"] =
            serde_json::json!(r#"{"delivered":false,"platform":"slack","message_ref":"2.3"}"#);
        assert!(!channel_message_was_delivered(&receipt));
        assert!(!channel_message_was_delivered(&serde_json::json!({})));
    }

    #[test]
    fn assistant_output_is_only_forwarded_in_automatic_mode() {
        let output = serde_json::json!({"message":{"content":[{"type":"text","text":"Normal assistant reply"}]}});
        assert_eq!(
            extract_delivery_text(
                "output.message.completed",
                SlackReplyMode::AllMessages,
                &output
            ),
            Some("Normal assistant reply".into())
        );
        assert_eq!(
            extract_delivery_text(
                "output.message.completed",
                SlackReplyMode::ToolOnly,
                &output
            ),
            None
        );
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
                None,
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
                None,
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
                None,
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
                None,
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
                None,
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
        use crate::storage::{CreateAppRow, CreateSessionRow, UpdateSession};
        use everruns_contracts::typed_id::PrincipalId;
        use everruns_contracts::typed_id::{AgentId, HarnessId};
        use tokio::sync::broadcast;

        const ORG_APP_OWNER: i64 = 10;
        const ORG_ATTACKER: i64 = 20;
        const LEGIT_APP_PUBLIC_ID: &str = "app_000000000000000000000000000000ee";

        async fn seed_slack_app(db: &StorageBackend, org_id: i64, public_id: &str) -> uuid::Uuid {
            db.create_app(
                org_id,
                CreateAppRow {
                    public_id: public_id.to_string(),
                    name: "Legit Slack App".to_string(),
                    description: None,
                    harness_id: uuid::Uuid::nil(),
                    agent_id: Some(uuid::Uuid::nil()),
                    virtual_user_id: None,
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
        ) -> everruns_contracts::typed_id::SessionId {
            use crate::storage::CreateEventRow;

            let session = db
                .create_session(CreateSessionRow {
                    org_id,
                    app_id,
                    harness_id: Some(HarnessId::from_uuid(uuid::Uuid::nil())),
                    agent_id: Some(AgentId::from_uuid(uuid::Uuid::nil())),
                    owner_principal_id: PrincipalId::from_seed(1),
                    title: Some("recovery test".to_string()),
                    tags: vec![format!("slack:app:{app_public_id}")],
                    ..Default::default()
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
                            "slack_thread_ts": "1234.0000",
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
            let db = Arc::new(StorageBackend::test_database());
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
            let db = Arc::new(StorageBackend::test_database());
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

        /// EVE-988: a turn cancelled before a restart is finished. Nothing will
        /// ever emit another terminal event for it, so re-registering a delivery
        /// leaks the registration and its `active_sessions` entry until the next
        /// restart — the leak EVE-966 fixed on the live path, on the recovery path.
        #[tokio::test]
        async fn recover_treats_a_cancelled_turn_as_finished() {
            use crate::storage::CreateEventRow;

            let db = Arc::new(StorageBackend::test_database());
            let app_id = seed_slack_app(&db, ORG_APP_OWNER, LEGIT_APP_PUBLIC_ID).await;
            let session_id = seed_active_slack_delivery_session(
                &db,
                ORG_APP_OWNER,
                Some(app_id),
                LEGIT_APP_PUBLIC_ID,
            )
            .await;

            // Production shape: both cancel paths build the synthetic event with a
            // fresh `MessageId::new()`, so it does not carry the cancelled turn's
            // own `input_message_id`. Adding `turn.cancelled` to the queried event
            // types is not enough on its own — the match has to be session-scoped.
            db.create_event(CreateEventRow {
                session_id,
                event_type: "turn.cancelled".to_string(),
                ts: chrono::Utc::now(),
                context: serde_json::json!({ "input_message_id": "msg_fresh_cancel_id" }),
                data: serde_json::json!({ "reason": "stopped from Slack" }),
                metadata: None,
                tags: None,
            })
            .await
            .expect("seed turn.cancelled");

            let dispatcher = build_dispatcher(db);
            dispatcher.recover(None).await;

            assert_eq!(
                dispatcher.active_delivery_count().await,
                0,
                "a cancelled last turn must not be re-registered on recovery"
            );
        }

        #[tokio::test]
        async fn recover_uses_session_app_id_instead_of_slack_app_tag() {
            let db = Arc::new(StorageBackend::test_database());
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
            assert_eq!(
                dispatcher
                    .deliveries
                    .read()
                    .await
                    .values()
                    .next()
                    .unwrap()
                    .thread_ts,
                "1234.0000"
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
            sent: Arc<Mutex<Vec<String>>>,
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
                    .push(message.text.clone());
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
        }

        #[tokio::test]
        async fn dispatcher_delivers_through_the_adapter() {
            let db = Arc::new(StorageBackend::test_database());
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
                    tool_visibility: PublicToolVisibility::default(),
                    generic_tool_text: DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string(),
                    approvals_enabled: true,
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
            assert_eq!(sent, vec!["Routed reply.".to_string()]);
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
                correlation_id: None,
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
        pub(super) enum Call {
            Start,
            Append(String, String),
            Replace(String, String),
            Stop(String),
            Discrete(String),
        }

        pub(super) struct RecordingAdapter {
            calls: Arc<Mutex<Vec<Call>>>,
            /// When false, `start` fails so the fallback path can be exercised.
            can_start: bool,
        }

        impl RecordingAdapter {
            pub(super) fn new(calls: Arc<Mutex<Vec<Call>>>) -> Self {
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

            async fn replace(
                &self,
                handle: &str,
                text: &str,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
                self.push(Call::Replace(handle.to_string(), text.to_string()));
                ChannelDeliveryResult::Ok
            }
        }

        pub(super) async fn dispatcher_with(
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
                    tool_visibility: PublicToolVisibility::default(),
                    generic_tool_text: DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string(),
                    approvals_enabled: true,
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

        async fn replaced(
            db: &StorageBackend,
            session: SessionId,
            message_id: &str,
            replacement: &str,
        ) {
            terminal_state_tests::emit(
                db,
                session,
                events::OUTPUT_MESSAGE_REPLACED,
                "msg_in",
                serde_json::json!({
                    "message_id": message_id,
                    "replacement": replacement,
                }),
            )
            .await;
        }

        pub(super) fn recorded(calls: &Arc<Mutex<Vec<Call>>>) -> Vec<Call> {
            calls.lock().expect("recorder").clone()
        }

        #[tokio::test]
        async fn pane_reply_streams_then_stops() {
            let db = Arc::new(StorageBackend::test_database());
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

        #[tokio::test]
        async fn guardrail_replacement_removes_streamed_text() {
            let db = Arc::new(StorageBackend::test_database());
            let session = terminal_state_tests::seed_session(&db).await;
            let calls = Arc::new(Mutex::new(Vec::new()));
            let dispatcher =
                dispatcher_with(db.clone(), RecordingAdapter::new(calls.clone())).await;
            register(&dispatcher, session.uuid(), SlackSurface::Pane).await;

            delta(&db, session, "m1", "protected output").await;
            dispatcher.process_session_events(session.uuid()).await;
            dispatcher.flush_open_streams().await;
            replaced(&db, session, "m1", "Response blocked").await;
            completed(&db, session, "m1", "Response blocked").await;
            dispatcher.process_session_events(session.uuid()).await;

            assert_eq!(
                recorded(&calls),
                vec![
                    Call::Start,
                    Call::Append("stream-1".to_string(), "protected output".to_string()),
                    Call::Replace("stream-1".to_string(), "Response blocked".to_string()),
                ],
                "the platform-visible stream must contain only the safe replacement"
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
            let db = Arc::new(StorageBackend::test_database());
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
            let db = Arc::new(StorageBackend::test_database());
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
            let db = Arc::new(StorageBackend::test_database());
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
                let db = Arc::new(StorageBackend::test_database());
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
            let db = Arc::new(StorageBackend::test_database());
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
            let db = Arc::new(StorageBackend::test_database());
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
                post_to_slack_with_retry_base(&mock_server.uri(), "xoxb-t", "C123", "", "hi", None)
                    .await;
            let waited = started.elapsed();

            assert!(result.is_ok(), "second attempt should succeed: {result:?}");
            assert!(
                waited >= Duration::from_secs(30),
                "loop waited {waited:?}, expected the advised 30s"
            );
        }
    }

    /// EVE-966: a turn that ends without a delivered reply must say so in the
    /// Slack thread exactly once, and must always release its registration.
    /// EVE-975: the pane's live status line, driven by turn and tool lifecycle.
    ///
    /// What may be shown is not decided here — `public_tool_activity_text` in
    /// `crate::domains::agent_channels::record::exposure` owns that for every public surface, and AG-UI
    /// reads the same function. These tests pin that the dispatcher asks it and
    /// honours the answer.
    mod agent_surface_tests {
        use super::*;
        use crate::storage::StorageBackend;
        use std::sync::Mutex;
        use tokio::sync::broadcast;

        #[derive(Debug, Clone, PartialEq, Eq)]
        enum Surfaced {
            Status(String),
            Title(String),
        }

        struct RecordingSurface {
            calls: Arc<Mutex<Vec<Surfaced>>>,
        }

        #[async_trait]
        impl ChannelDeliveryAdapter for RecordingSurface {
            fn platform(&self) -> &str {
                "recording-surface"
            }
            async fn deliver(
                &self,
                _message: &OutboundChannelMessage,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
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
            fn agent_surface(&self) -> Option<&dyn ChannelAgentSurface> {
                Some(self)
            }
        }

        #[async_trait]
        impl ChannelAgentSurface for RecordingSurface {
            async fn set_status(
                &self,
                status: &str,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
                self.calls
                    .lock()
                    .expect("recorder")
                    .push(Surfaced::Status(status.to_string()));
                ChannelDeliveryResult::Ok
            }
            async fn set_title(
                &self,
                title: &str,
                _context: &ChannelDeliveryContext,
            ) -> ChannelDeliveryResult {
                self.calls
                    .lock()
                    .expect("recorder")
                    .push(Surfaced::Title(title.to_string()));
                ChannelDeliveryResult::Ok
            }
        }

        /// Drive one turn's lifecycle and return everything that reached the surface.
        async fn surfaced_for(
            surface: SlackSurface,
            tool_visibility: PublicToolVisibility,
            events: &[(&str, serde_json::Value)],
        ) -> Vec<Surfaced> {
            surfaced_for_mode(
                surface,
                tool_visibility,
                SlackReplyMode::AllMessages,
                events,
            )
            .await
        }

        async fn surfaced_for_mode(
            surface: SlackSurface,
            tool_visibility: PublicToolVisibility,
            reply_mode: SlackReplyMode,
            events: &[(&str, serde_json::Value)],
        ) -> Vec<Surfaced> {
            let db = Arc::new(StorageBackend::test_database());
            let session = terminal_state_tests::seed_session(&db).await;
            let calls = Arc::new(Mutex::new(Vec::new()));
            let (_tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
            let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
                db.clone(),
                rx,
                "https://app.example.com".to_string(),
                Arc::new(RecordingSurface {
                    calls: calls.clone(),
                }),
            );
            dispatcher
                .register(DeliveryRegistration {
                    session_id: session.uuid(),
                    input_message_id: "msg_turn_one".to_string(),
                    bot_token: "xoxb-t".to_string(),
                    channel: "D_PANE".to_string(),
                    thread_ts: "1700000000.000100".to_string(),
                    reply_mode,
                    surface,
                    recipient_user_id: Some("U_HUMAN".to_string()),
                    recipient_team_id: Some("T_TEAM".to_string()),
                    tool_visibility,
                    generic_tool_text: DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string(),
                    approvals_enabled: true,
                })
                .await;

            for (event_type, data) in events {
                terminal_state_tests::emit(&db, session, event_type, "msg_turn_one", data.clone())
                    .await;
            }
            dispatcher.process_session_events(session.uuid()).await;

            let recorded = calls.lock().expect("recorder");
            recorded.clone()
        }

        #[tokio::test]
        async fn agent_controlled_mode_keeps_working_feedback_and_clears_it() {
            let surfaced = surfaced_for_mode(
                SlackSurface::Pane,
                PublicToolVisibility::Generic,
                SlackReplyMode::ToolOnly,
                &tool_lifecycle(),
            )
            .await;
            assert_eq!(
                surfaced.first(),
                Some(&Surfaced::Status(SLACK_THINKING_STATUS.into()))
            );
            assert!(
                surfaced
                    .iter()
                    .any(|item| matches!(item, Surfaced::Status(status) if status == "Working..."))
            );
            assert_eq!(surfaced.last(), Some(&Surfaced::Status(String::new())));
        }

        fn tool_lifecycle() -> Vec<(&'static str, serde_json::Value)> {
            vec![
                ("turn.started", serde_json::json!({})),
                (
                    "tool.started",
                    serde_json::json!({ "tool_name": "read_internal_secrets_db" }),
                ),
                (
                    "tool.completed",
                    serde_json::json!({ "tool_name": "read_internal_secrets_db" }),
                ),
                ("turn.completed", serde_json::json!({})),
            ]
        }

        #[tokio::test]
        async fn a_running_turn_reports_its_phase_and_clears_at_the_end() {
            let surfaced = surfaced_for(
                SlackSurface::Pane,
                PublicToolVisibility::Generic,
                &tool_lifecycle(),
            )
            .await;

            assert_eq!(
                surfaced,
                vec![
                    Surfaced::Status(SLACK_THINKING_STATUS.to_string()),
                    Surfaced::Status(DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string()),
                    Surfaced::Status(SLACK_THINKING_STATUS.to_string()),
                    // The terminal event clears the line; a status left set says
                    // the agent is working long after it stopped.
                    Surfaced::Status(String::new()),
                ]
            );
        }

        #[tokio::test]
        async fn no_raw_tool_name_reaches_slack() {
            for visibility in [
                PublicToolVisibility::Generic,
                PublicToolVisibility::Narrated,
                PublicToolVisibility::None,
            ] {
                let surfaced =
                    surfaced_for(SlackSurface::Pane, visibility, &tool_lifecycle()).await;
                for call in &surfaced {
                    let Surfaced::Status(status) = call else {
                        continue;
                    };
                    assert!(
                        !status.contains("read_internal_secrets_db"),
                        "{visibility:?} leaked the tool name: {status}"
                    );
                }
            }
        }

        #[tokio::test]
        async fn none_visibility_shows_nothing_about_the_running_tool() {
            let surfaced = surfaced_for(
                SlackSurface::Pane,
                PublicToolVisibility::None,
                &tool_lifecycle(),
            )
            .await;

            // The turn-level status still says work is happening — that reveals
            // nothing about it. What `None` removes is any tool-derived text, so
            // the line never changes while the tool runs.
            assert_eq!(
                surfaced,
                vec![
                    Surfaced::Status(SLACK_THINKING_STATUS.to_string()),
                    Surfaced::Status(String::new()),
                ],
                "None visibility must not narrate the tool at all"
            );
        }

        #[tokio::test]
        async fn overlapping_tools_do_not_clear_the_status_early() {
            let surfaced = surfaced_for(
                SlackSurface::Pane,
                PublicToolVisibility::Generic,
                &[
                    ("turn.started", serde_json::json!({})),
                    ("tool.started", serde_json::json!({})),
                    ("tool.started", serde_json::json!({})),
                    ("tool.completed", serde_json::json!({})),
                ],
            )
            .await;

            let generic = DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string();
            assert_eq!(
                surfaced.last(),
                Some(&Surfaced::Status(generic)),
                "one of two running tools finishing must leave the status on the tool text"
            );
        }

        #[tokio::test]
        async fn a_channel_thread_gets_no_status_line() {
            let surfaced = surfaced_for(
                SlackSurface::Channel,
                PublicToolVisibility::Generic,
                &tool_lifecycle(),
            )
            .await;

            assert!(
                surfaced.is_empty(),
                "a channel thread has no status line to set, got {surfaced:?}"
            );
        }

        #[tokio::test]
        async fn the_agents_title_reaches_the_thread() {
            let surfaced = surfaced_for(
                SlackSurface::Pane,
                PublicToolVisibility::Generic,
                &[
                    ("turn.started", serde_json::json!({})),
                    (
                        "session.title.updated",
                        serde_json::json!({ "title": "Refund policy for EU orders" }),
                    ),
                ],
            )
            .await;

            assert!(
                surfaced.contains(&Surfaced::Title("Refund policy for EU orders".to_string())),
                "the agent's title should reach the pane, got {surfaced:?}"
            );
        }

        #[tokio::test]
        async fn an_unchanged_status_is_not_pushed_twice() {
            let surfaced = surfaced_for(
                SlackSurface::Pane,
                PublicToolVisibility::Generic,
                &[
                    ("turn.started", serde_json::json!({})),
                    ("turn.started", serde_json::json!({})),
                ],
            )
            .await;

            assert_eq!(
                surfaced,
                vec![Surfaced::Status(SLACK_THINKING_STATUS.to_string())],
                "the same status twice is a wasted call on a rate-limited API"
            );
        }
    }

    mod terminal_state_tests;

    // ==========================================
    // Markdown block rendering (EVE-971)
    // ==========================================

    mod markdown_block_tests {
        use super::*;

        fn blocks_of(payload: &serde_json::Value) -> Vec<String> {
            payload["blocks"]
                .as_array()
                .expect("blocks array")
                .iter()
                .map(|b| {
                    assert_eq!(
                        b["type"], "markdown",
                        "every block must be a markdown block"
                    );
                    b["text"].as_str().unwrap().to_string()
                })
                .collect()
        }

        /// The bug: agent Markdown went out in `text`, which Slack reads as the
        /// much smaller `mrkdwn` dialect. A heading, a table and a fenced code
        /// block must now reach Slack verbatim inside a `markdown` block.
        #[test]
        fn reply_is_posted_as_a_markdown_block_verbatim() {
            let reply =
                "# Heading\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```";

            let payloads = build_post_payloads("C123", "1700.1", reply, None);

            assert_eq!(payloads.len(), 1);
            let blocks = blocks_of(&payloads[0]);
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0], reply, "Markdown must not be rewritten");
            // The fallback stays populated for notifications.
            assert_eq!(payloads[0]["text"], reply);
            assert_eq!(payloads[0]["thread_ts"], "1700.1");
        }

        /// `thread_ts` is omitted rather than sent empty for a top-level post.
        #[test]
        fn top_level_post_omits_thread_ts() {
            let payloads = build_post_payloads("C123", "", "hi", None);
            assert!(payloads[0].get("thread_ts").is_none());
        }

        /// Past the block limit the reply is split across blocks, not cut off.
        #[test]
        fn long_reply_splits_across_blocks_without_losing_text() {
            let line = "x".repeat(200);
            let reply = std::iter::repeat_n(line.as_str(), 200)
                .collect::<Vec<_>>()
                .join("\n"); // ~40k characters

            let payloads = build_post_payloads("C123", "", &reply, None);

            assert_eq!(payloads.len(), 1, "40k fits in one message");
            let blocks = blocks_of(&payloads[0]);
            assert!(
                blocks.len() > 1,
                "must actually split, got {}",
                blocks.len()
            );
            for block in &blocks {
                assert!(
                    block.chars().count() <= SLACK_MARKDOWN_BLOCK_LIMIT,
                    "block of {} exceeds Slack's limit",
                    block.chars().count()
                );
            }
            // Nothing was dropped: rejoining reproduces the reply.
            let rejoined: String = blocks.join("");
            assert_eq!(rejoined, reply, "split must be lossless");
        }

        /// A fence interrupted by a split is closed and reopened, so both halves
        /// still render as code rather than the second half leaking as prose.
        #[test]
        fn split_inside_a_fence_closes_and_reopens_it() {
            let body = std::iter::repeat_n("let x = 1;", 2000)
                .collect::<Vec<_>>()
                .join("\n");
            let reply = format!("intro\n\n```rust\n{}\n```", body);

            let blocks = blocks_of(&build_post_payloads("C123", "", &reply, None)[0]);
            assert!(blocks.len() > 1, "fixture must be long enough to split");

            // Every block has balanced fences, so none renders half-open.
            for (i, block) in blocks.iter().enumerate() {
                let fences = block
                    .lines()
                    .filter(|l| l.trim_start().starts_with("```"))
                    .count();
                assert_eq!(
                    fences % 2,
                    0,
                    "block {i} leaves a fence open: {fences} markers"
                );
            }
            // The reopened fence keeps the language hint.
            assert!(
                blocks[1].starts_with("```rust"),
                "continuation must reopen the fence with its info string, got: {:?}",
                &blocks[1][..20.min(blocks[1].len())]
            );
        }

        /// The notification fallback is capped; Slack rejects an oversized `text`.
        #[test]
        fn notification_fallback_is_capped() {
            let reply = "y".repeat(SLACK_TEXT_FALLBACK_LIMIT * 3);
            let payloads = build_post_payloads("C123", "", &reply, None);
            let fallback = payloads[0]["text"].as_str().unwrap();
            assert_eq!(fallback.chars().count(), SLACK_TEXT_FALLBACK_LIMIT);
        }

        /// Past 50 blocks the reply spills into a second message rather than
        /// losing the tail to Slack's per-message block cap.
        #[test]
        fn reply_past_the_block_cap_spills_into_another_message() {
            // Comfortably more than 50 full blocks.
            let reply = "z".repeat(SLACK_MARKDOWN_BLOCK_LIMIT * 52);
            let payloads = build_post_payloads("C123", "1700.1", &reply, None);

            assert_eq!(payloads.len(), 2);
            assert_eq!(blocks_of(&payloads[0]).len(), SLACK_MAX_BLOCKS_PER_MESSAGE);
            assert!(!blocks_of(&payloads[1]).is_empty());
            // Every part still targets the same thread.
            for payload in &payloads {
                assert_eq!(payload["thread_ts"], "1700.1");
            }
        }

        /// Correlation gives a durable Slack-message-to-run key.
        #[test]
        fn correlation_is_stamped_as_message_metadata() {
            let correlation = SlackCorrelation {
                session_id: "session_01abc".to_string(),
                input_message_id: "msg_01xyz".to_string(),
            };
            let payloads = build_post_payloads("C123", "", "hi", Some(&correlation));

            let metadata = &payloads[0]["metadata"];
            assert_eq!(metadata["event_type"], "everruns_agent_reply");
            assert_eq!(metadata["event_payload"]["session_id"], "session_01abc");
            assert_eq!(metadata["event_payload"]["input_message_id"], "msg_01xyz");
        }

        /// Without correlation the key is absent, not empty or invented.
        #[test]
        fn absent_correlation_omits_metadata() {
            let payloads = build_post_payloads("C123", "", "hi", None);
            assert!(payloads[0].get("metadata").is_none());
        }

        /// Multi-byte text must not panic the fallback truncation.
        #[test]
        fn fallback_truncation_respects_char_boundaries() {
            let reply = "\u{1f680}".repeat(SLACK_TEXT_FALLBACK_LIMIT * 2);
            let payloads = build_post_payloads("C123", "", &reply, None);
            let fallback = payloads[0]["text"].as_str().unwrap();
            assert_eq!(fallback.chars().count(), SLACK_TEXT_FALLBACK_LIMIT);
        }

        /// A single unbroken line past the limit has no line boundary to split
        /// on; it must still be chunked rather than emitted oversized.
        #[test]
        fn a_single_oversized_line_is_hard_split() {
            let reply = "q".repeat(SLACK_MARKDOWN_BLOCK_LIMIT * 3);
            let blocks = blocks_of(&build_post_payloads("C123", "", &reply, None)[0]);
            assert!(blocks.len() >= 3);
            for block in &blocks {
                assert!(block.chars().count() <= SLACK_MARKDOWN_BLOCK_LIMIT);
            }
            assert_eq!(blocks.join(""), reply);
        }

        /// Regression for a zero-progress hard split: an opening fence whose
        /// continuation prefix filled the effective block size used to make
        /// the following oversized line loop and allocate forever.
        #[test]
        fn boundary_sized_fence_info_cannot_stall_hard_split() {
            let effective = SLACK_MARKDOWN_BLOCK_LIMIT - 4;
            let reply = format!(
                "```{}\n{}",
                "i".repeat(effective - 4),
                "X".repeat(effective + 1)
            );

            let blocks = split_markdown_for_blocks(&reply, SLACK_MARKDOWN_BLOCK_LIMIT);

            assert!(blocks.len() >= 2);
            assert!(
                blocks
                    .iter()
                    .all(|block| block.chars().count() <= SLACK_MARKDOWN_BLOCK_LIMIT)
            );
        }

        /// Exercise varied limits, fence-info lengths, Unicode, and oversized
        /// lines. Returning proves termination; every output block must retain
        /// the splitter's hard size invariant.
        #[test]
        fn split_markdown_always_makes_progress_and_respects_limits() {
            for limit in 9..80 {
                for info_len in [0, 1, limit / 2, limit, limit * 2] {
                    let reply = format!(
                        "```{}\n{}\n~~~{}\n{}",
                        "lang".repeat(info_len),
                        "🚀".repeat(limit * 3),
                        "x".repeat(info_len),
                        "tail".repeat(limit * 2),
                    );
                    let blocks = split_markdown_for_blocks(&reply, limit);
                    assert!(!blocks.is_empty());
                    assert!(
                        blocks.iter().all(|block| block.chars().count() <= limit),
                        "limit {limit}, info length {info_len}, blocks: {blocks:?}"
                    );
                }
            }
        }

        #[test]
        fn outbound_reply_source_has_a_total_cap() {
            let reply = "z".repeat(SLACK_MAX_OUTBOUND_REPLY_CHARS + 10_000);
            let payloads = build_post_payloads("C123", "", &reply, None);
            let delivered_chars: usize = payloads
                .iter()
                .flat_map(blocks_of)
                .map(|block| block.chars().count())
                .sum();

            assert_eq!(delivered_chars, SLACK_MAX_OUTBOUND_REPLY_CHARS);
        }

        /// End to end through the real post path: the wire body Slack receives
        /// carries the blocks, not just the payload builder's return value.
        #[tokio::test]
        async fn posted_request_body_carries_markdown_blocks_and_metadata() {
            use wiremock::{
                Mock, MockServer, ResponseTemplate,
                matchers::{method, path},
            };

            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"ok": true, "ts": "1.2"})),
                )
                .mount(&mock_server)
                .await;

            let correlation = SlackCorrelation {
                session_id: "session_01abc".to_string(),
                input_message_id: "msg_01xyz".to_string(),
            };
            post_slack_message(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "1700.1",
                "## Title\n\n```py\nx = 1\n```",
                Some(&correlation),
            )
            .await
            .expect("post should succeed");

            let requests = mock_server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();

            assert_eq!(body["blocks"][0]["type"], "markdown");
            assert_eq!(body["blocks"][0]["text"], "## Title\n\n```py\nx = 1\n```");
            assert_eq!(
                body["metadata"]["event_payload"]["session_id"],
                "session_01abc"
            );
            assert_eq!(body["thread_ts"], "1700.1");
        }

        /// The ack path has no correlation to stamp, but still renders as a block.
        #[tokio::test]
        async fn ack_path_posts_blocks_without_metadata() {
            use wiremock::{
                Mock, MockServer, ResponseTemplate,
                matchers::{method, path},
            };

            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})),
                )
                .mount(&mock_server)
                .await;

            post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "On it.")
                .await
                .expect("ack should succeed");

            let requests = mock_server.received_requests().await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(body["blocks"][0]["text"], "On it.");
            assert!(body.get("metadata").is_none());
        }

        /// Short replies stay a single block — no gratuitous splitting.
        #[test]
        fn short_reply_is_one_block() {
            let blocks = blocks_of(&build_post_payloads("C123", "", "ok", None)[0]);
            assert_eq!(blocks, vec!["ok".to_string()]);
        }
    }
}
