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
    ChannelDeliveryAdapter, DeliveryContext as ChannelDeliveryContext,
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
    #[allow(dead_code)]
    surface: SlackSurface,
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

        loop {
            tokio::select! {
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

                // Post output messages to Slack
                if let Some(text) =
                    extract_delivery_text(&event.event_type, ctx.reply_mode, &event.data)
                {
                    // In report-progress-only mode the only text that reaches here
                    // is a progress report; in all-messages mode it is the answer.
                    let is_progress_report = ctx.reply_mode == SlackReplyMode::ReportProgressOnly;
                    match self
                        .post(
                            &ctx,
                            session_id,
                            Some(key.input_message_id.clone()),
                            text,
                            is_progress_report,
                        )
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

            // Update cursor or unregister
            if let Some(event_type) = terminal_event {
                // A turn that ended without a delivered reply is silence in the Slack
                // thread. Post exactly one status line so the user knows the request
                // is over, and unregister either way.
                if !delivered {
                    let notice = self.terminal_notice(&event_type, session_id);
                    match self
                        .post(
                            &ctx,
                            session_id,
                            Some(key.input_message_id.clone()),
                            notice,
                            false,
                        )
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
            } else if new_since_id != ctx.since_event_id || delivered != ctx.delivered {
                // Update the cursor
                let mut deliveries = self.deliveries.write().await;
                if let Some(ctx) = deliveries.get_mut(&key) {
                    ctx.since_event_id = new_since_id;
                    ctx.delivered = delivered;
                }
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
        input_message_id: Option<String>,
        text: String,
        is_progress_report: bool,
    ) -> ChannelDeliveryResult {
        let message = OutboundChannelMessage {
            session_id: SessionId::from_uuid(session_id),
            text,
            thread_ref: ctx.thread_ts.clone(),
            is_progress_report,
            correlation_id: input_message_id,
        };
        let delivery_ctx = ChannelDeliveryContext {
            auth_token: ctx.bot_token.clone(),
            channel_id: ctx.channel.clone(),
            thread_ref: ctx.thread_ts.clone(),
            reply_mode: ctx.reply_mode.into(),
            extra: HashMap::new(),
        };
        self.adapter.deliver(&message, &delivery_ctx).await
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

    fn format_progress_report(
        &self,
        report: &everruns_core::progress_reporting::ProgressReportPayload,
    ) -> String {
        format_progress_report_for_slack(report)
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

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Characters Slack accepts in one `markdown` block.
const SLACK_MARKDOWN_BLOCK_LIMIT: usize = 12_000;

/// Blocks Slack accepts in one `chat.postMessage` call.
const SLACK_MAX_BLOCKS_PER_MESSAGE: usize = 50;

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

/// Truncate on a char boundary, so multi-byte text cannot panic the slice.
fn truncate_chars(text: &str, limit: usize) -> &str {
    match text.char_indices().nth(limit) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

/// Split Markdown into pieces that each fit one Slack `markdown` block.
///
/// Splits on line boundaries, and never leaves a fenced code block open: when a
/// boundary lands inside a fence the fence is closed at the end of the piece and
/// reopened — with its original info string — at the start of the next, so a
/// split code block still renders as code on both sides.
fn split_markdown_for_blocks(text: &str, limit: usize) -> Vec<String> {
    debug_assert!(limit > 8, "limit must leave room for fence markers");
    if text.chars().count() <= limit {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    // The fence currently open, as (marker, info string) — e.g. ("```", "rust").
    let mut open_fence: Option<(String, String)> = None;
    // Reopened at the top of the next chunk when a split interrupts a fence.
    let mut reopen: Option<String> = None;

    // Reserve room for the closing fence we may have to append.
    let effective = limit.saturating_sub(4);

    let flush = |current: &mut String,
                 current_len: &mut usize,
                 open_fence: &Option<(String, String)>,
                 chunks: &mut Vec<String>,
                 reopen: &mut Option<String>| {
        if current.is_empty() {
            return;
        }
        let mut chunk = std::mem::take(current);
        if let Some((marker, info)) = open_fence {
            // Close the fence here and reopen it in the next chunk.
            if !chunk.ends_with('\n') {
                chunk.push('\n');
            }
            chunk.push_str(marker);
            *reopen = Some(format!("{}{}", marker, info));
        } else {
            *reopen = None;
        }
        chunks.push(chunk);
        *current_len = 0;
    };

    for line in text.split_inclusive('\n') {
        let line_len = line.chars().count();

        // A single line past the limit has no safe boundary; hard-split it.
        if line_len > effective {
            flush(
                &mut current,
                &mut current_len,
                &open_fence,
                &mut chunks,
                &mut reopen,
            );
            if let Some(ref head) = reopen.take() {
                current.push_str(head);
                current.push('\n');
                current_len = head.chars().count() + 1;
            }
            let mut rest = line;
            while rest.chars().count() > effective.saturating_sub(current_len) {
                let room = effective.saturating_sub(current_len);
                let head = truncate_chars(rest, room);
                current.push_str(head);
                current_len += head.chars().count();
                rest = &rest[head.len()..];
                flush(
                    &mut current,
                    &mut current_len,
                    &open_fence,
                    &mut chunks,
                    &mut reopen,
                );
                if let Some(ref h) = reopen.take() {
                    current.push_str(h);
                    current.push('\n');
                    current_len = h.chars().count() + 1;
                }
            }
            current.push_str(rest);
            current_len += rest.chars().count();
            continue;
        }

        if current_len + line_len > effective {
            flush(
                &mut current,
                &mut current_len,
                &open_fence,
                &mut chunks,
                &mut reopen,
            );
            if let Some(head) = reopen.take() {
                current.push_str(&head);
                current.push('\n');
                current_len = head.chars().count() + 1;
            }
        }

        // Track fence state after placement, so the marker line itself lands in
        // the chunk that opens or closes it.
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed
            .strip_prefix("```")
            .or_else(|| trimmed.strip_prefix("~~~"))
        {
            let marker = &trimmed[..3];
            match open_fence {
                // A closing fence carries no info string.
                Some((ref open_marker, _)) if open_marker == marker => open_fence = None,
                Some(_) => {}
                None => open_fence = Some((marker.to_string(), rest.trim_end().to_string())),
            }
        }

        current.push_str(line);
        current_len += line_len;
    }

    flush(
        &mut current,
        &mut current_len,
        &open_fence,
        &mut chunks,
        &mut reopen,
    );

    chunks
}

/// Build the `chat.postMessage` payloads for one reply.
///
/// Normally one payload. A reply past
/// `SLACK_MARKDOWN_BLOCK_LIMIT * SLACK_MAX_BLOCKS_PER_MESSAGE` (600k characters)
/// spills into further messages rather than being truncated.
fn build_post_payloads(
    channel: &str,
    thread_ts: &str,
    text: &str,
    correlation: Option<&SlackCorrelation>,
) -> Vec<serde_json::Value> {
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
