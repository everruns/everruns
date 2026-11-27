//! Explicit posting binds to the signed ingress of one invocation. Session-wide
//! thread context is insufficient for per-user and per-channel session reuse.
use super::*;
use crate::domains::agent_channels::record::SessionBinding;

/// The channel a server-authored event provenance names as initiator, or
/// `None` when the provenance is not channel-initiated.
///
/// Only server-authored channel provenance can authorize a post.
fn initiator_channel_id(provenance: &serde_json::Value) -> Option<&str> {
    let initiator = &provenance["initiator"];
    match initiator["type"].as_str() {
        Some("channel") => initiator["channel_id"].as_str(),
        Some("endpoint") => initiator["endpoint_id"].as_str(),
        _ => None,
    }
}

/// The Slack conversation `(channel, thread_ts)` a persisted input came from,
/// when its server-authored provenance binds it to this channel.
/// `Ok(None)` is a bound input without a thread reference.
pub(crate) fn slack_target(
    input: &crate::storage::EventRow,
    ingress_public_id: &str,
    endpoint_public_id: &str,
) -> Result<Option<(String, String)>, SlackActionError> {
    let provenance = input
        .metadata
        .as_ref()
        .ok_or(SlackActionError::NoSlackSession)?;
    // MessageService records the external participant as runtime subject
    // and nests the server-authored channel provenance under `source`.
    let provenance = if provenance["type"] == "virtual_user" {
        &provenance["source"]
    } else {
        provenance
    };
    let metadata = &input.data["message"]["metadata"];
    // THREAT[TM-SLACK-005]: bind posts to this invocation's signed ingress.
    // Event provenance is server-authored; message metadata alone is
    // caller-controlled on ordinary API inputs and cannot authorize a post.
    if initiator_channel_id(provenance) != Some(ingress_public_id)
        || metadata["_channel_id"].as_str() != Some(endpoint_public_id)
    {
        return Err(SlackActionError::NoSlackSession);
    }
    let channel = metadata["slack_channel"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or(SlackActionError::NoSlackSession)?;
    // A top-level DM message is answered in the DM itself (empty thread).
    if metadata[crate::channels::slack::host::SLACK_REPLY_INLINE] == true {
        return Ok(Some((channel.to_string(), String::new())));
    }
    Ok(metadata["slack_thread_ts"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(|thread_ts| (channel.to_string(), thread_ts.to_string())))
}

impl DbSlackActionInvoker {
    pub(crate) async fn resolve_action_channel(
        &self,
        session: &crate::storage::SessionRow,
    ) -> Result<(everruns_contracts::typed_id::AgentChannelId, AgentChannel), SlackActionError>
    {
        use crate::api::channel_ingress::{channel_liveness, resolve_channel};

        // The structural channel FK is authoritative.
        let public_id = match session.channel_id {
            Some(channel_id) => self
                .db
                .get_agent_channel_public_id(self.org_id, channel_id)
                .await
                .map_err(|e| SlackActionError::Transient(e.to_string()))?
                .ok_or(SlackActionError::ChannelUnavailable)?,
            None => return Err(SlackActionError::NoSlackSession),
        };
        let (context, channel) = resolve_channel(&self.db, self.encryption.as_ref(), &public_id)
            .await
            .map_err(|e| SlackActionError::Transient(e.to_string()))?
            .ok_or(SlackActionError::ChannelUnavailable)?;

        // THREAT[TM-SLACK-005]: require the same tenant, agent and channel.
        if context.org_id != self.org_id
            || channel.channel_type != ChannelType::Slack
            || session
                .channel_id
                .is_some_and(|id| id != channel.internal_id)
            || match session.agent_id {
                Some(id) => id.uuid() != context.agent_internal_id,
                None => true,
            }
        {
            return Err(SlackActionError::NoSlackSession);
        }
        channel_liveness(&context, &channel).map_err(|_| SlackActionError::ChannelUnavailable)?;
        Ok((context.public_id, channel.into_channel()))
    }

    pub(super) async fn resolve_post_context(
        &self,
        input_message_id: &str,
        ingress_public_id: everruns_contracts::typed_id::AgentChannelId,
        endpoint: &AgentChannel,
        config: crate::domains::agent_channels::record::slack_channel::SlackChannelConfig,
    ) -> Result<SlackActionContext, SlackActionError> {
        let session_id = self.session_id;
        let message_id = input_message_id.parse().map_err(|_| {
            SlackActionError::InvalidArgument("Invalid input message reference".into())
        })?;
        let input = self
            .db
            .find_input_message_event(session_id, message_id)
            .await
            .map_err(|e| SlackActionError::Transient(e.to_string()))?
            .ok_or(SlackActionError::NoSlackSession)?;
        let ingress_id = ingress_public_id.to_string();
        let endpoint_id = endpoint.public_id.to_string();
        let target = match self
            .scheduled_target(&input, &ingress_id, &endpoint_id, config.session_strategy)
            .await?
        {
            Some(target) => Some(target),
            None => slack_target(&input, &ingress_id, &endpoint_id)?,
        };
        let (mut channel, mut thread_ts) = target.ok_or_else(|| {
            SlackActionError::InvalidArgument(
                "The input has no Slack thread reference; send a new Slack message before posting"
                    .into(),
            )
        })?;
        // A message that steered this turn mid-flight gets no delivery of its
        // own, so the answer goes to the newest Slack input of the turn: the
        // question it answers. Same signed-ingress checks; anything else keeps
        // the turn's own input.
        if let Some(latest) = self
            .db
            .latest_input_message_event_since(session_id, input.sequence)
            .await
            .map_err(|e| SlackActionError::Transient(e.to_string()))?
            .filter(|latest| latest.sequence > input.sequence)
            && let Ok(Some(target)) = slack_target(&latest, &ingress_id, &endpoint_id)
        {
            (channel, thread_ts) = target;
        }
        if config.channel_id.as_deref().is_some_and(|id| id != channel) {
            return Err(SlackActionError::ChannelUnavailable);
        }
        Ok(SlackActionContext {
            channel_id: endpoint.internal_id,
            channel_revision: endpoint.updated_at,
            installation: endpoint.public_id.to_string(),
            team: config.team_id,
            bot_token: config.bot_token,
            channel,
            thread_ts,
        })
    }

    /// The Slack conversation a session-schedule turn speaks to, or `None`
    /// when `input` is not a scheduled input.
    ///
    /// Decision (schedules in Slack sessions, D1): resolved here at post time
    /// rather than stamped onto the input when the schedule fires. The input
    /// keeps its `schedule` provenance and `schedule_id`, so the trace still
    /// says "scheduled", and no Slack-shaped provenance is ever written for a
    /// message Slack did not send. The target comes only from the session's
    /// latest input that passes the same signed-ingress check
    /// (THREAT[TM-SLACK-005]); the schedule's text never chooses a channel, and
    /// without such an input the post fails as before (`NoSlackSession`).
    ///
    /// Where it lands: a thread-bound session is that thread, so it keeps it.
    /// Any other binding posts top-level, which for a DM is the DM itself, so a
    /// scheduled post never lands in whichever thread happened to be last.
    async fn scheduled_target(
        &self,
        input: &crate::storage::EventRow,
        ingress_id: &str,
        endpoint_id: &str,
        binding: SessionBinding,
    ) -> Result<Option<(String, String)>, SlackActionError> {
        let initiator = input
            .metadata
            .as_ref()
            .and_then(|provenance| provenance["initiator"]["type"].as_str());
        if initiator != Some("schedule") {
            return Ok(None);
        }
        let candidates = self
            .db
            .recent_input_message_events_for_channel(self.session_id, endpoint_id, 20)
            .await
            .map_err(|e| SlackActionError::Transient(e.to_string()))?;
        let (channel, thread_ts) = candidates
            .iter()
            .find_map(|row| slack_target(row, ingress_id, endpoint_id).ok().flatten())
            .ok_or(SlackActionError::NoSlackSession)?;
        Ok(Some(match binding {
            SessionBinding::Thread => (channel, thread_ts),
            _ => (channel, String::new()),
        }))
    }
}

pub(super) async fn post_message(
    api_base: &str,
    session_id: SessionId,
    context: &SlackActionContext,
    provider_context: &everruns_contracts::runtime::channel::DeliveryContext,
    input_message_id: String,
    tool_call_id: String,
    text: String,
) -> Result<SlackActionOutcome, SlackActionError> {
    use everruns_core::conversation::MAX_MESSAGE_CHARS;
    if tool_call_id.trim().is_empty()
        || text.trim().is_empty()
        || text.chars().count() > MAX_MESSAGE_CHARS
    {
        return Err(SlackActionError::InvalidArgument(
            "Posting requires a tool call reference and 1–12000 characters of text".into(),
        ));
    }
    let options = channelkit_slack::SlackPostOptions {
        metadata: Some(channelkit_slack::SlackMessageMetadata {
            event_type: "everruns_agent_reply".into(),
            event_payload: [
                ("session_id".into(), json!(session_id.to_string())),
                ("input_message_id".into(), json!(input_message_id)),
                ("tool_call_id".into(), json!(tool_call_id)),
            ]
            .into(),
        }),
    };
    // Durable Act owns at-most-once claims. Do not retry an ambiguous
    // HTTP failure here: Slack may already have accepted the post.
    let factory = everruns_integrations::slack_channel::bridge::direct_factory()
        .map_err(everruns_integrations::slack_channel::web_api::provider_error)
        .map_err(super::action_error)?;
    let client = factory
        .for_context(provider_context, api_base)
        .await
        .map_err(everruns_integrations::slack_channel::web_api::provider_error)
        .map_err(super::action_error)?;
    let target = everruns_integrations::slack_channel::bridge::target(&client, provider_context)
        .map_err(everruns_integrations::slack_channel::web_api::provider_error)
        .map_err(super::action_error)?;
    let receipt = client
        .post_markdown_single(
            &target,
            &everruns_integrations::slack_channel::markdown::neutralize_broadcasts(&text),
            &options,
            None,
        )
        .await
        .map_err(everruns_integrations::slack_channel::web_api::provider_error)
        .map_err(super::action_error)?;
    let timestamp = receipt
        .messages
        .into_iter()
        .next()
        .ok_or_else(|| SlackActionError::DeliveryUnknown { confirmed: vec![] })?
        .id;
    Ok(SlackActionOutcome::MessagePosted {
        channel: context.channel.clone(),
        timestamp,
    })
}

#[cfg(test)]
mod initiator_provenance_tests {
    use super::initiator_channel_id;
    use serde_json::json;

    #[test]
    fn channel_shaped_provenance_names_its_channel() {
        let p = json!({"initiator": {"type": "channel", "channel_id": "appchan_01"}});
        assert_eq!(initiator_channel_id(&p), Some("appchan_01"));
    }

    #[test]
    fn endpoint_shaped_provenance_still_names_its_channel() {
        // Persisted by the short-lived `endpoint` writer.
        let p = json!({"initiator": {"type": "endpoint", "endpoint_id": "appchan_01"}});
        assert_eq!(initiator_channel_id(&p), Some("appchan_01"));
    }

    #[test]
    fn other_initiators_authorize_nothing() {
        // THREAT[TM-SLACK-005]: only channel ingress authorizes a post. A
        // user- or trigger-initiated turn must not, and neither must a
        // caller-shaped object that merely carries an id.
        for p in [
            json!({"initiator": {"type": "user", "user_id": "user_01"}}),
            json!({"initiator": {"type": "agent_trigger", "trigger_id": "trg_01"}}),
            json!({"initiator": {"type": "channel"}}),
            json!({"initiator": {"channel_id": "appchan_01"}}),
            json!({"initiator": {"type": "endpoint"}}),
            json!({"initiator": {"endpoint_id": "appchan_01"}}),
            json!({}),
        ] {
            assert_eq!(initiator_channel_id(&p), None, "authorized: {p}");
        }
    }
}
