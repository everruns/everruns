//! Slack-native action tools with an explicitly bound conversation. Stability: alpha.
use crate::capability::async_trait;
use everruns_contracts::slack_action::*;
use everruns_integrations::slack_channel::{
    actions,
    markdown::{markdown_block, neutralize_broadcasts},
    web_api::{SLACK_API_BASE, slack_api_call},
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
        }
    }
    /// Use a trusted Slack API root, normally only for testing.
    pub fn api_base(mut self, url: impl Into<String>) -> Self {
        self.api_base = url.into();
        self
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
        let body = slack_api_call(
            &self.api_base,
            &self.token,
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
                actions::add_reaction(&self.api_base, &self.token, &channel, &timestamp, &name)
                    .await
                    .map_err(actions::action_error)
            }
            SlackAction::UpdateMessage {
                channel,
                timestamp,
                text,
            } => {
                self.authorize_message(&channel, &timestamp).await?;
                actions::update_message(&self.api_base, &self.token, &channel, &timestamp, &text)
                    .await
            }
            SlackAction::LookupUser { user_id } => {
                self.authorize(&self.channel)?;
                actions::lookup_user(&self.api_base, &self.token, &user_id).await
            }
            SlackAction::UploadFile {
                channel,
                thread_ts,
                filename,
                content,
                initial_comment,
            } => {
                self.authorize(&channel)?;
                let expected = (!self.thread.is_empty()).then_some(self.thread.as_str());
                if thread_ts.as_deref() != expected {
                    return Err(SlackActionError::InvalidArgument(
                        "Slack upload targets a different thread".into(),
                    ));
                }
                actions::upload_file(
                    &self.api_base,
                    &self.token,
                    &channel,
                    thread_ts.as_deref(),
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
            } => {
                self.authorize(&self.channel)?;
                let id = uuid::Uuid::new_v5(
                    &uuid::Uuid::NAMESPACE_URL,
                    format!("{input_message_id}:{tool_call_id}").as_bytes(),
                );
                let mut body = serde_json::json!({"channel":self.channel,"text":neutralize_broadcasts(&text),"blocks":[markdown_block(&text)],"client_msg_id":id.to_string()});
                if !self.thread.is_empty() {
                    body["thread_ts"] = self.thread.clone().into();
                }
                let result = slack_api_call(&self.api_base, &self.token, "chat.postMessage", body)
                    .await
                    .map_err(actions::action_error)?;
                let timestamp = result["ts"].as_str().ok_or_else(|| {
                    SlackActionError::Transient("Slack did not return a message timestamp".into())
                })?;
                Ok(SlackActionOutcome::MessagePosted {
                    channel: self.channel.clone(),
                    timestamp: timestamp.into(),
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
