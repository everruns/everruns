//! A Slack channel driver: Events API in, `chat.postMessage` out.
//!
//! In: `app_mention` events, and plain messages unless the channel is
//! mention-only. Each Slack thread is one conversation; a message outside a
//! thread starts one. Out: replies are posted in that thread.
//!
//! Decisions:
//! - Requests are verified with Slack's `v0` signature and refused when the
//!   timestamp is more than five minutes off, so a captured request cannot be
//!   replayed.
//! - [`Slack::new`] always verifies. [`Slack::from_env`] verifies when
//!   `SLACK_SIGNING_SECRET` is set and otherwise accepts unsigned requests with
//!   a warning, for trying an app locally; a host that deploys it must require
//!   the secret, which [`ChannelDriver::secrets`] names.
//! - Without a bot token, replies are logged instead of posted.
//! - The binding thread is `channel:thread_ts`: Slack timestamps are unique
//!   per channel only.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use everruns_contracts::runtime::channel::{
    ChannelDeliveryAdapter, ChannelDriver, ChannelError, ChannelReplyMode, ChannelRequest,
    ChannelResponse, DeliveryContext, DeliveryResult, DeliveryTarget, ExternalActor, Inbound,
    InboundAttachment, InboundChannelEvent, InboundMessage, OutboundChannelMessage,
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tracing::{info, warn};

const BOT_TOKEN_ENV: &str = "SLACK_BOT_TOKEN";
const SIGNING_SECRET_ENV: &str = "SLACK_SIGNING_SECRET";
/// Slack's own replay window.
const MAX_CLOCK_SKEW_SECS: i64 = 5 * 60;

#[derive(Debug, Clone)]
enum Credential {
    /// Read from this environment variable at use, so a host can set it
    /// after the channel is built.
    Env(&'static str),
    Value(String),
}

impl Credential {
    fn value(&self) -> Option<String> {
        match self {
            Self::Env(name) => std::env::var(name).ok().filter(|value| !value.is_empty()),
            Self::Value(value) => Some(value.clone()),
        }
    }
}

/// The Slack driver.
#[derive(Debug, Clone)]
pub struct Slack {
    token: Credential,
    signing_secret: Credential,
    mention_only: bool,
    api_base: String,
    client: reqwest::Client,
}

impl Slack {
    /// A driver with a bot token (`xoxb-…`) and the app's signing secret.
    pub fn new(bot_token: impl Into<String>, signing_secret: impl Into<String>) -> Self {
        Self::with(
            Credential::Value(bot_token.into()),
            Credential::Value(signing_secret.into()),
        )
    }

    /// A driver reading `SLACK_BOT_TOKEN` and `SLACK_SIGNING_SECRET` from the
    /// environment when it needs them.
    pub fn from_env() -> Self {
        Self::with(
            Credential::Env(BOT_TOKEN_ENV),
            Credential::Env(SIGNING_SECRET_ENV),
        )
    }

    fn with(token: Credential, signing_secret: Credential) -> Self {
        Self {
            token,
            signing_secret,
            mention_only: false,
            api_base: "https://slack.com/api".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Answer only when the bot is @-mentioned.
    pub fn mention_only(mut self) -> Self {
        self.mention_only = true;
        self
    }

    /// Post to another Web API base, such as a test server.
    pub fn api_base(mut self, url: impl Into<String>) -> Self {
        self.api_base = url.into();
        self
    }

    fn verify(&self, request: &ChannelRequest) -> Result<(), ChannelError> {
        let Some(secret) = self.signing_secret.value() else {
            warn!("slack: accepting an unsigned request, {SIGNING_SECRET_ENV} is not set");
            return Ok(());
        };
        let unauthorized = |why: &str| ChannelError::Unauthorized(format!("slack: {why}"));
        let timestamp = request
            .header_value("x-slack-request-timestamp")
            .ok_or_else(|| unauthorized("missing x-slack-request-timestamp"))?;
        let sent: i64 = timestamp
            .parse()
            .map_err(|_| unauthorized("bad x-slack-request-timestamp"))?;
        if (unix_now() - sent).abs() > MAX_CLOCK_SKEW_SECS {
            return Err(unauthorized("request timestamp is too old"));
        }
        let signature = request
            .header_value("x-slack-signature")
            .and_then(|value| value.strip_prefix("v0="))
            .and_then(|hex_signature| hex::decode(hex_signature).ok())
            .ok_or_else(|| unauthorized("missing or malformed x-slack-signature"))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .map_err(|_| unauthorized("unusable signing secret"))?;
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(&request.body);
        mac.verify_slice(&signature)
            .map_err(|_| unauthorized("signature does not match"))
    }
}

#[async_trait]
impl ChannelDeliveryAdapter for Slack {
    fn platform(&self) -> &str {
        "slack"
    }

    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        let channel = &context.channel_id;
        if context.auth_token.is_empty() {
            info!(%channel, thread = %message.thread_ref, text = %message.text, "slack reply (no {BOT_TOKEN_ENV}, not posted)");
            return DeliveryResult::Ok;
        }
        let mut body = json!({ "channel": channel, "text": message.text });
        if !message.thread_ref.is_empty() {
            body["thread_ts"] = json!(message.thread_ref);
        }
        let response = match self
            .client
            .post(format!("{}/chat.postMessage", self.api_base))
            .bearer_auth(&context.auth_token)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return DeliveryResult::TransientError(error.to_string()),
        };
        let status = response.status();
        if status.is_server_error() || status.as_u16() == 429 {
            return DeliveryResult::TransientError(format!("chat.postMessage answered {status}"));
        }
        let body: Value = match response.json().await {
            Ok(body) => body,
            Err(error) => return DeliveryResult::TransientError(error.to_string()),
        };
        if body.get("ok").and_then(Value::as_bool) == Some(true) {
            DeliveryResult::Ok
        } else {
            let error = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            DeliveryResult::PermanentError(format!("chat.postMessage failed: {error}"))
        }
    }

    async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelDriver for Slack {
    fn secrets(&self) -> Vec<String> {
        [&self.token, &self.signing_secret]
            .into_iter()
            .filter_map(|credential| match credential {
                Credential::Env(name) => Some(name.to_string()),
                Credential::Value(_) => None,
            })
            .collect()
    }

    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        self.verify(request)?;
        let body = request.body_json()?;
        match body.get("type").and_then(Value::as_str) {
            Some("url_verification") => {
                let challenge = body.get("challenge").cloned().unwrap_or(Value::Null);
                return Ok(Inbound::Respond(ChannelResponse::ok(
                    json!({ "challenge": challenge }),
                )));
            }
            Some("event_callback") => {}
            _ => return Ok(Inbound::Ignore),
        }
        let Some(event) = body.get("event") else {
            return Ok(Inbound::Ignore);
        };
        let field = |name: &str| event.get(name).and_then(Value::as_str).unwrap_or("");
        let kind = field("type");
        let wanted = kind == "app_mention" || (!self.mention_only && kind == "message");
        // Bots (this one included) and edits, joins and other subtypes are
        // not messages for the agent.
        if !wanted || event.get("bot_id").is_some() || event.get("subtype").is_some() {
            return Ok(Inbound::Ignore);
        }
        let channel = field("channel");
        let ts = field("ts");
        if channel.is_empty() || ts.is_empty() {
            return Err(ChannelError::BadRequest(
                "slack event without channel or ts".into(),
            ));
        }
        let thread_ts = event.get("thread_ts").and_then(Value::as_str).unwrap_or(ts);
        let user = field("user");
        let attachments = event
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|file| {
                Some(InboundAttachment::FileDescription {
                    name: file.get("name")?.as_str()?.to_string(),
                    mime_type: file
                        .get("mimetype")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect();
        let routing_metadata = HashMap::from([
            ("channel_id".to_string(), channel.to_string()),
            ("user_id".to_string(), user.to_string()),
        ]);
        Ok(Inbound::Message(Box::new(InboundMessage {
            event: InboundChannelEvent {
                actor: ExternalActor {
                    actor_id: user.to_string(),
                    actor_name: None,
                    source: "slack".into(),
                    metadata: None,
                },
                text: strip_mentions(field("text")),
                attachments,
                dedup_key: body
                    .get("event_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                thread_ref: Some(format!("{channel}:{thread_ts}")),
                routing_metadata,
            },
            reply_to: DeliveryTarget::new(channel, thread_ts),
        })))
    }

    fn delivery_context(
        &self,
        target: &DeliveryTarget,
        reply_mode: ChannelReplyMode,
    ) -> DeliveryContext {
        target.context(self.token.value().unwrap_or_default(), reply_mode)
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Remove `<@U123>` mention tokens and collapse the whitespace around them.
fn strip_mentions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;
