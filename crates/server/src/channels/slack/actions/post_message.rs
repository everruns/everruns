//! Explicit posting binds to the signed ingress of one invocation. Session-wide
//! thread context is insufficient for per-user and per-channel session reuse.
use super::*;

/// The channel a server-authored event provenance names as initiator, or
/// `None` when the provenance is not channel-initiated.
///
/// Writes are channel-shaped (`execution_metadata::channel_message_metadata`).
/// Reads also accept the two older shapes, `endpoint` and the frozen `app`,
/// because the caller gate reads *persisted* provenance: a session mid-thread
/// when either rename shipped still carries the old key, and rejecting it would
/// silence a live Slack conversation until its next inbound message. All three
/// name the same channel public id, so the authorized set does not widen. Same
/// posture as the permanent ingress aliases — nothing new is written in an old
/// shape.
fn initiator_channel_id(provenance: &serde_json::Value) -> Option<&str> {
    let initiator = &provenance["initiator"];
    match initiator["type"].as_str() {
        Some("channel") => initiator["channel_id"].as_str(),
        Some("endpoint") => initiator["endpoint_id"].as_str(),
        Some("app") => initiator["app_id"].as_str(),
        _ => None,
    }
}

impl DbSlackActionInvoker {
    pub(crate) async fn trusted_post_route(
        &self,
        input_message_id: &str,
    ) -> Result<(String, String), SlackActionError> {
        let context = self.resolve_context(Some(input_message_id)).await?;
        Ok((context.channel, context.thread_ts))
    }

    pub(crate) async fn resolve_action_channel(
        &self,
        session: &crate::storage::SessionRow,
    ) -> Result<(everruns_contracts::typed_id::AppId, AgentChannel), SlackActionError> {
        let Some(app_internal_id) = session.app_id else {
            use crate::api::channel_ingress::{channel_liveness, resolve_channel};
            let channel_id = session.channel_id.ok_or(SlackActionError::NoSlackSession)?;
            let public_id = self
                .db
                .get_agent_channel_public_id(self.org_id, channel_id)
                .await
                .map_err(|e| SlackActionError::Transient(e.to_string()))?
                .ok_or(SlackActionError::ChannelUnavailable)?;
            let (context, endpoint) =
                resolve_channel(&self.db, self.encryption.as_ref(), &public_id)
                    .await
                    .map_err(|e| SlackActionError::Transient(e.to_string()))?
                    .ok_or(SlackActionError::ChannelUnavailable)?;
            // THREAT[TM-SLACK-005]: native endpoints have no archival App row.
            // Resolve only the authoritative session FK and recheck its agent.
            if context.org_id != self.org_id
                || context.historical_app_id.is_some()
                || session.agent_id.map(|id| id.uuid()) != Some(context.agent_internal_id)
                || endpoint.internal_id != channel_id
            {
                return Err(SlackActionError::NoSlackSession);
            }
            channel_liveness(&context, &endpoint)
                .map_err(|_| SlackActionError::ChannelUnavailable)?;
            let app_id = context.public_id;
            return Ok((app_id, endpoint.into_channel()));
        };
        // Decide which endpoint before loading the app, so the tag fallback and
        // the FK agree on a single target.
        let endpoint_selector = match session.channel_id {
            Some(internal_id) => ChannelSelector::Internal(internal_id),
            None => session
                .tags
                .iter()
                .find_map(|tag| tag.strip_prefix(SLACK_CHANNEL_TAG_PREFIX))
                .map(|public_id| ChannelSelector::Public(public_id.to_string()))
                .ok_or(SlackActionError::NoSlackSession)?,
        };

        let app = crate::domains::apps::queries::get_by_internal_id(
            &self.db,
            self.encryption.as_ref(),
            self.org_id,
            app_internal_id,
        )
        .await
        .map_err(|e| SlackActionError::Transient(e.to_string()))?
        .ok_or(SlackActionError::ChannelUnavailable)?;

        let endpoint = select_slack_channel(&app, &endpoint_selector)
            .ok_or(SlackActionError::NoSlackSession)?;

        Ok((app.public_id, endpoint.clone()))
    }

    pub(super) async fn resolve_post_context(
        &self,
        input_message_id: &str,
        app_public_id: everruns_contracts::typed_id::AppId,
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
        let provenance = input
            .metadata
            .as_ref()
            .ok_or(SlackActionError::NoSlackSession)?;
        // MessageService records the external participant as runtime subject
        // and nests the server-authored app provenance under `source`.
        let provenance = if provenance["type"] == "virtual_user" {
            &provenance["source"]
        } else {
            provenance
        };
        let metadata = &input.data["message"]["metadata"];
        // THREAT[TM-SLACK-005]: bind posts to this invocation's signed ingress.
        // Event provenance is server-authored; message metadata alone is
        // caller-controlled on ordinary API inputs and cannot authorize a post.
        // Writes are channel-shaped (`execution_metadata::channel_message_metadata`).
        // Reads also accept the frozen `app` shape, because this gate reads
        // *persisted* provenance: a session mid-thread when this shipped still
        // carries the old key, and rejecting it would silence a live Slack
        // conversation until its next inbound message. Same posture as the
        // permanent ingress aliases — nothing new is written in that shape.
        if initiator_channel_id(provenance) != Some(app_public_id.to_string().as_str())
            || metadata["_app_channel_id"].as_str() != Some(endpoint.public_id.to_string().as_str())
        {
            return Err(SlackActionError::NoSlackSession);
        }
        let channel = metadata["slack_channel"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or(SlackActionError::NoSlackSession)?;
        let thread_ts = metadata["slack_thread_ts"].as_str().filter(|v| !v.is_empty())
                .ok_or_else(|| SlackActionError::InvalidArgument("The input has no Slack thread reference; send a new Slack message before posting".into()))?;
        if config.channel_id.as_deref().is_some_and(|id| id != channel) {
            return Err(SlackActionError::ChannelUnavailable);
        }
        Ok(SlackActionContext {
            channel_id: endpoint.internal_id,
            channel_revision: endpoint.updated_at,
            bot_token: config.bot_token,
            channel: channel.into(),
            thread_ts: thread_ts.into(),
        })
    }
}

pub(super) async fn post_message(
    api_base: &str,
    session_id: SessionId,
    context: &SlackActionContext,
    input_message_id: String,
    tool_call_id: String,
    text: String,
) -> Result<SlackActionOutcome, SlackActionError> {
    use everruns_core::channel_messaging::MAX_CHANNEL_MESSAGE_CHARS;
    if tool_call_id.trim().is_empty()
        || text.trim().is_empty()
        || text.chars().count() > MAX_CHANNEL_MESSAGE_CHARS
    {
        return Err(SlackActionError::InvalidArgument(
            "Posting requires a tool call reference and 1–12000 characters of text".into(),
        ));
    }
    let correlation = crate::channels::slack::delivery::SlackCorrelation {
        session_id: session_id.to_string(),
        input_message_id,
    };
    let mut payloads = crate::channels::slack::delivery::build_post_payloads(
        &context.channel,
        &context.thread_ts,
        &text,
        Some(&correlation),
    );
    if payloads.len() != 1 {
        return Err(SlackActionError::InvalidArgument(
            "Message is too large for one post; share a file instead".into(),
        ));
    }
    let mut payload = payloads.remove(0);
    payload["metadata"]["event_payload"]["tool_call_id"] = json!(tool_call_id);
    // Durable Act owns at-most-once claims. Do not retry an ambiguous
    // HTTP failure here: Slack may already have accepted the post.
    let response = slack_api_call(api_base, &context.bot_token, "chat.postMessage", payload)
        .await
        .map_err(super::action_error)?;
    let timestamp = response["ts"]
        .as_str()
        .filter(|ts| !ts.is_empty())
        .ok_or_else(|| {
            SlackActionError::Transient(
                "Slack accepted the message but returned no message reference".into(),
            )
        })?;
    Ok(SlackActionOutcome::MessagePosted {
        channel: context.channel.clone(),
        timestamp: timestamp.into(),
    })
}

#[cfg(test)]
mod initiator_provenance_tests {
    use super::initiator_channel_id;
    use serde_json::json;

    #[test]
    fn channel_shaped_provenance_names_its_channel() {
        let p = json!({"initiator": {"type": "channel", "channel_id": "app_01"}});
        assert_eq!(initiator_channel_id(&p), Some("app_01"));
    }

    #[test]
    fn endpoint_shaped_provenance_still_names_its_channel() {
        // Persisted by the short-lived `endpoint` writer.
        let p = json!({"initiator": {"type": "endpoint", "endpoint_id": "app_01"}});
        assert_eq!(initiator_channel_id(&p), Some("app_01"));
    }

    #[test]
    fn frozen_app_shaped_provenance_still_names_its_channel() {
        // Persisted by the pre-rename writer. A live Slack thread must keep
        // posting across the deploy that renamed the key.
        let p = json!({"initiator": {"type": "app", "app_id": "app_01"}});
        assert_eq!(initiator_channel_id(&p), Some("app_01"));
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
            json!({"initiator": {"channel_id": "app_01"}}),
            json!({"initiator": {"type": "endpoint"}}),
            json!({"initiator": {"endpoint_id": "app_01"}}),
            json!({}),
        ] {
            assert_eq!(initiator_channel_id(&p), None, "authorized: {p}");
        }
    }
}
