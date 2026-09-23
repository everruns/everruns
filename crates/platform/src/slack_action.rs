//! The seam a Slack-native capability acts through (EVE-1024).
//!
//! # Why the action crosses a seam instead of the token
//!
//! A Slack channel's `bot_token` lives in `SlackChannelConfig`, encrypted at
//! rest and resolved by the control plane. The obvious shape — hand the worker
//! the token and let the capability speak HTTP — would put a long-lived
//! workspace credential in the process that also runs model-chosen tool
//! arguments, and would need a second Slack HTTP path beside the one
//! `slack_delivery` already maintains.
//!
//! So the *action* travels and the credential does not. The capability names
//! what it wants done; the implementor (the control plane, which already holds
//! the channel row and the retry/`Retry-After` handling) resolves the
//! session's Slack channel, performs the call, and returns an outcome. The
//! worker never sees `bot_token`.
//!
//! # Fail closed
//!
//! Resolution keys off the `slack:endpoint:{id}` session tag that
//! `build_session_tags` stamps on Slack-originated sessions. A session without
//! that tag — one started from the API, a schedule, or another channel — has no
//! Slack channel to act as, and every action must fail with
//! [`SlackActionError::NoSlackSession`] rather than falling back to some other
//! channel's credential.
//!
//! Resolution deliberately goes through the **channel**, not the app: since
//! EVE-1008 the channel owns Slack bot identity, so one agent can carry two
//! Slack channels with different bots and resolving via `slack:app:{id}` would
//! pick the wrong one.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// One thing an agent can ask its own Slack channel to do.
///
/// Kept deliberately small. Each variant is a single Slack Web API call whose
/// blast radius is the conversation the agent is already in; anything wider
/// (posting to an arbitrary channel, opening a DM) is not in this enum, so a
/// model cannot reach it by choosing arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SlackAction {
    /// `reactions.add` — acknowledge a message with an emoji.
    AddReaction {
        /// Channel the message lives in.
        channel: String,
        /// The message's `ts`.
        timestamp: String,
        /// Emoji name without colons (e.g. `eyes`).
        name: String,
    },
    /// `chat.update` — rewrite a message the bot posted.
    UpdateMessage {
        channel: String,
        /// The `ts` of the message to rewrite.
        timestamp: String,
        /// Replacement text, rendered as Markdown blocks.
        text: String,
    },
    /// `users.info` — resolve a Slack user ID to a profile.
    LookupUser { user_id: String },
    /// `files.getUploadURLExternal` + `files.completeUploadExternal` — share a
    /// file into the conversation. The legacy `files.upload` is retired.
    UploadFile {
        channel: String,
        /// Thread to share into, when the conversation is threaded.
        thread_ts: Option<String>,
        filename: String,
        /// The file's bytes.
        content: Vec<u8>,
        /// Optional message posted alongside the file.
        initial_comment: Option<String>,
    },
}

impl SlackAction {
    /// Stable short name for logs and error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AddReaction { .. } => "add_reaction",
            Self::UpdateMessage { .. } => "update_message",
            Self::LookupUser { .. } => "lookup_user",
            Self::UploadFile { .. } => "upload_file",
        }
    }
}

/// What an action produced, shaped for a tool result.
///
/// Slack's own response bodies are not forwarded wholesale: they carry fields
/// the model has no use for and, in the case of `users.info`, more personal
/// data than an agent needs to address someone. Each variant carries the
/// narrow answer instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SlackActionOutcome {
    /// The reaction is on the message (including when it already was).
    ReactionAdded { already_reacted: bool },
    /// The message was rewritten; `timestamp` echoes the message acted on.
    MessageUpdated { channel: String, timestamp: String },
    /// A resolved user. Fields are `None` when the workspace withholds them.
    User {
        user_id: String,
        display_name: Option<String>,
        real_name: Option<String>,
        is_bot: bool,
        /// Workspace-local timezone identifier, when published.
        tz: Option<String>,
    },
    /// The file is shared into the conversation.
    FileUploaded {
        file_id: String,
        permalink: Option<String>,
    },
}

/// Why an action did not happen.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SlackActionError {
    /// This session did not come from Slack, so there is no channel to act as.
    ///
    /// The one case that is a capability configuration story rather than a
    /// fault: the agent has the capability enabled and is running somewhere the
    /// capability cannot apply.
    #[error(
        "this session did not originate from Slack, so there is no Slack channel to act as; \
         Slack tools only work in a session created by a Slack message"
    )]
    NoSlackSession,

    /// The session's channel is gone, disabled, or belongs to another org.
    #[error("the Slack channel for this session is no longer available")]
    ChannelUnavailable,

    /// The channel exists but carries no bot token yet.
    #[error("the Slack channel for this session has no bot token configured")]
    NotConfigured,

    /// Slack refused in a way retrying cannot fix (bad scope, unknown channel).
    #[error("Slack rejected the request: {0}")]
    Rejected(String),

    /// Slack asked us to slow down.
    #[error("Slack rate limited the request{}", match .retry_after_secs {
        Some(secs) => format!(" (retry after {secs}s)"),
        None => String::new(),
    })]
    RateLimited { retry_after_secs: Option<u64> },

    /// The argument the model chose cannot be used.
    #[error("{0}")]
    InvalidArgument(String),

    /// Network trouble, a 5xx, or a store failure.
    #[error("Slack request failed: {0}")]
    Transient(String),
}

impl SlackActionError {
    /// Whether the model should see this as a tool error it can reason about,
    /// rather than an internal fault.
    ///
    /// Everything except [`Self::Transient`] is the model's to act on: it can
    /// stop offering Slack actions, correct an argument, or back off. A
    /// transient failure is infrastructure and is reported as such.
    pub fn is_tool_error(&self) -> bool {
        !matches!(self, Self::Transient(_))
    }
}

/// Performs [`SlackAction`]s as one session's own Slack channel bot.
///
/// Implemented by the control plane. An instance is **bound to one org and one
/// session** at construction, the same way `platform_store(org_id, session_id)`
/// is: the tenant and the session are the implementor's to fix, not an argument
/// a caller supplies. A capability holds only the handle it was given, so there
/// is no argument it could vary to reach another session's channel.
#[async_trait]
pub trait SlackActionInvoker: Send + Sync {
    /// Resolve this session's Slack channel and perform `action` as its bot.
    ///
    /// Returns [`SlackActionError::NoSlackSession`] when the bound session did
    /// not come through a Slack channel. Implementations must fail closed
    /// there rather than falling back to any other channel's credential.
    async fn invoke(&self, action: SlackAction) -> Result<SlackActionOutcome, SlackActionError>;
}

/// `ToolContextExtensions` handle for [`SlackActionInvoker`].
///
/// Core carries the generic extension bag but does not name this service: a
/// Slack-native capability is a hosted product capability, so the capability
/// and its backend contract stay together in platform (EVE-897).
#[derive(Clone)]
pub struct SlackActionInvokerExt(pub std::sync::Arc<dyn SlackActionInvoker>);
