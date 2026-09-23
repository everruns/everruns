//! The worker end of the Slack channel action seam (EVE-1024).
//!
//! Split out of `grpc_adapters` rather than added to it: that file is on the
//! file-size ratchet, and a per-integration client is exactly the kind of thing
//! that should not accrete there.

use async_trait::async_trait;
use everruns_internal_protocol::proto;

use crate::grpc_adapters::GrpcClient;

/// Forwards one session's Slack actions to the control plane (EVE-1024).
///
/// Bound to the org and session it was built for, so the capability holding it
/// has no argument it could vary to reach another session's channel. The
/// channel's `bot_token` stays in the control plane; only the action and its
/// outcome cross this client.
pub struct GrpcSlackActionInvoker {
    client: GrpcClient,
    org_id: i64,
    session_id: everruns_provider::typed_id::SessionId,
}

impl GrpcSlackActionInvoker {
    pub fn new(
        client: GrpcClient,
        org_id: i64,
        session_id: everruns_provider::typed_id::SessionId,
    ) -> Self {
        Self {
            client,
            org_id,
            session_id,
        }
    }
}

#[async_trait]
impl everruns_platform::slack_action::SlackActionInvoker for GrpcSlackActionInvoker {
    async fn invoke(
        &self,
        action: everruns_platform::slack_action::SlackAction,
    ) -> std::result::Result<
        everruns_platform::slack_action::SlackActionOutcome,
        everruns_platform::slack_action::SlackActionError,
    > {
        use everruns_platform::slack_action::{SlackActionError, SlackActionOutcome};

        let request = proto::InvokeSlackActionRequest {
            org_id: self.org_id,
            session_id: self.session_id.to_string(),
            action: Some(action.into()),
        };

        let mut client = self.client.inner.lock().await;
        let response = client
            .invoke_slack_action(request)
            .await
            .map_err(|error| SlackActionError::Transient(error.message().to_string()))?
            .into_inner();

        use proto::invoke_slack_action_response::Result as Wire;
        match response.result {
            Some(Wire::ReactionAdded(r)) => Ok(SlackActionOutcome::ReactionAdded {
                already_reacted: r.already_reacted,
            }),
            Some(Wire::MessageUpdated(r)) => Ok(SlackActionOutcome::MessageUpdated {
                channel: r.channel,
                timestamp: r.timestamp,
            }),
            Some(Wire::User(r)) => Ok(SlackActionOutcome::User {
                user_id: r.user_id,
                display_name: r.display_name,
                real_name: r.real_name,
                is_bot: r.is_bot,
                tz: r.tz,
            }),
            Some(Wire::FileUploaded(r)) => Ok(SlackActionOutcome::FileUploaded {
                file_id: r.file_id,
                permalink: r.permalink,
            }),
            Some(Wire::Error(error)) => Err(error.into()),
            // A control plane that answered without filling the oneof. Treated
            // as a fault rather than told to the model as a fact about its
            // Slack session.
            None => Err(SlackActionError::Transient(
                "control plane returned an empty Slack action result".to_string(),
            )),
        }
    }
}
