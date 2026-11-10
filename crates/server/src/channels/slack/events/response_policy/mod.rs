//! Participation is decided before session creation, acknowledgement, or tools.
//! Topic overlap alone is not an invitation to respond. Errors on an unmentioned
//! message therefore mean silence; explicit mentions and DMs never need a judge.

use std::collections::HashSet;

use crate::domains::agent_channels::record::slack_channel::{
    SlackChannelConfig, SlackResponsePolicy,
};
use crate::services::relevance::{HISTORY_LIMIT, RelevanceQuestion, bounded, question_state};
use serde_json::{Value, json};

use crate::api::channel_ingress::{IngressChannel, IngressContext};
use crate::storage::{AgentRow, EventRow, SessionRow};

use super::{SlackEvent, SlackState};
use everruns_contracts::typed_id::SessionId;

const HISTORY_SCAN_LIMIT: i32 = 32;

/// Whether the agent answers `event`. `session` is the conversation's session
/// when it has one: its recent thread messages are the judge's context.
pub(crate) async fn should_process_message(
    state: &SlackState,
    app: &IngressContext,
    endpoint: &IngressChannel,
    config: &SlackChannelConfig,
    event: &SlackEvent,
    session: Option<SessionId>,
) -> bool {
    if config.response_policy == SlackResponsePolicy::AllMessages
        || is_directed_message(event)
        || mentions_agent(state, config, endpoint, event).await
    {
        return true;
    }
    if config.response_policy == SlackResponsePolicy::MentionsOnly {
        tracing::debug!(endpoint_id = %endpoint.public_id, "Slack response policy ignored unmentioned message");
        return false;
    }
    let endpoint_id = endpoint.public_id.to_string();
    let decided = async {
        let (context, session, agent) =
            decision_state_and_session(state, app, event, session).await?;
        state
            .relevance_checker()
            .decide(RelevanceQuestion {
                org_id: app.org_id,
                agent: &agent,
                state: context,
                session: session.as_ref(),
                source: "slack_response_policy",
                subject: ("endpoint_id", &endpoint_id),
            })
            .await
    }
    .await;
    match decided {
        Ok(verdict) => {
            tracing::info!(
                endpoint_id,
                probability = verdict.probability,
                respond = verdict.respond,
                calibrated = verdict.calibrated,
                model = %verdict.model,
                judge = verdict.judge.as_str(),
                "Slack relevance decision"
            );
            verdict.respond
        }
        Err(error) => {
            tracing::warn!(endpoint_id, %error, "Slack relevance decision unavailable; ignoring unmentioned message");
            false
        }
    }
}

fn is_directed_message(event: &SlackEvent) -> bool {
    event.event_type == "app_mention"
        || event.channel_type.as_deref() == Some("im")
        || event
            .channel
            .as_deref()
            .is_some_and(|channel| channel.starts_with('D'))
}

/// Whether a plain `message` event @-mentions the agent's bot user.
///
/// Slack delivers one channel mention twice under the same `ts`: a `message`
/// (from `message.channels`) and an `app_mention`. Which comes first is not
/// guaranteed, so the `message` half must read as directed too, or the policy
/// answer would depend on arrival order. When Slack cannot say who the bot is,
/// this is `false` and the `app_mention` half is still answered.
async fn mentions_agent(
    state: &SlackState,
    config: &SlackChannelConfig,
    endpoint: &IngressChannel,
    event: &SlackEvent,
) -> bool {
    let Some(text) = event.text.as_deref().filter(|text| text.contains("<@")) else {
        return false;
    };
    let Some(bot) = super::resolve_slack_bot_user(
        &state.bot_user_cache,
        &config.bot_token,
        &endpoint.public_id.to_string(),
    )
    .await
    else {
        return false;
    };
    mentions_user(text, &bot)
}

/// `<@U123>` or `<@U123|name>` in Slack's message markup.
fn mentions_user(text: &str, user: &str) -> bool {
    text.match_indices("<@").any(|(at, _)| {
        let rest = &text[at + 2..];
        rest.strip_prefix(user)
            .is_some_and(|tail| tail.starts_with('>') || tail.starts_with('|'))
    })
}

#[cfg(test)]
async fn decision_state(
    state: &SlackState,
    app: &IngressContext,
    event: &SlackEvent,
    session: Option<SessionId>,
) -> anyhow::Result<Value> {
    Ok(decision_state_and_session(state, app, event, session)
        .await?
        .0)
}

/// The decision's state, the Slack session it belongs to when one exists,
/// and the endpoint's agent.
async fn decision_state_and_session(
    state: &SlackState,
    app: &IngressContext,
    event: &SlackEvent,
    session: Option<SessionId>,
) -> anyhow::Result<(Value, Option<SessionRow>, AgentRow)> {
    let existing = match session {
        Some(session) => state.db.get_session(app.org_id, session).await?,
        None => None,
    };
    let agent_id = app
        .agent_id
        .ok_or_else(|| anyhow::anyhow!("Slack endpoint has no agent"))?;
    let agent = state
        .db
        .get_agent(app.org_id, agent_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Slack endpoint agent missing"))?;
    // Judge the purpose the session will run: the agent's current
    // configuration (agent versions and pinning are retired).
    let recent = match (&existing, event.thread_ts.as_deref()) {
        (Some(session), Some(thread_ts)) => {
            let events = state
                .db
                .list_message_events_limited(session.id, Some(HISTORY_SCAN_LIMIT))
                .await?;
            thread_history(
                &events,
                event.channel.as_deref().unwrap_or_default(),
                thread_ts,
            )
        }
        _ => vec![],
    };
    let latest = json!({
        "text": bounded(event.text.as_deref().unwrap_or_default(), 2048),
        "author": bounded(event.user.as_deref().unwrap_or_default(), 128),
        "files": event.files.iter().take(4).map(|file| bounded(file.name.as_deref().unwrap_or_default(), 128)).collect::<Vec<_>>(),
    });
    let value = question_state(&agent, None, latest, recent);
    Ok((value, existing, agent))
}

// THREAT[TM-SLACK-011]: unrelated threads and tool results must not steer participation.
/// Only this endpoint's persisted messages from this Slack thread are context.
/// Shared per-channel/per-user sessions must not import unrelated conversations.
fn thread_history(events: &[EventRow], channel: &str, thread_ts: &str) -> Vec<Value> {
    let mut inputs = HashSet::new();
    let mut recent = Vec::new();
    for event in events {
        if !matches!(
            event.event_type.as_str(),
            "input.message" | "output.message.completed"
        ) {
            continue;
        }
        let message = &event.data["message"];
        let metadata = &message["metadata"];
        let belongs = if event.event_type == "input.message" {
            let matches = metadata["slack_channel"].as_str() == Some(channel)
                && metadata["slack_thread_ts"]
                    .as_str()
                    .or_else(|| metadata["slack_ts"].as_str())
                    == Some(thread_ts);
            if matches && let Some(id) = message["id"].as_str() {
                inputs.insert(id);
            }
            matches
        } else {
            event.context["input_message_id"]
                .as_str()
                .is_some_and(|id| inputs.contains(id))
        };
        if belongs {
            let mut text = String::new();
            for value in thread_message_texts(&event.event_type, &event.data) {
                text.push_str(&bounded(&value, 512 - text.len()));
                if text.len() == 512 {
                    break;
                }
            }
            if !text.is_empty() {
                recent.push(json!({"role": message["role"], "text": text}));
            }
        }
    }
    recent
        .into_iter()
        .rev()
        .take(HISTORY_LIMIT)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// What one stored message said in the thread: a person's text, or what the
/// agent said. Agent commentary is never context. In explicit communication
/// the agent's words are its `send_message` calls.
fn thread_message_texts(event_type: &str, data: &Value) -> Vec<String> {
    let message = &data["message"];
    let parts = message["content"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if event_type == "input.message" {
        return parts
            .iter()
            .filter_map(|part| part["text"].as_str().map(str::to_owned))
            .collect();
    }
    let mut texts: Vec<String> = everruns_core::conversation::said_in_event(event_type, data)
        .into_iter()
        .collect();
    texts.extend(parts.iter().filter_map(|part| {
        (part["type"].as_str() == Some("tool_call")
            && part["name"].as_str() == Some(everruns_core::conversation::SEND_MESSAGE_TOOL_NAME))
        .then(|| part["arguments"]["text"].as_str().map(str::to_owned))
        .flatten()
    }));
    texts
}

#[cfg(test)]
mod tests;
