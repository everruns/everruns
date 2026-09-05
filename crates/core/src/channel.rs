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
}

impl ThreadContext {
    /// Create a new thread context for a platform thread.
    pub fn new(thread_ref: impl Into<String>, platform: impl Into<String>) -> Self {
        Self {
            thread_ref: thread_ref.into(),
            platform: platform.into(),
            platform_metadata: HashMap::new(),
            participants: HashMap::new(),
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
pub fn build_session_routing_tag(
    platform: &str,
    strategy: &SessionRoutingStrategy,
    metadata: &HashMap<String, String>,
) -> Option<String> {
    match strategy {
        SessionRoutingStrategy::PerThread => metadata
            .get("thread_ref")
            .map(|t| format!("{}:thread:{}", platform, t)),
        SessionRoutingStrategy::PerChannel => metadata
            .get("channel_id")
            .map(|c| format!("{}:channel:{}", platform, c)),
        SessionRoutingStrategy::PerUser => metadata
            .get("user_id")
            .map(|u| format!("{}:user:{}", platform, u)),
    }
}

/// How incoming messages map to sessions — generalized from Slack's SessionStrategy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionRoutingStrategy {
    /// Each thread gets its own session (default).
    #[default]
    PerThread,
    /// One session per channel/conversation.
    PerChannel,
    /// One session per user across all threads.
    PerUser,
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
        for (strategy, platform, key, expected) in [
            (
                SessionRoutingStrategy::PerThread,
                "slack",
                "thread_ref",
                "slack:thread:1234.5678",
            ),
            (
                SessionRoutingStrategy::PerChannel,
                "discord",
                "channel_id",
                "discord:channel:C0123",
            ),
            (
                SessionRoutingStrategy::PerUser,
                "teams",
                "user_id",
                "teams:user:U999",
            ),
        ] {
            assert_eq!(
                build_session_routing_tag(platform, &strategy, &metadata).as_deref(),
                Some(expected)
            );
            let mut missing = metadata.clone();
            missing.remove(key);
            assert_eq!(
                build_session_routing_tag(platform, &strategy, &missing),
                None
            );
            assert_eq!(
                build_session_routing_tag(platform, &strategy, &HashMap::new()),
                None
            );
        }
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

    #[test]
    fn test_session_routing_strategy_wire_contract() {
        assert_eq!(
            SessionRoutingStrategy::default(),
            SessionRoutingStrategy::PerThread
        );
        for (strategy, wire) in [
            (SessionRoutingStrategy::PerThread, "\"per_thread\""),
            (SessionRoutingStrategy::PerChannel, "\"per_channel\""),
            (SessionRoutingStrategy::PerUser, "\"per_user\""),
        ] {
            assert_eq!(serde_json::to_string(&strategy).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<SessionRoutingStrategy>(wire).unwrap(),
                strategy
            );
        }
    }
}
