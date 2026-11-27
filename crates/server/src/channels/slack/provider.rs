//! Channelkit mechanics behind the server's existing Slack routing policy.
use super::{
    events::{SlackEvent, SlackState},
    socket_mode::SOCKET_MODE_PATH,
};
use crate::domains::agent_channels::record::slack_channel::{SlackChannelConfig, SlackConnection};
use channelkit::{Control, EventKind, ReactionChange};
use everruns_contracts::runtime::{
    channel::ChannelError, channel_driver::ChannelRequest, egress::EgressService,
};
use everruns_integrations::slack_channel::{
    bridge::direct_factory,
    ingress::{HttpIngress, VerifiedHttpIngress},
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) fn error(error: channelkit::Error) -> ChannelError {
    match error {
        channelkit::Error::Authentication | channelkit::Error::Scope => {
            ChannelError::Unauthorized("Slack authentication or scope".into())
        }
        _ => ChannelError::BadRequest("Invalid Slack provider input".into()),
    }
}

pub(super) fn egress(state: &SlackState, org_id: i64) -> Option<Arc<dyn EgressService>> {
    state
        .org_decisions
        .as_ref()
        .map(|host| host.egress.clone())
        .map(|egress| crate::domains::messages::intake::org_egress(egress, org_id))
}

pub(super) async fn receive(
    installation: &str,
    config: &SlackChannelConfig,
    request: &ChannelRequest,
    egress: Option<Arc<dyn EgressService>>,
) -> Result<VerifiedHttpIngress, ChannelError> {
    let factory = direct_factory().map_err(error)?;
    if request.path != SOCKET_MODE_PATH || config.connection != SlackConnection::SocketMode {
        return HttpIngress {
            installation,
            team: config.team_id.as_deref(),
            token: (!config.bot_token.is_empty()).then_some(config.bot_token.as_str()),
            signing_secret: &config.signing_secret,
            api: super::events::SLACK_API_BASE,
            factory,
        }
        .receive(request, egress)
        .await;
    }
    // Only the authenticated socket supervisor creates this request path and
    // retains the original envelope identifier. HTTP routes always leave it empty.
    let raw = request.body_json()?;
    let team = raw["payload"]["team_id"]
        .as_str()
        .or_else(|| raw["payload"]["team"]["id"].as_str())
        .ok_or_else(|| ChannelError::BadRequest("Slack workspace".into()))?;
    if config
        .team_id
        .as_deref()
        .is_some_and(|expected| expected != team)
    {
        return Err(error(channelkit::Error::Scope));
    }
    let client = factory
        .for_credentials(
            installation,
            Some(team),
            &config.bot_token,
            super::events::SLACK_API_BASE,
            egress,
        )
        .await
        .map_err(error)?;
    let ingress = client.receive_socket_mode(&request.body).map_err(error)?;
    Ok(VerifiedHttpIngress::Events {
        team: team.into(),
        ingress: Box::new(ingress),
    })
}

/// A host policy projection of an already authenticated, normalized event.
/// Private URLs and response credentials never enter routing/session metadata.
pub(super) fn policy_event(
    kind: EventKind,
    context: &channelkit_slack::SlackEventContext,
) -> Option<(Value, SlackEvent)> {
    let value = match kind {
        EventKind::Message(message) => json!({
            "type": if message.mentioned { "app_mention" } else { "message" },
            "user": message.author.id, "text": message.text,
            "channel": message.reference.conversation.channel,
            "ts": message.reference.id, "thread_ts": message.reply_to,
            "channel_type": context.channel_type, "subtype": context.subtype,
            "files": context.files, "attachments": context.attachments,
        }),
        EventKind::Reaction(reaction) => {
            let (kind, name) = match reaction.change {
                ReactionChange::Added(name) => ("reaction_added", name),
                ReactionChange::Removed(name) => ("reaction_removed", name),
                _ => return None,
            };
            json!({"type": kind, "user": reaction.actor.id, "reaction": name,
                "item": {"type": "message", "channel": reaction.target.conversation.channel, "ts": reaction.target.id}})
        }
        EventKind::Control {
            conversation,
            actor,
            action,
        } => {
            let mut value = json!({"user": actor.map(|actor| actor.id),
                "channel": conversation.channel, "thread_ts": conversation.thread,
                "assistant_thread": {"channel_id": conversation.channel, "thread_ts": conversation.thread}});
            match action {
                Control::Stop => value["type"] = json!("agent_session_stopped"),
                Control::Title(title) => {
                    value["type"] = json!("agent_session_title_changed");
                    value["title"] = json!(title);
                }
                Control::View(view) => {
                    value["type"] = json!("app_context_changed");
                    value["assistant_thread"]["context"] = json!(view);
                }
                _ => return None,
            }
            value
        }
        _ => return None,
    };
    let event = serde_json::from_value(value.clone()).ok()?;
    Some((value, event))
}

pub(super) fn cache_key(installation: &str, token: &str, subject: &str) -> String {
    format!(
        "{installation}:{}:{subject}",
        everruns_integrations::slack_channel::web_api::tool_installation(token)
    )
}

pub(super) async fn user_name(
    state: &SlackState,
    installation: &str,
    config: &SlackChannelConfig,
    user: &str,
    egress: Option<Arc<dyn EgressService>>,
) -> Option<String> {
    let key = cache_key(installation, &config.bot_token, user);
    if let Some(value) = state.user_name_cache.get(&key) {
        return value;
    }
    let factory = direct_factory().ok()?;
    let client = factory
        .for_credentials(
            installation,
            config.team_id.as_deref(),
            &config.bot_token,
            super::events::SLACK_API_BASE,
            egress,
        )
        .await
        .ok()?;
    match client
        .api_call("users.info", json!({"user": user}), false)
        .await
    {
        Ok(body) => {
            let name = display_name(&body["user"]);
            state.user_name_cache.insert(key, name.clone());
            name
        }
        Err(channelkit::Error::Rejected { code })
            if everruns_integrations::slack_channel::web_api::PERMANENT_SLACK_ERRORS
                .contains(&code.as_str()) =>
        {
            state.user_name_cache.insert(key, None);
            None
        }
        Err(_) => None,
    }
}

pub(super) async fn bot_user(
    state: &SlackState,
    installation: &str,
    config: &SlackChannelConfig,
    egress: Option<Arc<dyn EgressService>>,
) -> Option<String> {
    let key = cache_key(installation, &config.bot_token, "bot");
    if let Some(Some(user)) = state.bot_user_cache.get(&key) {
        return Some(user);
    }
    let identity = direct_factory()
        .ok()?
        .scoped_identity(
            installation,
            &config.bot_token,
            super::events::SLACK_API_BASE,
            egress,
        )
        .await
        .ok()?;
    if config
        .team_id
        .as_deref()
        .is_some_and(|team| team != identity.team_id)
    {
        return None;
    }
    state
        .bot_user_cache
        .insert(key, Some(identity.user_id.clone()));
    Some(identity.user_id)
}

/// Host display policy projects only a name, never the profile's email/phone.
pub(super) fn display_name(user: &Value) -> Option<String> {
    [
        &user["profile"]["display_name"],
        &user["real_name"],
        &user["name"],
    ]
    .into_iter()
    .filter_map(Value::as_str)
    .find(|name| !name.is_empty())
    .map(str::to_owned)
}
