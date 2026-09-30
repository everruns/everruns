//! Channels: where conversations come from and where replies go.
//!
//! A channel is served at `POST /v1/channels/{name}`. It turns a webhook into
//! a [`ChannelEvent`]; the host maps each conversation thread to one session
//! (so a Slack thread keeps its context), and after every turn hands the reply
//! back to [`Channel::deliver`].

use async_trait::async_trait;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::connection::Secret;

/// An inbound webhook request.
#[derive(Clone, Debug, Default)]
pub struct Inbound {
    /// Lower-cased header names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Inbound {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn json(&self) -> crate::Result<Value> {
        Ok(serde_json::from_slice(&self.body)?)
    }
}

/// What a webhook means.
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelEvent {
    /// A message for the agent. `thread` identifies the conversation (one
    /// session per thread); `reply_to` is where [`Channel::deliver`] posts.
    Message {
        thread: String,
        reply_to: String,
        text: String,
    },
    /// Answer the webhook directly (e.g. Slack's URL verification).
    Respond(Value),
    /// Acknowledge and do nothing.
    Ignore,
}

/// A conversation surface.
#[async_trait]
pub trait Channel: Send + Sync + 'static {
    /// Short kind for the manifest and agent card, e.g. `"slack"`.
    fn kind(&self) -> &'static str;

    /// Secrets this channel needs, for the manifest.
    fn secrets(&self) -> Vec<Secret> {
        Vec::new()
    }

    /// Interpret one webhook.
    async fn receive(&self, inbound: Inbound) -> crate::Result<ChannelEvent>;

    /// Post a reply. `target` is a `reply_to` from [`receive`](Self::receive)
    /// or the target of a [`DeliveryTarget`](crate::DeliveryTarget).
    async fn deliver(&self, target: &str, text: &str) -> crate::Result;
}

/// A generic JSON webhook: `{"thread": "...", "text": "..."}` in, replies
/// printed (or POSTed to `callback`, when the request carries one).
#[derive(Clone, Debug, Default)]
pub struct Webhook;

impl Webhook {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Channel for Webhook {
    fn kind(&self) -> &'static str {
        "webhook"
    }

    async fn receive(&self, inbound: Inbound) -> crate::Result<ChannelEvent> {
        let body = inbound.json()?;
        let Some(text) = body.get("text").and_then(Value::as_str) else {
            anyhow::bail!("webhook body needs a `text` string");
        };
        let thread = body
            .get("thread")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();
        let reply_to = body
            .get("callback")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("log:{thread}"));
        Ok(ChannelEvent::Message {
            thread,
            reply_to,
            text: text.to_string(),
        })
    }

    async fn deliver(&self, target: &str, text: &str) -> crate::Result {
        if target.starts_with("http://") || target.starts_with("https://") {
            reqwest::Client::new()
                .post(target)
                .json(&json!({ "text": text }))
                .send()
                .await?
                .error_for_status()?;
        } else {
            println!("  ↳ [webhook {target}] {text}");
        }
        Ok(())
    }
}

/// Slack via the Events API: `app_mention` (and, unless mention-only, direct
/// messages) in; `chat.postMessage` out, threaded.
///
/// Secrets: `SLACK_BOT_TOKEN` to post, `SLACK_SIGNING_SECRET` to verify
/// requests. Without the token, replies are printed instead of posted, which
/// is what `dev` wants.
#[derive(Clone, Debug)]
pub struct Slack {
    token: Secret,
    signing_secret: Secret,
    mention_only: bool,
    api_base: String,
}

impl Slack {
    /// Read credentials from the host-provided secrets.
    pub fn from_secrets() -> Self {
        Self {
            token: Secret::named("SLACK_BOT_TOKEN"),
            signing_secret: Secret::named("SLACK_SIGNING_SECRET"),
            mention_only: false,
            api_base: "https://slack.com/api".to_string(),
        }
    }

    /// Only respond when the bot is @-mentioned.
    pub fn mention_only(mut self) -> Self {
        self.mention_only = true;
        self
    }

    fn verify(&self, inbound: &Inbound) -> crate::Result {
        let Ok(secret) = self.signing_secret.value() else {
            // Unverified in dev by design; a hosted deploy always sets it.
            return Ok(());
        };
        let timestamp = inbound
            .header("x-slack-request-timestamp")
            .ok_or_else(|| anyhow::anyhow!("missing x-slack-request-timestamp"))?;
        let signature = inbound
            .header("x-slack-signature")
            .and_then(|value| value.strip_prefix("v0="))
            .ok_or_else(|| anyhow::anyhow!("missing x-slack-signature"))?;
        let expected = hex::decode(signature)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .map_err(|err| anyhow::anyhow!("signing secret: {err}"))?;
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(&inbound.body);
        mac.verify_slice(&expected)
            .map_err(|_| anyhow::anyhow!("Slack signature does not match"))
    }
}

#[async_trait]
impl Channel for Slack {
    fn kind(&self) -> &'static str {
        "slack"
    }

    fn secrets(&self) -> Vec<Secret> {
        vec![self.token, self.signing_secret]
    }

    async fn receive(&self, inbound: Inbound) -> crate::Result<ChannelEvent> {
        self.verify(&inbound)?;
        let body = inbound.json()?;
        match body.get("type").and_then(Value::as_str) {
            Some("url_verification") => {
                return Ok(ChannelEvent::Respond(
                    json!({ "challenge": body.get("challenge").cloned().unwrap_or(Value::Null) }),
                ));
            }
            Some("event_callback") => {}
            _ => return Ok(ChannelEvent::Ignore),
        }
        let Some(event) = body.get("event") else {
            return Ok(ChannelEvent::Ignore);
        };
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let from_bot = event.get("bot_id").is_some();
        let wanted = kind == "app_mention" || (!self.mention_only && kind == "message");
        if from_bot || !wanted {
            return Ok(ChannelEvent::Ignore);
        }
        let channel = event.get("channel").and_then(Value::as_str).unwrap_or("");
        let ts = event.get("ts").and_then(Value::as_str).unwrap_or("");
        let thread_ts = event.get("thread_ts").and_then(Value::as_str).unwrap_or(ts);
        let text = strip_mentions(event.get("text").and_then(Value::as_str).unwrap_or(""));
        Ok(ChannelEvent::Message {
            thread: format!("{channel}:{thread_ts}"),
            reply_to: format!("{channel}:{thread_ts}"),
            text,
        })
    }

    async fn deliver(&self, target: &str, text: &str) -> crate::Result {
        let (channel, thread_ts) = match target.split_once(':') {
            Some((channel, ts)) => (channel, Some(ts)),
            None => (target, None),
        };
        let Ok(token) = self.token.value() else {
            println!("  ↳ [slack {target}] (no SLACK_BOT_TOKEN, not posted) {text}");
            return Ok(());
        };
        let mut body = json!({ "channel": channel, "text": text });
        if let Some(ts) = thread_ts {
            body["thread_ts"] = json!(ts);
        }
        let response: Value = reqwest::Client::new()
            .post(format!("{}/chat.postMessage", self.api_base))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            anyhow::bail!("chat.postMessage failed: {response}");
        }
        Ok(())
    }
}

/// Remove `<@U123>` mention tokens and surrounding whitespace.
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
mod tests {
    use super::*;

    fn inbound(body: Value) -> Inbound {
        Inbound {
            headers: Vec::new(),
            body: serde_json::to_vec(&body).unwrap(),
        }
    }

    #[tokio::test]
    async fn slack_answers_url_verification() {
        let slack = Slack::from_secrets();
        let event = slack
            .receive(inbound(
                json!({"type": "url_verification", "challenge": "abc"}),
            ))
            .await
            .unwrap();
        assert_eq!(event, ChannelEvent::Respond(json!({"challenge": "abc"})));
    }

    #[tokio::test]
    async fn slack_mention_becomes_a_threaded_message() {
        let slack = Slack::from_secrets().mention_only();
        let event = slack
            .receive(inbound(json!({
                "type": "event_callback",
                "event": {"type": "app_mention", "channel": "C1", "ts": "1.2", "text": "<@U9> revenue?"}
            })))
            .await
            .unwrap();
        assert_eq!(
            event,
            ChannelEvent::Message {
                thread: "C1:1.2".into(),
                reply_to: "C1:1.2".into(),
                text: "revenue?".into()
            }
        );
    }

    #[tokio::test]
    async fn slack_mention_only_ignores_plain_messages_and_bots() {
        let slack = Slack::from_secrets().mention_only();
        let plain = json!({"type": "event_callback", "event": {"type": "message", "channel": "C1", "ts": "1", "text": "hi"}});
        assert_eq!(
            slack.receive(inbound(plain)).await.unwrap(),
            ChannelEvent::Ignore
        );
        let bot = json!({"type": "event_callback", "event": {"type": "app_mention", "bot_id": "B1", "channel": "C1", "ts": "1", "text": "hi"}});
        assert_eq!(
            slack.receive(inbound(bot)).await.unwrap(),
            ChannelEvent::Ignore
        );
    }

    #[tokio::test]
    async fn webhook_requires_text() {
        assert!(
            Webhook::new()
                .receive(inbound(json!({"thread": "t"})))
                .await
                .is_err()
        );
        let event = Webhook::new()
            .receive(inbound(json!({"thread": "t", "text": "hi"})))
            .await
            .unwrap();
        assert!(matches!(event, ChannelEvent::Message { ref thread, .. } if thread == "t"));
    }

    #[test]
    fn strip_mentions_keeps_the_rest() {
        assert_eq!(strip_mentions("<@U1> hello <@U2>there"), "hello there");
        assert_eq!(strip_mentions("no mentions"), "no mentions");
        assert_eq!(strip_mentions("broken <@U1"), "broken <@U1");
    }
}
