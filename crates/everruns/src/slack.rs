//! Slack-native action tools with an explicitly bound conversation. Stability: alpha.
use crate::capability::async_trait;
use everruns_contracts::slack_action::*;
use everruns_integrations::slack_channel::{
    actions,
    web_api::{SLACK_API_BASE, slack_api_call_scoped},
};
use std::{fmt, sync::Arc};

/// Slack action credentials and a trusted channel/thread selected by the application.
/// Model arguments cannot redirect actions to another conversation.
#[derive(Clone)]
pub struct SlackActions {
    token: Arc<str>,
    channel: String,
    thread: String,
    api_base: String,
    egress: Option<Arc<dyn everruns_core::EgressService>>,
}
impl fmt::Debug for SlackActions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlackActions")
            .field("channel", &self.channel)
            .field("thread", &self.thread)
            .finish_non_exhaustive()
    }
}
impl SlackActions {
    /// Bind the bot to one conversation. An empty thread means the channel itself.
    pub fn new(
        token: impl Into<String>,
        channel: impl Into<String>,
        thread: impl Into<String>,
    ) -> Self {
        Self {
            token: token.into().into(),
            channel: channel.into(),
            thread: thread.into(),
            api_base: SLACK_API_BASE.into(),
            egress: None,
        }
    }
    /// Use a trusted Slack API root, normally only for testing.
    pub fn api_base(mut self, url: impl Into<String>) -> Self {
        self.api_base = url.into();
        self
    }
    /// Route credential discovery, actions and every media hop through this host service.
    pub fn egress(mut self, egress: Arc<dyn everruns_core::EgressService>) -> Self {
        self.egress = Some(egress);
        self
    }
    fn delivery_context(&self) -> everruns_contracts::runtime::channel::DeliveryContext {
        let mut context = everruns_integrations::slack_channel::web_api::tool_context(
            &self.token,
            &self.channel,
            &self.thread,
        );
        context.egress = self.egress.clone();
        context
    }
    async fn authorize_message(
        &self,
        channel: &str,
        timestamp: &str,
    ) -> Result<(), SlackActionError> {
        // THREAT[TM-FW-002]: model-selected timestamps cannot escape the bound thread.
        self.authorize(channel)?;
        if self.thread.is_empty() || timestamp == self.thread {
            return Ok(());
        }
        let body = slack_api_call_scoped(
            &self.api_base,
            &self.delivery_context(),
            "conversations.replies",
            serde_json::json!({
                "channel": channel, "ts": self.thread,
                "oldest": timestamp, "latest": timestamp,
                "inclusive": true, "limit": 100,
            }),
        )
        .await
        .map_err(actions::action_error)?;
        let valid = body["messages"].as_array().into_iter().flatten().any(|m| {
            m["ts"].as_str() == Some(timestamp)
                && m["thread_ts"].as_str() == Some(self.thread.as_str())
        });
        if valid {
            Ok(())
        } else {
            Err(SlackActionError::InvalidArgument(
                "Slack message is outside the bound thread".into(),
            ))
        }
    }
    fn authorize(&self, channel: &str) -> Result<(), SlackActionError> {
        if self.token.trim().is_empty() {
            return Err(SlackActionError::NotConfigured);
        }
        if channel.is_empty() || channel != self.channel {
            return Err(SlackActionError::InvalidArgument(
                "Slack action targets a different conversation".into(),
            ));
        }
        Ok(())
    }
}
#[async_trait]
impl SlackActionInvoker for SlackActions {
    async fn invoke(&self, action: SlackAction) -> Result<SlackActionOutcome, SlackActionError> {
        match action {
            SlackAction::AddReaction {
                channel,
                timestamp,
                name,
            } => {
                self.authorize_message(&channel, &timestamp).await?;
                actions::add_reaction_scoped(
                    &self.api_base,
                    &self.delivery_context(),
                    &timestamp,
                    &name,
                )
                .await
                .map_err(actions::action_error)
            }
            SlackAction::UpdateMessage {
                channel,
                timestamp,
                text,
            } => {
                self.authorize_message(&channel, &timestamp).await?;
                actions::update_message_scoped(
                    &self.api_base,
                    &self.delivery_context(),
                    &timestamp,
                    &text,
                )
                .await
            }
            SlackAction::LookupUser { user_id } => {
                self.authorize(&self.channel)?;
                actions::lookup_user_scoped(&self.api_base, &self.delivery_context(), &user_id)
                    .await
            }
            SlackAction::UploadFile {
                channel,
                thread_ts,
                filename,
                content,
                initial_comment,
            } => {
                let expected = (!self.thread.is_empty()).then_some(self.thread.clone());
                // No channel: this binding's own conversation.
                let (channel, thread_ts) = match channel {
                    None => (self.channel.clone(), expected.clone()),
                    Some(channel) => (channel, thread_ts),
                };
                self.authorize(&channel)?;
                if thread_ts != expected {
                    return Err(SlackActionError::InvalidArgument(
                        "Slack upload targets a different thread".into(),
                    ));
                }
                actions::upload_file_scoped(
                    &self.api_base,
                    &self.delivery_context(),
                    &filename,
                    content,
                    initial_comment.as_deref(),
                )
                .await
            }
            SlackAction::PostMessage {
                input_message_id,
                tool_call_id,
                text,
                files,
            } => {
                self.authorize(&self.channel)?;
                let id = uuid::Uuid::new_v5(
                    &uuid::Uuid::NAMESPACE_URL,
                    format!("{input_message_id}:{tool_call_id}").as_bytes(),
                );
                let context = self.delivery_context();
                let client = everruns_integrations::slack_channel::bridge::direct_factory()
                    .map_err(everruns_integrations::slack_channel::web_api::provider_error)
                    .map_err(actions::action_error)?
                    .for_context(&context, &self.api_base)
                    .await
                    .map_err(everruns_integrations::slack_channel::web_api::provider_error)
                    .map_err(actions::action_error)?;
                let target =
                    everruns_integrations::slack_channel::bridge::target(&client, &context)
                        .map_err(everruns_integrations::slack_channel::web_api::provider_error)
                        .map_err(actions::action_error)?;
                let receipt = client
                    .post_markdown_single(
                        &target,
                        &everruns_integrations::slack_channel::markdown::neutralize_broadcasts(
                            &text,
                        ),
                        &Default::default(),
                        Some(&id.to_string()),
                    )
                    .await
                    .map_err(everruns_integrations::slack_channel::web_api::provider_error)
                    .map_err(actions::action_error)?;
                let timestamp = receipt
                    .messages
                    .into_iter()
                    .next()
                    .ok_or_else(|| SlackActionError::DeliveryUnknown { confirmed: vec![] })?
                    .id;
                if !files.is_empty() {
                    actions::share_message_files_scoped(&self.api_base, &context, files)
                        .await
                        .map_err(|error| {
                            actions::with_confirmed(
                                error,
                                vec![format!("{}:{timestamp}", self.channel)],
                            )
                        })?;
                }
                Ok(SlackActionOutcome::MessagePosted {
                    channel: self.channel.clone(),
                    timestamp,
                })
            }
        }
    }
}
impl crate::AgentBuilder {
    /// Add the standard Slack action tools, bound to an application-authorized conversation.
    pub fn slack_actions(mut self, actions: SlackActions) -> Self {
        self.native_services.slack = Some(actions);
        self = self.capability("slack");
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    #[tokio::test]
    async fn explicit_post_uses_stable_id_and_retains_unknown_acceptance() {
        let api = MockServer::start().await;
        Mock::given(path("/auth.test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"team_id":"T1","user_id":"B1"})),
            )
            .mount(&api)
            .await;
        Mock::given(path("/chat.postMessage"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&api)
            .await;
        let actions = SlackActions::new("fixture-token", "C1", "1.0").api_base(api.uri());
        let result = actions
            .invoke(SlackAction::PostMessage {
                input_message_id: "input-1".into(),
                tool_call_id: "call-1".into(),
                text: "Hello <!here>".into(),
                files: vec![],
            })
            .await;
        assert_eq!(
            result.unwrap_err(),
            SlackActionError::DeliveryUnknown { confirmed: vec![] }
        );
        let requests = api.received_requests().await.unwrap();
        let post = requests
            .iter()
            .find(|r| r.url.path() == "/chat.postMessage")
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        assert_eq!(body["channel"], "C1");
        assert_eq!(body["thread_ts"], "1.0");
        assert_eq!(
            body["client_msg_id"],
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, b"input-1:call-1").to_string()
        );
        assert!(!body.to_string().contains("<!here>"));
    }
}
