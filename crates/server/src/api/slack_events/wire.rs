//! Serde shapes for Slack event, file, attachment and reply payloads.

use serde::{Deserialize, Serialize};

/// Slack event wrapper (Events API envelope).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct SlackEventEnvelope {
    /// Event type: "url_verification", "event_callback", etc.
    #[serde(rename = "type")]
    pub(crate) event_type: String,
    /// Challenge string for URL verification.
    #[serde(default)]
    pub(crate) challenge: Option<String>,
    /// The actual event payload.
    #[serde(default)]
    pub(crate) event: Option<SlackEvent>,
    /// Team ID.
    #[serde(default)]
    pub(crate) team_id: Option<String>,
}

/// Inner Slack event (message, app_mention, etc.).
/// The agent pane's own thread, plus the user's current position.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SlackAssistantThread {
    #[serde(default)]
    pub(crate) channel_id: Option<String>,
    #[serde(default)]
    pub(crate) thread_ts: Option<String>,
    #[serde(default)]
    pub(crate) context: Option<SlackViewContext>,
    /// Present on a rename. Slack has carried it both here and at the event
    /// root across the `assistant.threads.*` → `agents.sessions.*` rename, so
    /// both are read (EVE-975).
    #[serde(default)]
    pub(crate) title: Option<String>,
}

/// What Slack says the user is looking at. Ids only — see
/// `ThreadContext::view_summary` for why nothing is resolved to a name.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SlackViewContext {
    #[serde(default)]
    pub(crate) channel_id: Option<String>,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct SlackEvent {
    /// Event type: "message", "app_mention", etc.
    #[serde(rename = "type")]
    pub(crate) event_type: String,
    /// User who sent the message.
    #[serde(default)]
    pub(crate) user: Option<String>,
    /// Message text.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// New thread title on a rename. Slack has carried it at the event root as
    /// well as under `assistant_thread`; both are read (EVE-975).
    #[serde(default)]
    pub(crate) title: Option<String>,
    /// Channel where the event occurred.
    #[serde(default)]
    pub(crate) channel: Option<String>,
    /// Thread timestamp (for threaded messages).
    #[serde(default)]
    pub(crate) thread_ts: Option<String>,
    /// Message timestamp.
    #[serde(default)]
    pub(crate) ts: Option<String>,
    /// Bot ID (present when message is from a bot — used to ignore own messages).
    #[serde(default)]
    pub(crate) bot_id: Option<String>,
    /// Subtype (e.g., "bot_message", "message_changed").
    #[serde(default)]
    pub(crate) subtype: Option<String>,
    /// Conversation kind: "channel", "group", "im", "mpim". `im` is the agent
    /// pane once the agent surface is enabled (EVE-973).
    #[serde(default)]
    pub(crate) channel_type: Option<String>,
    /// Agent-pane payload. `app_context_changed` reports the pane thread here
    /// rather than at the event root, plus what the user is now viewing.
    #[serde(default)]
    pub(crate) assistant_thread: Option<SlackAssistantThread>,
    /// File attachments (images, documents, videos, etc.).
    #[serde(default)]
    pub(crate) files: Vec<SlackFile>,
    /// Legacy attachments (link unfurls, bot attachments, etc.).
    #[serde(default)]
    pub(crate) attachments: Vec<SlackAttachment>,
}

/// Slack file attachment object (subset of fields we care about).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct SlackFile {
    /// File ID (e.g., "F0123456789").
    #[serde(default)]
    pub(crate) id: Option<String>,
    /// Original filename.
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// MIME type (e.g., "image/png", "video/mp4").
    #[serde(default)]
    pub(crate) mimetype: Option<String>,
    /// File type label from Slack (e.g., "png", "mp4", "pdf").
    #[serde(default)]
    pub(crate) filetype: Option<String>,
    /// URL for private download (requires bot token auth).
    #[serde(default)]
    pub(crate) url_private: Option<String>,
    /// File size in bytes.
    #[serde(default)]
    pub(crate) size: Option<u64>,
}

/// Slack legacy attachment (link unfurls, rich attachments from apps/workflows).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct SlackAttachment {
    /// Attachment title.
    #[serde(default)]
    pub(crate) title: Option<String>,
    /// Title link URL.
    #[serde(default)]
    pub(crate) title_link: Option<String>,
    /// Main text body.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// Fallback text (plain-text summary).
    #[serde(default)]
    pub(crate) fallback: Option<String>,
    /// Pretext (displayed above the attachment block).
    #[serde(default)]
    pub(crate) pretext: Option<String>,
    /// Image URL (full-size image in the attachment).
    #[serde(default)]
    pub(crate) image_url: Option<String>,
    /// Thumbnail URL.
    #[serde(default)]
    pub(crate) thumb_url: Option<String>,
    /// Author name.
    #[serde(default)]
    pub(crate) author_name: Option<String>,
    /// Service name (e.g., "GitHub", "Jira").
    #[serde(default)]
    pub(crate) service_name: Option<String>,
    /// Original URL that triggered the unfurl.
    #[serde(default)]
    pub(crate) original_url: Option<String>,
    /// Content subtype hint from Slack (e.g., "canvas", "huddle_transcript").
    #[serde(default)]
    pub(crate) app_unfurl_url: Option<String>,
}

/// Response for URL verification challenge.
#[derive(Serialize)]
pub(crate) struct ChallengeResponse {
    pub(crate) challenge: String,
}

/// Acknowledgement response for event callbacks.
#[derive(Serialize)]
pub(crate) struct AckResponse {
    pub(crate) ok: bool,
}

/// Reply message from Slack's conversations.replies API.
#[derive(Debug, Deserialize)]
pub(crate) struct SlackReplyMessage {
    /// User who sent the message (absent for bot messages).
    #[serde(default)]
    pub(crate) user: Option<String>,
    /// Message text.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// Message timestamp.
    #[serde(default)]
    pub(crate) ts: Option<String>,
    /// Bot ID (present when message is from a bot).
    #[serde(default)]
    pub(crate) bot_id: Option<String>,
    /// Subtype (e.g., "bot_message", "thread_broadcast").
    #[serde(default)]
    pub(crate) subtype: Option<String>,
}

/// Response for the manifest endpoint.
#[derive(Serialize)]
pub(crate) struct ManifestResponse {
    /// YAML manifest string for creating a Slack app
    pub(crate) manifest_yaml: String,
    /// Pre-filled URL to create a Slack app from the manifest
    pub(crate) create_url: String,
}
