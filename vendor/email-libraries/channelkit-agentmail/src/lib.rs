//! AgentMail transport. The host owns credentials, acceptance, queues and policy.
//! Email retains its typed alternatives and envelope instead of flattening them
//! into chat text. Reads never authorize replies. Unknown sends are not retried.

#![doc = include_str!("../docs/agentmail.md")]

use base64::{Engine, engine::general_purpose::STANDARD};
use channelkit::http::{Request, SharedTransport};
use channelkit::{Error, Result, WebhookRequest};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeMap;

pub const PROVIDER: &str = "agentmail";
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EmailMessage {
    pub inbox_id: String,
    pub message_id: String,
    pub thread_id: String,
    #[serde(default, rename = "from")]
    pub from: String,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub cc: Vec<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub subject: String,
    #[serde(default, deserialize_with = "null_default")]
    pub preview: String,
    pub text: Option<String>,
    pub html: Option<String>,
    pub extracted_text: Option<String>,
    pub extracted_html: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub attachments: Vec<EmailAttachment>,
    #[serde(default, deserialize_with = "null_default")]
    pub headers: BTreeMap<String, String>,
    pub authentication_results: Option<AuthenticationResults>,
    pub agent_armor: Option<ArmorResult>,
}

fn null_default<'de, D: serde::Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> std::result::Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthenticationResults {
    pub spf: Option<String>,
    pub dkim: Option<String>,
    pub dmarc: Option<String>,
    pub spf_domain: Option<String>,
    pub dkim_domain: Option<String>,
    pub dmarc_domain: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArmorResult {
    pub status: String,
    pub verdict: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessagePage {
    pub messages: Vec<EmailMessage>,
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailAttachment {
    pub attachment_id: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailEvent {
    pub event_id: String,
    pub event_type: String,
    pub message: EmailMessage,
}

/// Verification is over the exact bytes. Caller supplies a trusted clock and
/// persists event_id plus (inbox_id, message_id) before acknowledging delivery.
pub fn verify_received(
    request: &WebhookRequest,
    secret: &str,
    inbox: &str,
    now: i64,
) -> Result<Option<EmailEvent>> {
    request.validate()?;
    if request.method != "POST" {
        return Err(Error::Authentication);
    }
    let id = request
        .header_value("svix-id")
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or(Error::Authentication)?;
    let timestamp = request
        .header_value("svix-timestamp")
        .ok_or(Error::Authentication)?;
    let seconds = timestamp
        .parse::<i64>()
        .map_err(|_| Error::Authentication)?;
    if now.abs_diff(seconds) > 300 {
        return Err(Error::Authentication);
    }
    let key = STANDARD
        .decode(secret.strip_prefix("whsec_").ok_or(Error::Authentication)?)
        .map_err(|_| Error::Authentication)?;
    if key.len() < 16 || key.len() > 256 {
        return Err(Error::Authentication);
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| Error::Authentication)?;
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(&request.body);
    let signatures = request
        .header_value("svix-signature")
        .filter(|s| s.len() <= 4096)
        .ok_or(Error::Authentication)?;
    let valid = signatures
        .split_whitespace()
        .filter_map(|s| s.strip_prefix("v1,"))
        .filter_map(|s| STANDARD.decode(s).ok())
        .any(|s| mac.clone().verify_slice(&s).is_ok());
    if !valid {
        return Err(Error::Authentication);
    }
    let value: serde_json::Value =
        serde_json::from_slice(&request.body).map_err(|_| Error::InvalidInput("webhook JSON"))?;
    // Delivery receipts and security notifications cannot become new prompts.
    if value.get("event_type").and_then(|v| v.as_str()) != Some("message.received") {
        return Ok(None);
    }
    let event: EmailEvent =
        serde_json::from_value(value).map_err(|_| Error::InvalidInput("email event"))?;
    if event.message.inbox_id != inbox {
        return Err(Error::Scope);
    }
    for id in [
        &event.event_id,
        &event.message.message_id,
        &event.message.thread_id,
    ] {
        identifier(id)?;
    }
    Ok(Some(event))
}

/// One composed message; HTML/text are alternatives, not two sends.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailSend {
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<String>,
    pub subject: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
}
impl EmailSend {
    pub fn validate(&self) -> Result<()> {
        if self.to.is_empty() || self.to.len() + self.cc.len() > 20 {
            return Err(Error::InvalidInput("recipients"));
        }
        for address in self.to.iter().chain(&self.cc) {
            mailbox(address)?;
        }
        if self.subject.len() > 998 || self.subject.chars().any(char::is_control) {
            return Err(Error::InvalidInput("subject"));
        }
        if self.text.is_empty()
            || self
                .text
                .len()
                .saturating_add(self.html.as_ref().map_or(0, String::len))
                > MAX_BODY_BYTES
        {
            return Err(Error::Capacity("email body"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailReceipt {
    pub message_id: String,
    pub thread_id: String,
}

/// Credentials intentionally have no Debug/Serialize implementation. The host
/// constructs a client per operation so account revocation/rotation takes effect.
pub struct AgentMail {
    key: String,
    inbox: String,
    transport: SharedTransport,
}
impl AgentMail {
    pub fn new(key: String, inbox: String, transport: SharedTransport) -> Result<Self> {
        if key.is_empty()
            || key.len() > 512
            || key.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(Error::InvalidInput("API key"));
        }
        mailbox(&inbox)?;
        Ok(Self {
            key,
            inbox,
            transport,
        })
    }
    pub async fn verify_inbox(&self) -> Result<()> {
        let value = self.call("GET", &[], None, None, &[]).await?;
        if value.get("inbox_id").and_then(|v| v.as_str()) != Some(self.inbox.as_str()) {
            return Err(Error::Scope);
        }
        Ok(())
    }
    pub async fn list_received(&self, after: &str, page: Option<&str>) -> Result<MessagePage> {
        // The provider's datetime validator requires the UTC `Z` spelling,
        // rejecting the equivalent RFC 3339 `+00:00` emitted by many clocks.
        let after = after
            .strip_suffix("+00:00")
            .map(|utc| format!("{utc}Z"))
            .unwrap_or_else(|| after.to_owned());
        let mut query = vec![
            ("limit", "100"),
            ("ascending", "true"),
            ("labels", "received"),
            ("after", after.as_str()),
        ];
        if let Some(page) = page {
            if page.len() > 4096 {
                return Err(Error::InvalidInput("page token"));
            }
            query.push(("page_token", page));
        }
        let value = self.call("GET", &["messages"], None, None, &query).await?;
        let page: MessagePage =
            serde_json::from_value(value).map_err(|_| Error::InvalidResponse)?;
        if page.messages.len() > 100
            || page.messages.iter().any(|m| {
                m.inbox_id != self.inbox
                    || identifier(&m.message_id).is_err()
                    || identifier(&m.thread_id).is_err()
            })
        {
            return Err(Error::Scope);
        }
        Ok(page)
    }
    pub async fn get_message(&self, message: &str) -> Result<EmailMessage> {
        identifier(message)?;
        let value = self
            .call("GET", &["messages", message], None, None, &[])
            .await?;
        let result: EmailMessage =
            serde_json::from_value(value).map_err(|_| Error::InvalidResponse)?;
        if result.inbox_id != self.inbox || result.message_id != message {
            return Err(Error::Scope);
        }
        Ok(result)
    }
    pub async fn send(
        &self,
        message: &EmailSend,
        reply_to: Option<&str>,
        idempotency_key: &str,
    ) -> Result<EmailReceipt> {
        message.validate()?;
        if idempotency_key.is_empty()
            || idempotency_key.len() > 256
            || !idempotency_key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        {
            return Err(Error::InvalidInput("idempotency key"));
        }
        let mut path = vec!["messages"];
        if let Some(id) = reply_to {
            identifier(id)?;
            path.extend([id, "reply"]);
        } else {
            path.push("send");
        }
        let mut body = serde_json::to_value(message).map_err(|_| Error::InvalidInput("email"))?;
        body["headers"] =
            serde_json::json!({"Auto-Submitted":"auto-replied","X-Auto-Response-Suppress":"All"});
        if reply_to.is_some() {
            body.as_object_mut()
                .expect("email object")
                .remove("subject");
            body["reply_all"] = false.into();
        }
        let value = self
            .call("POST", &path, Some(body), Some(idempotency_key), &[])
            .await?;
        let receipt: EmailReceipt =
            serde_json::from_value(value).map_err(|_| Error::DeliveryUnknown)?;
        if identifier(&receipt.message_id).is_err() || identifier(&receipt.thread_id).is_err() {
            return Err(Error::DeliveryUnknown);
        }
        Ok(receipt)
    }
    async fn call(
        &self,
        method: &str,
        suffix: &[&str],
        body: Option<serde_json::Value>,
        key: Option<&str>,
        query: &[(&str, &str)],
    ) -> Result<serde_json::Value> {
        let mut url =
            url::Url::parse("https://api.agentmail.to/v0/inboxes/").expect("constant URL");
        url.path_segments_mut()
            .expect("base URL")
            .pop_if_empty()
            .push(&self.inbox)
            .extend(suffix);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        let mut headers = BTreeMap::from([
            ("authorization".into(), format!("Bearer {}", self.key)),
            ("content-type".into(), "application/json".into()),
        ]);
        if let Some(key) = key {
            headers.insert("idempotency-key".into(), key.into());
        }
        let sending = method == "POST";
        let response = self
            .transport
            .execute(Request {
                method: method.into(),
                url: url.into(),
                headers,
                body: body.map_or_else(Vec::new, |v| v.to_string().into_bytes()),
                max_response_bytes: MAX_BODY_BYTES,
            })
            .await
            .map_err(|e| if sending { Error::DeliveryUnknown } else { e })?;
        if let Some(error) = response.rate_limit() {
            return Err(error);
        }
        if response.status >= 500 || response.status == 408 {
            return Err(if sending {
                Error::DeliveryUnknown
            } else {
                Error::Transport
            });
        }
        if !(200..300).contains(&response.status) {
            return Err(Error::Rejected {
                code: format!("http_{}", response.status),
            });
        }
        response
            .json()
            .map_err(|e| if sending { Error::DeliveryUnknown } else { e })
    }
}

fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 512
        || value.chars().any(char::is_control)
        || matches!(value, "." | "..")
    {
        Err(Error::InvalidInput("identifier"))
    } else {
        Ok(())
    }
}
pub fn mailbox(value: &str) -> Result<()> {
    let Some((local, domain)) = value.split_once('@') else {
        return Err(Error::InvalidInput("mailbox"));
    };
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || value.len() > 254
        || value.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '/' | '\\' | '?' | '#')
        })
    {
        return Err(Error::InvalidInput("mailbox"));
    }
    Ok(())
}
/// Only for a provider-confirmed delivery recipient, never sender identity or
/// authorization. Aliases share the base inbox's quotas and conversation policy.
pub fn recipient_tag<'a>(base: &str, delivered_to: &'a str) -> Option<&'a str> {
    let (local, domain) = base.split_once('@')?;
    let (target, target_domain) = delivered_to.split_once('@')?;
    if !domain.eq_ignore_ascii_case(target_domain) {
        return None;
    }
    target
        .strip_prefix(local)?
        .strip_prefix('+')
        .filter(|tag| !tag.is_empty())
}
