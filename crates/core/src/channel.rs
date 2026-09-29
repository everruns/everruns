// Multi-platform channel abstractions
//
// Design Decision: Channel adapters are the boundary between platform-specific
// protocols (Slack, Discord, Teams, Telegram) and the platform-agnostic core.
// Each adapter translates inbound platform events into InboundChannelEvent and
// receives OutboundChannelMessage for delivery. The core never imports
// platform-specific types.
//
// Design Decision: ThreadContext carries participant tracking across all
// platforms. Multi-user threads (e.g. a Slack thread with 3 people, a Discord
// channel) share a single session with per-message ExternalActor attribution.
// ThreadContext is the "who's in this conversation" view; ExternalActor is the
// "who sent this message" view.
//
// Design Decision: Async agent invocations are first-class. The
// ChannelDeliveryAdapter trait models the webhook→ack→async-response pattern
// generically. Platform adapters implement `deliver()` to post results back
// when the agent finishes (minutes to hours later).
//
// Design Decision: Platform-contributed tools (e.g. slack_add_reaction,
// discord_create_thread) are a known gap. The Capability trait already supports
// tools(), but no channel adapter contributes tools yet. Tracked for future
// work — see TODO(platform-tools) below.

use crate::message::ExternalActor;
use crate::typed_id::SessionId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use std::collections::HashMap;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// ============================================
// Thread & Participant tracking
// ============================================

/// A participant in a multi-user thread.
///
/// Wraps ExternalActor with thread-level metadata (when they joined,
/// their role in the thread). Participants are accumulated over the
/// lifetime of a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Participant {
    /// The external actor identity (platform user ID, display name, source).
    pub actor: ExternalActor,
    /// When this participant first appeared in the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Platform-specific role (e.g. "owner", "member", "guest").
    /// Not all platforms expose this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Thread-level context for multi-user conversations.
///
/// A ThreadContext is created when a session is bound to a platform thread
/// and accumulates participants as messages arrive. Platform adapters update
/// this when new users join a thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadContext {
    /// Platform-specific thread identifier (e.g. Slack thread_ts, Discord channel_id).
    pub thread_ref: String,
    /// Source platform (e.g. "slack", "discord", "teams").
    pub platform: String,
    /// Platform-specific channel/workspace context.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub platform_metadata: HashMap<String, String>,
    /// Known participants in this thread, keyed by actor_id for O(1) lookup.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub participants: HashMap<String, Participant>,
    /// What the user is currently looking at on the platform, when it reports
    /// that (Slack: `app_context_changed`). Last write wins — it is a current
    /// position, not a history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_view: Option<ChannelViewContext>,
}

/// Session KV key holding the persisted [`ThreadContext`].
///
/// One key per session, not a prefix: a session belongs to exactly one channel
/// thread. `session_storage` reserves it from the user-facing `kv_store` tool
/// (see `is_internal_session_kv_key`) so a session or tool actor cannot forge
/// its own participant list or the "user is viewing" hint — both of which reach
/// the model as context (TM-TOOL/TM-AGENT).
pub const THREAD_CONTEXT_KV_KEY: &str = "channel:thread_context";

/// Where the user's attention is on the platform, as the platform reports it.
///
/// Deliberately opaque ids and nothing resolved. The agent has not been granted
/// access to whatever the user happens to be looking at, so this is a hint that
/// it should ask about, not a fact it can act on — see [`ThreadContext::view_summary`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelViewContext {
    /// Platform channel/conversation id the user is viewing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Platform team/workspace id, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// When the platform reported this position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl ChannelViewContext {
    /// True when there is nothing worth telling the model.
    pub fn is_empty(&self) -> bool {
        self.channel_id.is_none() && self.team_id.is_none()
    }
}

impl ThreadContext {
    /// Create a new thread context for a platform thread.
    pub fn new(thread_ref: impl Into<String>, platform: impl Into<String>) -> Self {
        Self {
            thread_ref: thread_ref.into(),
            platform: platform.into(),
            platform_metadata: HashMap::new(),
            participants: HashMap::new(),
            current_view: None,
        }
    }

    /// Record a participant. Updates first_seen_at only if new.
    /// Returns true if this is a newly seen participant.
    pub fn track_participant(&mut self, actor: &ExternalActor) -> bool {
        use std::collections::hash_map::Entry;
        match self.participants.entry(actor.actor_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(Participant {
                    actor: actor.clone(),
                    first_seen_at: Some(chrono::Utc::now()),
                    role: None,
                });
                true
            }
            Entry::Occupied(mut entry) => {
                // Update display name if it changed (user renamed)
                if actor.actor_name != entry.get().actor.actor_name {
                    entry.get_mut().actor.actor_name = actor.actor_name.clone();
                }
                false
            }
        }
    }

    /// Number of distinct participants.
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    /// Build a summary line for LLM context injection.
    /// e.g. "Thread participants: Alice, Bob, Charlie"
    pub fn participants_summary(&self) -> String {
        if self.participants.is_empty() {
            return String::new();
        }
        let mut names: Vec<String> = self
            .participants
            .values()
            .map(|p| p.actor.display_label().to_string())
            .collect();
        names.sort();
        format!("Thread participants: {}", names.join(", "))
    }

    /// Record where the user is now looking. Last write wins.
    ///
    /// Returns true when this actually changed the stored position, so callers
    /// can skip a write when the platform re-reports the same place. Observation
    /// time is metadata and does not make an otherwise identical position new.
    pub fn set_current_view(&mut self, view: ChannelViewContext) -> bool {
        let view = (!view.is_empty()).then_some(view);
        let same_position = match (&self.current_view, &view) {
            (Some(current), Some(next)) => {
                current.channel_id == next.channel_id && current.team_id == next.team_id
            }
            (None, None) => true,
            _ => false,
        };
        if same_position {
            return false;
        }
        self.current_view = view;
        true
    }

    /// One line describing where the user is looking, for model context.
    ///
    /// Phrased as a hint the agent must ask about rather than a fact it can act
    /// on. The platform reports what the *user* is viewing, which the agent may
    /// have no access to and no tool for; stating it as available context would
    /// invite the model to claim knowledge of a channel it cannot read. The id
    /// stays opaque for the same reason — resolving it to a name would mean
    /// fetching a channel the agent was never granted.
    pub fn view_summary(&self) -> String {
        let Some(view) = self.current_view.as_ref() else {
            return String::new();
        };
        let Some(channel_id) = view.channel_id.as_deref() else {
            return String::new();
        };
        format!(
            "The user is currently viewing {} channel {}. You have not been given \
             access to it — ask before assuming you can read it.",
            self.platform, channel_id
        )
    }
}

/// Decode a persisted thread context record.
///
/// A malformed record decodes to `None` rather than erroring: losing
/// accumulated participants degrades the prompt, but failing a turn over it
/// would take the whole conversation down for a context line.
///
/// The codec is shared by both writers (the channel webhook, through whatever
/// storage handle it has) and the reader (prompt assembly, through
/// `SessionStorageStore`), so the two cannot drift on shape.
pub fn decode_thread_context(raw: &str) -> Option<ThreadContext> {
    match serde_json::from_str(raw) {
        Ok(ctx) => Some(ctx),
        Err(error) => {
            tracing::warn!(%error, "Discarding malformed thread context record");
            None
        }
    }
}

/// Encode a thread context for persistence. See [`decode_thread_context`].
pub fn encode_thread_context(context: &ThreadContext) -> crate::error::Result<String> {
    serde_json::to_string(context).map_err(|e| crate::error::AgentLoopError::store(e.to_string()))
}

/// Load the persisted thread context for a session, if any.
///
/// An unreadable record is treated as absent, for the reason in
/// [`decode_thread_context`].
pub async fn load_thread_context(
    store: &dyn crate::session_services::SessionStorageStore,
    session_id: SessionId,
) -> Option<ThreadContext> {
    match store.get_value(session_id, THREAD_CONTEXT_KV_KEY).await {
        Ok(Some(raw)) => decode_thread_context(&raw),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%session_id, %error, "Failed to read persisted thread context");
            None
        }
    }
}

/// Persist the thread context for a session, replacing any previous record.
pub async fn save_thread_context(
    store: &dyn crate::session_services::SessionStorageStore,
    session_id: SessionId,
    context: &ThreadContext,
) -> crate::error::Result<()> {
    let encoded = encode_thread_context(context)?;
    store
        .set_value(session_id, THREAD_CONTEXT_KV_KEY, &encoded)
        .await
}

// ============================================
// Inbound channel events
// ============================================

/// A platform-agnostic inbound event from a channel.
///
/// Platform adapters parse their native webhook payloads into this type.
/// The server routes it to the correct session and creates the appropriate
/// input.message event.
#[derive(Debug, Clone)]
pub struct InboundChannelEvent {
    /// Who sent this message.
    pub actor: ExternalActor,
    /// Message text content (may be empty for attachment-only messages).
    pub text: String,
    /// Attached content (images, files) as platform-agnostic parts.
    pub attachments: Vec<InboundAttachment>,
    /// Platform-specific dedup key (e.g. Slack event_ts, Discord message_id).
    /// Used to prevent duplicate processing on webhook retries.
    pub dedup_key: String,
    /// Thread reference for routing to the correct session.
    /// None for DMs or platforms without threading.
    pub thread_ref: Option<String>,
    /// Platform-specific metadata for session tag construction.
    pub routing_metadata: HashMap<String, String>,
}

/// Attachment from an inbound platform message.
#[derive(Debug, Clone)]
pub enum InboundAttachment {
    /// Image with a fetchable URL.
    Image {
        url: String,
        alt_text: Option<String>,
    },
    /// Non-image file described as text.
    FileDescription {
        name: String,
        mime_type: Option<String>,
    },
}

// ============================================
// Outbound channel messages
// ============================================

/// A platform-agnostic outbound message to deliver to a channel.
///
/// The delivery adapter translates this into platform-specific API calls
/// (e.g. Slack chat.postMessage, Discord channel message create).
#[derive(Debug, Clone)]
pub struct OutboundChannelMessage {
    /// The session this message belongs to.
    pub session_id: SessionId,
    /// Text content to deliver.
    pub text: String,
    /// Thread reference for reply targeting.
    pub thread_ref: String,
    /// Whether this is a progress report (vs. a final answer).
    pub is_progress_report: bool,
    /// Id of the input message this reply answers, when the platform can stamp
    /// it onto the posted message for later correlation. `None` leaves the
    /// message unstamped rather than inventing a key.
    pub correlation_id: Option<String>,
}

// ============================================
// Channel delivery adapter (async agent responses)
// ============================================

/// Reply mode for channel delivery — controls which agent output reaches the channel.
///
/// Generalizes SlackReplyMode to work across all platforms.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChannelReplyMode {
    /// Forward all completed assistant messages to the channel.
    #[default]
    AllMessages,
    /// Only deliver explicit report_progress tool outputs.
    ReportProgressOnly,
}

/// Trait for platform-specific delivery of agent responses.
///
/// Implementations handle the "last mile" of posting messages back to
/// Slack, Discord, Teams, etc. The generic delivery dispatcher calls
/// these methods; platform adapters implement them.
///
/// Lifecycle:
/// 1. Webhook arrives → adapter parses InboundChannelEvent
/// 2. Server routes to session, creates input.message, triggers agent
/// 3. Core delivery dispatcher records a pending delivery for this session/turn
/// 4. Agent runs asynchronously (seconds to hours)
/// 5. When agent output is ready, the delivery dispatcher calls `deliver()`
/// 6. When the turn completes or is cancelled, the dispatcher clears the pending delivery
#[async_trait]
pub trait ChannelDeliveryAdapter: Send + Sync {
    /// Platform identifier (e.g. "slack", "discord").
    fn platform(&self) -> &str;

    /// Deliver a message to the platform channel.
    ///
    /// Called by the generic delivery dispatcher when agent output is ready.
    /// Implementations should handle retries internally for transient failures.
    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> DeliveryResult;

    /// Send an immediate acknowledgement to the channel.
    ///
    /// Called right after webhook ingestion for async agent invocations.
    /// e.g. Slack's "On it." message in report_progress_only mode.
    /// Platforms that don't need an ack can return Ok(()).
    async fn send_ack(
        &self,
        thread_ref: &str,
        text: &str,
        context: &DeliveryContext,
    ) -> DeliveryResult;

    /// Format a progress report for this platform.
    ///
    /// Different platforms have different formatting (Slack mrkdwn, Discord markdown, etc.)
    fn format_progress_report(
        &self,
        report: &crate::progress_reporting::ProgressReportPayload,
    ) -> String;

    /// Progressive delivery, when the platform supports it.
    ///
    /// A capability probe rather than three more required methods: `None` — the
    /// default — means the dispatcher uses discrete delivery, so a platform
    /// without streaming stays honest instead of stubbing an API it does not
    /// have (EVE-974).
    fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
        None
    }

    /// Live status and thread title, when the platform has an agent surface.
    ///
    /// Same capability-probe shape as `streaming`, for the same reason: `None`
    /// — the default — means the dispatcher skips status and title entirely,
    /// rather than every adapter stubbing methods for affordances its platform
    /// does not have (EVE-975).
    fn agent_surface(&self) -> Option<&dyn ChannelAgentSurface> {
        None
    }
}

/// The agent-pane affordances a platform may offer alongside the reply itself:
/// a live status line while a turn runs, and a thread title.
///
/// Both are advisory. A failure here must never fail the turn — the reply is the
/// product and the status is decoration — so the dispatcher logs and continues.
#[async_trait]
pub trait ChannelAgentSurface: Send + Sync {
    /// Set the live status line for a thread. An empty `status` clears it.
    async fn set_status(&self, status: &str, context: &DeliveryContext) -> DeliveryResult;

    /// Set the thread's title.
    async fn set_title(&self, title: &str, context: &DeliveryContext) -> DeliveryResult;
}

/// Progressive delivery of one message as it is produced.
///
/// A stream is per *output message*, not per turn: a turn that produces three
/// messages with tool calls between them is three streams, so the reader sees
/// three replies rather than one concatenated blob.
#[async_trait]
pub trait ChannelStreamDelivery: Send + Sync {
    /// Open a stream. The returned handle identifies it until `stop`.
    async fn start(&self, context: &DeliveryContext) -> Result<String, String>;

    /// Append newly produced text to an open stream.
    async fn append(&self, handle: &str, text: &str, context: &DeliveryContext) -> DeliveryResult;

    /// Close the stream.
    ///
    /// Must run for every `start`, including on failure and cancellation: an
    /// unstopped stream is a message left spinning in the client forever, which
    /// is worse than never having streamed at all.
    async fn stop(&self, handle: &str, context: &DeliveryContext) -> DeliveryResult;
}

/// Context needed by a delivery adapter to post messages.
///
/// Stored when a delivery is registered, consumed when events arrive.
/// Platform adapters extend this with platform-specific fields via `extra`.
#[derive(Clone)]
pub struct DeliveryContext {
    /// Bot/app authentication token for the platform API.
    pub auth_token: String,
    /// Platform-specific channel/conversation ID.
    pub channel_id: String,
    /// Thread reference for reply targeting.
    pub thread_ref: String,
    /// Reply mode controlling which output is delivered.
    pub reply_mode: ChannelReplyMode,
    /// Platform-specific extra context (e.g. team_id, workspace URL).
    pub extra: HashMap<String, String>,
}

impl std::fmt::Debug for DeliveryContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryContext")
            .field("auth_token", &"[REDACTED]")
            .field("channel_id", &self.channel_id)
            .field("thread_ref", &self.thread_ref)
            .field("reply_mode", &self.reply_mode)
            .field("extra", &self.extra)
            .finish()
    }
}

/// Result of a delivery attempt.
#[derive(Debug)]
pub enum DeliveryResult {
    /// Message delivered successfully.
    Ok,
    /// Transient failure — caller should retry with backoff.
    TransientError(String),
    /// Permanent failure — do not retry (e.g. invalid token, channel deleted).
    PermanentError(String),
}

// ============================================
// Session strategy (generalized from Slack)
// ============================================

/// Channel-agnostic session routing tag builder.
///
/// Given platform metadata from an InboundChannelEvent, produces the
/// session tags used to find or create the correct session.
///
/// The tag segment is deliberately NOT the binding's name: these tags key live
/// sessions, so `Thread` must keep emitting `thread`, `Conversation` `channel`,
/// and `Requester` `user`. Renaming a segment silently orphans every session
/// routed under the old one (EVE-1005).
///
/// `Endpoint` and `Ephemeral` return `None`: they are not keyed off inbound
/// message metadata at all. Their tags come from the exposure that owns the
/// invocation — see `trigger_session_tags` in the agent-triggers domain.
pub fn build_session_routing_tag(
    platform: &str,
    binding: &SessionBinding,
    metadata: &HashMap<String, String>,
) -> Option<String> {
    match binding {
        SessionBinding::Thread => metadata
            .get("thread_ref")
            .map(|t| format!("{}:thread:{}", platform, t)),
        SessionBinding::Conversation => metadata
            .get("channel_id")
            .map(|c| format!("{}:channel:{}", platform, c)),
        SessionBinding::Requester => metadata
            .get("user_id")
            .map(|u| format!("{}:user:{}", platform, u)),
        SessionBinding::Endpoint | SessionBinding::Ephemeral => None,
    }
}

/// Resolve the binding actually used for one inbound event.
///
/// The declared binding is a default the transport may override per event,
/// because the surface is a property of the event rather than of configuration.
/// Slack's assistant pane is the existing case: a pane is inherently one thread,
/// so `Conversation` and `Requester` have no meaning there — but rejecting them
/// at write time would be wrong, since the same exposure also serves channels
/// where they are legitimate (`knowledge/integrations/slack-modernization.md`).
///
/// Expressing that as one function keeps the pane from being a special case in
/// the Slack adapter, and gives the next transport somewhere to put the same
/// rule instead of re-deriving it (EVE-1005).
pub fn resolve_session_binding(
    declared: SessionBinding,
    event_override: Option<SessionBinding>,
) -> SessionBinding {
    event_override.unwrap_or(declared)
}

/// What identity keys a session, for every exposure and every transport.
///
/// One enum replaces the former `SessionStrategy` (messaging channels) and
/// `InvocationSessionMode` (triggers and request/reply endpoints), which asked
/// the same question with disjoint vocabularies and forced every new surface to
/// pick a side (EVE-1005).
///
/// **The serialized values are deliberately the legacy ones.** Every variant
/// renames in Rust but serializes exactly as it did before, with the new name
/// accepted as a read alias. Persisted `channel_config` JSONB therefore needs no
/// migration, and the API and UI keep exchanging the values they already do.
/// Moving the wire vocabulary is a separate, migration-bearing change.
///
/// `Requester` keys on the **transport's own external actor id** — the Slack
/// user id, the Public Chat visitor id — never on an Everruns principal. Those
/// actors are unrelated to Everruns accounts (a Public Chat visitor is anonymous
/// or Google-signed-in), so there is one consistent answer rather than a split
/// variant: whatever the transport calls the requester, scoped by the
/// `{platform}:` tag prefix that already namespaces it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "per_thread"))]
pub enum SessionBinding {
    /// One session per thread. Was `per_thread`.
    #[default]
    #[serde(rename = "per_thread", alias = "thread")]
    Thread,
    /// One session per channel/conversation/room. Was `per_channel`.
    #[serde(rename = "per_channel", alias = "conversation")]
    Conversation,
    /// One session per external actor. Was `per_user`.
    #[serde(rename = "per_user", alias = "requester")]
    Requester,
    /// One durable session shared by every invocation of the exposure.
    /// Was `shared_session`.
    #[serde(rename = "shared_session", alias = "endpoint")]
    Endpoint,
    /// A fresh session per invocation. Was `session_per_invocation`.
    #[serde(rename = "session_per_invocation", alias = "ephemeral")]
    Ephemeral,
}

impl SessionBinding {
    /// Bindings keyed off an inbound message's metadata.
    pub const MESSAGE_KEYED: [SessionBinding; 3] = [
        SessionBinding::Thread,
        SessionBinding::Conversation,
        SessionBinding::Requester,
    ];

    /// Bindings available where nothing is listening on a thread — triggers and
    /// request/reply endpoints.
    pub const INVOCATION_KEYED: [SessionBinding; 2] =
        [SessionBinding::Endpoint, SessionBinding::Ephemeral];

    /// Whether this binding is keyed off inbound message metadata.
    pub fn is_message_keyed(self) -> bool {
        Self::MESSAGE_KEYED.contains(&self)
    }
}

// TODO(platform-tools): Channel adapters should optionally contribute
// platform-specific tools via the Capability trait. Examples:
// - Slack: add_reaction, post_to_channel, create_thread, upload_file
// - Discord: create_thread, add_reaction, pin_message
// - Teams: send_adaptive_card, create_tab
// The plumbing exists (Capability::tools()), but no adapter uses it yet.
// When implementing, tools should receive platform context via ToolContext
// (which already has session access for looking up channel config).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thread_context_track_participant() {
        let mut ctx = ThreadContext::new("1234.5678", "slack");
        let mut actor = ExternalActor {
            actor_id: "U001".into(),
            actor_name: Some("Alice".into()),
            source: "slack".into(),
            metadata: Some(HashMap::from([("team".into(), "T1".into())])),
        };
        assert!(ctx.track_participant(&actor));
        let participant = &ctx.participants["U001"];
        assert_eq!(participant.actor, actor);
        assert!(participant.first_seen_at.is_some());
        assert!(!ctx.track_participant(&actor));
        assert_eq!(ctx.participant_count(), 1);

        // A fixed earlier instant detects timestamp replacement without sleeps.
        let first_seen = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let participant = ctx.participants.get_mut("U001").unwrap();
        participant.first_seen_at = Some(first_seen);
        participant.role = Some("owner".into());
        actor.actor_name = Some("Alice B.".into());
        assert!(!ctx.track_participant(&actor));
        assert_eq!(ctx.participant_count(), 1);
        assert_eq!(
            ctx.participants["U001"],
            Participant {
                actor,
                first_seen_at: Some(first_seen),
                role: Some("owner".into()),
            }
        );
    }

    #[test]
    fn test_thread_context_participants_summary() {
        let mut ctx = ThreadContext::new("thread_1", "discord");
        assert_eq!(ctx.participants_summary(), "");
        for (actor_id, name) in [
            ("U003", Some("Zoe")),
            ("U002", None),
            ("U001", Some("Alice")),
        ] {
            ctx.track_participant(&ExternalActor {
                actor_id: actor_id.into(),
                actor_name: name.map(str::to_string),
                source: "discord".into(),
                metadata: None,
            });
        }
        assert_eq!(
            ctx.participants_summary(),
            "Thread participants: Alice, U002, Zoe"
        );
    }

    #[test]
    fn test_build_session_routing_tags() {
        let metadata = HashMap::from([
            ("thread_ref".into(), "1234.5678".into()),
            ("channel_id".into(), "C0123".into()),
            ("user_id".into(), "U999".into()),
        ]);
        for (binding, platform, key, expected) in [
            (
                SessionBinding::Thread,
                "slack",
                "thread_ref",
                "slack:thread:1234.5678",
            ),
            (
                SessionBinding::Conversation,
                "discord",
                "channel_id",
                "discord:channel:C0123",
            ),
            (
                SessionBinding::Requester,
                "teams",
                "user_id",
                "teams:user:U999",
            ),
        ] {
            assert_eq!(
                build_session_routing_tag(platform, &binding, &metadata).as_deref(),
                Some(expected)
            );
            let mut missing = metadata.clone();
            missing.remove(key);
            assert_eq!(
                build_session_routing_tag(platform, &binding, &missing),
                None
            );
            assert_eq!(
                build_session_routing_tag(platform, &binding, &HashMap::new()),
                None
            );
        }
    }

    /// EVE-1005: the tag segment is the old strategy word, not the new binding
    /// name. A rename here silently orphans every live session keyed under it,
    /// so the exact strings are pinned rather than derived.
    #[test]
    fn session_binding_tags_keep_their_legacy_segments() {
        let metadata = HashMap::from([
            ("thread_ref".into(), "T1".into()),
            ("channel_id".into(), "C1".into()),
            ("user_id".into(), "U1".into()),
        ]);
        for (binding, expected) in [
            (SessionBinding::Thread, Some("slack:thread:T1")),
            (SessionBinding::Conversation, Some("slack:channel:C1")),
            (SessionBinding::Requester, Some("slack:user:U1")),
            // Not keyed off inbound metadata: the exposure that owns the
            // invocation supplies these tags.
            (SessionBinding::Endpoint, None),
            (SessionBinding::Ephemeral, None),
        ] {
            assert_eq!(
                build_session_routing_tag("slack", &binding, &metadata).as_deref(),
                expected,
                "{binding:?}"
            );
        }
    }

    /// EVE-1005: the declared binding is a default the event may override.
    #[test]
    fn resolve_session_binding_lets_the_event_override_the_declaration() {
        // No override: configuration wins, whatever it says.
        for declared in SessionBinding::MESSAGE_KEYED {
            assert_eq!(resolve_session_binding(declared, None), declared);
        }
        // The Slack pane case: a one-thread surface forces Thread even though
        // the exposure legitimately declares Conversation for its channels.
        assert_eq!(
            resolve_session_binding(SessionBinding::Conversation, Some(SessionBinding::Thread)),
            SessionBinding::Thread
        );
        assert_eq!(
            resolve_session_binding(SessionBinding::Requester, Some(SessionBinding::Thread)),
            SessionBinding::Thread
        );
    }

    #[test]
    fn test_channel_reply_mode_wire_contract() {
        assert_eq!(ChannelReplyMode::default(), ChannelReplyMode::AllMessages);
        for (mode, wire) in [
            (ChannelReplyMode::AllMessages, "\"all_messages\""),
            (
                ChannelReplyMode::ReportProgressOnly,
                "\"report_progress_only\"",
            ),
        ] {
            assert_eq!(serde_json::to_string(&mode).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<ChannelReplyMode>(wire).unwrap(),
                mode
            );
        }
    }

    /// EVE-1005: every value persisted in `channel_config` JSONB before the
    /// enums were unified must still deserialize, and must still serialize back
    /// to the same string. This is what makes the change migration-free; if it
    /// fails, stored channel configs are unreadable.
    #[test]
    fn test_session_binding_wire_contract() {
        assert_eq!(SessionBinding::default(), SessionBinding::Thread);
        for (binding, wire) in [
            // Legacy `SessionBinding` values.
            (SessionBinding::Thread, "\"per_thread\""),
            (SessionBinding::Conversation, "\"per_channel\""),
            (SessionBinding::Requester, "\"per_user\""),
            // Legacy `SessionBinding` values.
            (SessionBinding::Endpoint, "\"shared_session\""),
            (SessionBinding::Ephemeral, "\"session_per_invocation\""),
        ] {
            assert_eq!(
                serde_json::to_string(&binding).unwrap(),
                wire,
                "{binding:?} must still serialize to its legacy value"
            );
            assert_eq!(
                serde_json::from_str::<SessionBinding>(wire).unwrap(),
                binding,
                "{wire} must still deserialize"
            );
        }
    }

    /// The new vocabulary is accepted on read, so a config written with the
    /// binding names is understood even though nothing emits them yet.
    #[test]
    fn test_session_binding_accepts_new_names_as_aliases() {
        for (alias, binding) in [
            ("\"thread\"", SessionBinding::Thread),
            ("\"conversation\"", SessionBinding::Conversation),
            ("\"requester\"", SessionBinding::Requester),
            ("\"endpoint\"", SessionBinding::Endpoint),
            ("\"ephemeral\"", SessionBinding::Ephemeral),
        ] {
            assert_eq!(
                serde_json::from_str::<SessionBinding>(alias).unwrap(),
                binding
            );
        }
    }
    // ============================================
    // Persisted thread context (EVE-977)
    // ============================================

    fn actor(id: &str, name: &str) -> ExternalActor {
        ExternalActor {
            actor_id: id.to_string(),
            actor_name: Some(name.to_string()),
            source: "slack".to_string(),
            metadata: None,
        }
    }

    /// The bug: a ThreadContext built per message only ever saw one speaker, so
    /// the summary never named the thread. Accumulation is the whole point.
    #[test]
    fn participants_accumulate_across_a_round_trip() {
        let mut ctx = ThreadContext::new("1700.1", "slack");
        assert!(ctx.track_participant(&actor("U1", "Alice")));

        // Survive a restart: encode, drop, decode.
        let encoded = encode_thread_context(&ctx).expect("encode");
        let mut restored = decode_thread_context(&encoded).expect("decode");

        assert!(restored.track_participant(&actor("U2", "Bob")));
        assert!(
            !restored.track_participant(&actor("U1", "Alice")),
            "re-seen actor is not new"
        );

        assert_eq!(restored.participant_count(), 2);
        assert_eq!(
            restored.participants_summary(),
            "Thread participants: Alice, Bob"
        );
    }

    /// A malformed record degrades to "no context", never an error: losing the
    /// participant line must not take the conversation down with it.
    #[test]
    fn malformed_record_decodes_to_none() {
        assert!(decode_thread_context("not json").is_none());
        assert!(decode_thread_context("").is_none());
    }

    /// Re-reporting the same position is not a change, so it does not cause a write.
    #[test]
    fn setting_the_same_view_twice_reports_no_change() {
        let mut ctx = ThreadContext::new("1700.1", "slack");
        let view = ChannelViewContext {
            channel_id: Some("C123".to_string()),
            team_id: Some("T1".to_string()),
            observed_at: Some(chrono::Utc::now()),
        };

        assert!(
            ctx.set_current_view(view.clone()),
            "first report is a change"
        );
        let repeated = ChannelViewContext {
            observed_at: Some(chrono::Utc::now() + chrono::Duration::seconds(1)),
            ..view
        };
        assert!(
            !ctx.set_current_view(repeated),
            "a new observation time does not make the same location a change"
        );

        let moved = ChannelViewContext {
            channel_id: Some("C999".to_string()),
            team_id: Some("T1".to_string()),
            observed_at: None,
        };
        assert!(ctx.set_current_view(moved), "a real move is a change");
    }

    /// An empty report clears rather than storing a hollow record.
    #[test]
    fn empty_view_clears_the_current_position() {
        let mut ctx = ThreadContext::new("1700.1", "slack");
        ctx.set_current_view(ChannelViewContext {
            channel_id: Some("C123".to_string()),
            ..Default::default()
        });
        assert!(ctx.current_view.is_some());

        assert!(ctx.set_current_view(ChannelViewContext::default()));
        assert!(ctx.current_view.is_none());
        assert_eq!(ctx.view_summary(), "");
    }

    /// The view line must read as a hint to ask about, not as granted access —
    /// the agent has no tool for a channel the user merely happens to be in.
    #[test]
    fn view_summary_does_not_imply_access() {
        let mut ctx = ThreadContext::new("1700.1", "slack");
        ctx.set_current_view(ChannelViewContext {
            channel_id: Some("C123".to_string()),
            team_id: None,
            observed_at: None,
        });

        let summary = ctx.view_summary();
        assert!(summary.contains("C123"), "{summary}");
        assert!(summary.contains("slack"), "{summary}");
        assert!(
            summary.contains("have not been given access"),
            "must not present the channel as readable: {summary}"
        );
        assert!(summary.contains("ask before"), "{summary}");
    }

    /// No position reported means no line at all — not an empty or hedging one.
    #[test]
    fn no_view_yields_no_line() {
        let ctx = ThreadContext::new("1700.1", "slack");
        assert_eq!(ctx.view_summary(), "");
        assert_eq!(ctx.participants_summary(), "");
    }

    /// Round-tripping keeps the reported position, not just the participants.
    #[test]
    fn current_view_survives_encoding() {
        let mut ctx = ThreadContext::new("1700.1", "slack");
        ctx.set_current_view(ChannelViewContext {
            channel_id: Some("C123".to_string()),
            team_id: Some("T1".to_string()),
            observed_at: None,
        });

        let restored = decode_thread_context(&encode_thread_context(&ctx).unwrap()).unwrap();
        assert_eq!(restored.current_view, ctx.current_view);
    }
}
