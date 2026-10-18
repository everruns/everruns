//! Slack conversations a schedule trigger runs in (design: schedules in Slack
//! sessions, D2/D3).
//!
//! Decisions:
//! - A target names the conversation the way the channel binds it
//!   (`slack:user:{U}` or `slack:channel:{C}`), so a run lands in the session a
//!   message from that conversation would, and the next message lands in the
//!   run's.
//! - A run posts top-level: in a direct message that is the DM itself, in a
//!   channel a new message (open question 1 of the design). Never into a
//!   thread the schedule did not start.
//! - The input carries the channel's metadata naming the conversation, with
//!   `slack_reply_inline`, so `send_message` reaches it too. The conversation
//!   comes from the trigger's validated target, never from the message text
//!   (THREAT[TM-SLACK-005]).
//! - A DM's conversation id is read from the session's latest signed Slack
//!   input. Before anyone has written, the person's user id stands in:
//!   `chat.postMessage` to a user id posts in the app's DM with them, so the
//!   first run opens the DM without the `im:write` scope.

use std::collections::HashMap;

use everruns_contracts::runtime::channel::{DeliveryTarget, SessionBinding};
use serde_json::Value;

use super::delivery::{SLACK_RECIPIENT_TEAM_ID, SLACK_RECIPIENT_USER_ID};
use super::host::{SLACK_REPLY_INLINE, SLACK_THREAD_KEY, TARGET_SURFACE};
use crate::domains::agent_channels::ingress::{IngressChannel, IngressContext};
use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use crate::storage::StorageBackend;

/// A Slack conversation a schedule can run in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlackConversation {
    /// A direct message with this user, on a `per_user` channel.
    User(String),
    /// Top-level posts in this conversation, on a `per_channel` channel.
    Channel(String),
}

impl SlackConversation {
    /// The conversation a binding key names, or `None` when it names none.
    pub(crate) fn parse(binding_key: &str) -> Option<Self> {
        let (kind, id) = binding_key.strip_prefix("slack:")?.split_once(':')?;
        let valid = |prefixes: &[char]| {
            id.len() >= 2
                && id.len() <= 32
                && id.starts_with(prefixes)
                && id
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        };
        match kind {
            "user" if valid(&['U', 'W']) => Some(Self::User(id.to_string())),
            "channel" if valid(&['C', 'G', 'D']) => Some(Self::Channel(id.to_string())),
            _ => None,
        }
    }

    /// The channel binding this conversation is a key of.
    pub(crate) fn binding(&self) -> SessionBinding {
        match self {
            Self::User(_) => SessionBinding::Requester,
            Self::Channel(_) => SessionBinding::Conversation,
        }
    }

    /// The binding key, as the channel host stores it.
    pub(crate) fn binding_key(&self) -> String {
        match self {
            Self::User(user) => format!("slack:user:{user}"),
            Self::Channel(channel) => format!("slack:channel:{channel}"),
        }
    }
}

/// Why `conversation` is not one `config` can run a schedule in, or `None`.
pub(crate) fn target_problem(
    config: &SlackChannelConfig,
    conversation: &SlackConversation,
) -> Option<String> {
    if config.bot_token.trim().is_empty() {
        return Some("The Slack channel is not connected to a workspace".into());
    }
    if config.session_strategy != conversation.binding() {
        let expected = match config.session_strategy {
            SessionBinding::Requester => "slack:user:{user_id}",
            SessionBinding::Conversation => "slack:channel:{channel_id}",
            _ => {
                return Some(
                    "A schedule can run only in a Slack channel with per_user or per_channel \
                     sessions"
                        .into(),
                );
            }
        };
        return Some(format!(
            "This Slack channel keys sessions as {expected}; the target must use that form"
        ));
    }
    // A channel scoped to one Slack conversation answers nowhere else, so a
    // schedule may not post elsewhere either.
    if let Some(scoped) = config.channel_id.as_deref() {
        match conversation {
            SlackConversation::Channel(channel) if channel == scoped => {}
            _ => {
                return Some(format!(
                    "This Slack channel answers only in {scoped}; the target must be \
                     slack:channel:{scoped}"
                ));
            }
        }
    }
    None
}

/// Where a run in `conversation` posts and what its input carries.
pub(crate) struct ScheduledDelivery {
    pub target: DeliveryTarget,
    pub metadata: HashMap<String, Value>,
}

/// The delivery of a scheduled run in `conversation`, whose bound session (if
/// any) is `session`.
pub(crate) async fn scheduled_delivery(
    db: &StorageBackend,
    context: &IngressContext,
    channel: &IngressChannel,
    config: &SlackChannelConfig,
    conversation: &SlackConversation,
    session: Option<everruns_contracts::typed_id::SessionId>,
) -> ScheduledDelivery {
    let slack_channel = match conversation {
        SlackConversation::Channel(channel) => channel.clone(),
        SlackConversation::User(user) => match session {
            Some(session) => latest_dm(db, context, channel, session)
                .await
                .unwrap_or_else(|| user.clone()),
            None => user.clone(),
        },
    };
    let mut target = DeliveryTarget::new(slack_channel.clone(), String::new());
    target.extra.insert(TARGET_SURFACE.into(), "channel".into());
    if let SlackConversation::User(user) = conversation {
        target
            .extra
            .insert(SLACK_RECIPIENT_USER_ID.into(), user.clone());
    }
    if let Some(team) = &config.team_id {
        target
            .extra
            .insert(SLACK_RECIPIENT_TEAM_ID.into(), team.clone());
    }
    let metadata = [
        ("_app_id", Value::String(context.public_id.to_string())),
        (
            "_app_channel_id",
            Value::String(channel.public_id.to_string()),
        ),
        ("slack_channel", Value::String(slack_channel)),
        (SLACK_THREAD_KEY, Value::String(String::new())),
        (SLACK_REPLY_INLINE, Value::Bool(true)),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect();
    ScheduledDelivery { target, metadata }
}

/// The DM conversation id of the session's latest Slack input from this
/// channel, if it has one.
async fn latest_dm(
    db: &StorageBackend,
    context: &IngressContext,
    channel: &IngressChannel,
    session: everruns_contracts::typed_id::SessionId,
) -> Option<String> {
    let endpoint = channel.public_id.to_string();
    let rows = match db
        .recent_input_message_events_for_channel(session, &endpoint, 20)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%session, %error, "Scheduled Slack run: could not read the session's inputs");
            return None;
        }
    };
    rows.iter().find_map(|row| {
        super::actions::signed_slack_target(row, &context.public_id.to_string(), &endpoint)
            .ok()
            .flatten()
            .map(|(channel, _)| channel)
            .filter(|channel| channel.starts_with('D'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_keys_name_one_conversation() {
        assert_eq!(
            SlackConversation::parse("slack:user:U0123ABC"),
            Some(SlackConversation::User("U0123ABC".into()))
        );
        assert_eq!(
            SlackConversation::parse("slack:channel:C0123"),
            Some(SlackConversation::Channel("C0123".into()))
        );
        for bad in [
            "slack:user:C0123",
            "slack:user:u0123",
            "slack:thread:1700000000.1",
            "slack:channel:",
            "slack:user:U01 23",
            "teams:user:U0123",
            "slack:user:U0123:extra",
        ] {
            assert_eq!(SlackConversation::parse(bad), None, "{bad}");
        }
        let user = SlackConversation::parse("slack:user:W9").unwrap();
        assert_eq!(user.binding(), SessionBinding::Requester);
        assert_eq!(user.binding_key(), "slack:user:W9");
    }

    fn config(binding: SessionBinding, scoped: Option<&str>) -> SlackChannelConfig {
        serde_json::from_value(serde_json::json!({
            "signing_secret": "s",
            "bot_token": "xoxb-1",
            "session_strategy": binding,
            "channel_id": scoped,
        }))
        .unwrap()
    }

    #[test]
    fn targets_must_match_how_the_channel_binds() {
        let dm = SlackConversation::User("U1".into());
        let room = SlackConversation::Channel("C1".into());
        assert_eq!(
            target_problem(&config(SessionBinding::Requester, None), &dm),
            None
        );
        assert_eq!(
            target_problem(&config(SessionBinding::Conversation, None), &room),
            None
        );
        assert!(target_problem(&config(SessionBinding::Conversation, None), &dm).is_some());
        assert!(target_problem(&config(SessionBinding::Thread, None), &dm).is_some());
        // A channel scoped to C1 posts nowhere else.
        assert_eq!(
            target_problem(&config(SessionBinding::Conversation, Some("C1")), &room),
            None
        );
        assert!(target_problem(&config(SessionBinding::Conversation, Some("C2")), &room).is_some());
        let mut disconnected = config(SessionBinding::Requester, None);
        disconnected.bot_token.clear();
        assert!(target_problem(&disconnected, &dm).is_some());
    }
}
