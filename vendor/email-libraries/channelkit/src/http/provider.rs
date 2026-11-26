//! Shared bounded provider HTTP operations. Tokens remain outside serialized models.
use super::{Request, ReqwestTransport, SharedTransport, validate_api_origin};
use crate::{
    Content, ConversationKey, DeliveryStatus, Error, MessageRef, Receipt, Result, WebhookRequest,
};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[async_trait]
pub trait TokenProvider: Send + Sync {
    /// Resolve credentials per operation; the host owns refresh and encrypted storage.
    async fn token(&self) -> Result<String>;
}
struct StaticToken(String);
#[async_trait]
impl TokenProvider for StaticToken {
    async fn token(&self) -> Result<String> {
        Ok(self.0.clone())
    }
}

#[derive(Clone, Copy)]
pub enum Authorization {
    Bearer,
    Bot,
    Raw,
}

/// No Debug implementation: provider credentials must never enter diagnostics.
#[derive(Clone)]
pub struct JsonClient {
    origin: url::Url,
    token: Arc<dyn TokenProvider>,
    authorization: Authorization,
    transport: SharedTransport,
    headers: BTreeMap<String, String>,
}
pub struct JsonResponse {
    pub value: Value,
    pub headers: BTreeMap<String, String>,
}
impl JsonClient {
    pub fn new(
        origin: &str,
        token: impl Into<String>,
        authorization: Authorization,
    ) -> Result<Self> {
        let token = token.into();
        validate_secret(&token)?;
        validate_api_origin(origin)?;
        Ok(Self {
            origin: url::Url::parse(origin).map_err(|_| Error::InvalidInput("API origin"))?,
            token: Arc::new(StaticToken(token)),
            authorization,
            transport: Arc::new(ReqwestTransport::new()?),
            headers: BTreeMap::new(),
        })
    }
    pub fn api_origin(mut self, origin: &str) -> Result<Self> {
        validate_api_origin(origin)?;
        self.origin = url::Url::parse(origin).map_err(|_| Error::InvalidInput("API origin"))?;
        Ok(self)
    }
    pub fn transport(mut self, transport: SharedTransport) -> Self {
        self.transport = transport;
        self
    }
    pub fn token_provider(mut self, token: Arc<dyn TokenProvider>) -> Self {
        self.token = token;
        self
    }
    pub fn header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }
    pub async fn call(
        &self,
        method: &str,
        path: &[&str],
        query: &[(&str, String)],
        body: Option<Value>,
        side_effect: bool,
    ) -> Result<JsonResponse> {
        let mut url = self.origin.clone();
        for segment in path {
            if segment.is_empty() || matches!(*segment, "." | "..") || segment.len() > 4096 {
                return Err(Error::InvalidInput("API path segment"));
            }
        }
        url.path_segments_mut()
            .map_err(|_| Error::InvalidInput("API origin"))?
            .pop_if_empty()
            .extend(path);
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        let token = self.token.token().await?;
        validate_secret(&token)?;
        let authorization = match self.authorization {
            Authorization::Bearer => format!("Bearer {token}"),
            Authorization::Bot => format!("Bot {token}"),
            Authorization::Raw => token,
        };
        let mut headers = self.headers.clone();
        headers.insert("authorization".into(), authorization);
        headers.insert("accept".into(), "application/json".into());
        headers.insert("user-agent".into(), "channelkit".into());
        let body = body
            .map(|value| {
                headers.insert("content-type".into(), "application/json".into());
                value.to_string().into_bytes()
            })
            .unwrap_or_default();
        let response = self
            .transport
            .execute(Request {
                method: method.into(),
                url: url.into(),
                headers,
                body,
                max_response_bytes: 1024 * 1024,
            })
            .await
            .map_err(|error| {
                if side_effect {
                    Error::DeliveryUnknown
                } else {
                    error
                }
            })?;
        if let Some(error) = response.rate_limit() {
            return Err(error);
        }
        if response.status >= 500 || response.status == 408 {
            return Err(if side_effect {
                Error::DeliveryUnknown
            } else {
                Error::Rejected {
                    code: format!("http_{}", response.status),
                }
            });
        }
        if !(200..300).contains(&response.status) {
            return Err(Error::Rejected {
                code: format!("http_{}", response.status),
            });
        }
        let value = if response.body.is_empty() {
            Value::Null
        } else {
            response.json().map_err(|error| {
                if side_effect {
                    Error::DeliveryUnknown
                } else {
                    error
                }
            })?
        };
        Ok(JsonResponse {
            value,
            headers: response.headers,
        })
    }
}

pub fn validate_secret(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 16 * 1024
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(Error::InvalidInput("provider credential"))
    } else {
        Ok(())
    }
}
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
pub fn identifier(value: &Value) -> Result<String> {
    let id = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|v| v.to_string()))
        .ok_or(Error::InvalidResponse)?;
    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return Err(Error::InvalidResponse);
    }
    Ok(id)
}
pub fn required<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or(Error::InvalidInput("provider field"))
}
pub fn receipt(target: &ConversationKey, value: &Value) -> Result<Receipt> {
    let id = identifier(value).map_err(|_| Error::DeliveryUnknown)?;
    Ok(Receipt {
        messages: vec![MessageRef {
            conversation: target.clone(),
            id,
        }],
        status: DeliveryStatus::Accepted,
    })
}
pub fn text(content: Content) -> Result<String> {
    content.validate()?;
    match content {
        Content::Text(text) | Content::Markdown(text) => Ok(text),
        Content::Card(card) if card.actions.is_empty() => {
            Ok(format!("## {}\n\n{}", card.title, card.body))
        }
        Content::Card(_) => Err(Error::Unsupported("interactive cards")),
    }
}
pub fn json_webhook(request: &WebhookRequest) -> Result<Value> {
    request.validate()?;
    if request.method != "POST" {
        return Err(Error::InvalidInput("webhook method"));
    }
    serde_json::from_slice(&request.body).map_err(|_| Error::InvalidInput("webhook JSON"))
}
pub fn verify_hmac_hex(
    request: &WebhookRequest,
    header: &str,
    secret: &str,
    prefix: &str,
) -> Result<()> {
    request.validate()?;
    if request.method != "POST" {
        return Err(Error::Authentication);
    }
    let supplied = request
        .header_value(header)
        .and_then(|v| v.strip_prefix(prefix))
        .filter(|v| v.len() == 64)
        .ok_or(Error::Authentication)?;
    let signature = hex::decode(supplied).map_err(|_| Error::Authentication)?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| Error::Authentication)?;
    mac.update(&request.body);
    mac.verify_slice(&signature)
        .map_err(|_| Error::Authentication)
}
pub fn decimal(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 32 || !value.bytes().all(|b| b.is_ascii_digit()) {
        Err(Error::InvalidInput("numeric provider identifier"))
    } else {
        Ok(())
    }
}
pub fn page(cursor: Option<&str>, limit: usize) -> Result<(String, String)> {
    let cursor = cursor.unwrap_or("1");
    decimal(cursor)?;
    if cursor.parse::<u32>().ok().is_none_or(|page| page == 0) || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput("history page"));
    }
    Ok((cursor.into(), limit.to_string()))
}
pub fn next_page(headers: &BTreeMap<String, String>, current: &str) -> Option<String> {
    headers
        .get("link")
        .filter(|link| link.contains("rel=\"next\""))
        .and_then(|_| {
            current
                .parse::<u64>()
                .ok()?
                .checked_add(1)
                .map(|v| v.to_string())
        })
}
