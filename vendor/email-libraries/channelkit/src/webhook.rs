use crate::{Error, Event, Result};
use std::collections::BTreeMap;

pub const MAX_WEBHOOK_BYTES: usize = 1024 * 1024;
#[derive(Clone)]
pub struct WebhookRequest {
    pub method: String,
    pub headers: BTreeMap<String, String>,
    pub query: BTreeMap<String, String>,
    pub body: Vec<u8>,
}
impl WebhookRequest {
    pub fn post(body: impl Into<Vec<u8>>) -> Self {
        Self {
            method: "POST".into(),
            headers: BTreeMap::new(),
            query: BTreeMap::new(),
            body: body.into(),
        }
    }
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.insert(name.to_ascii_lowercase(), value.into());
        self
    }
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn validate(&self) -> Result<()> {
        if self.body.len() > MAX_WEBHOOK_BYTES {
            Err(Error::Capacity("webhook body"))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct WebhookResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}
impl WebhookResponse {
    pub fn json(value: serde_json::Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: value.to_string().into_bytes(),
        }
    }
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "text/plain",
            body: value.into().into_bytes(),
        }
    }
}
/// Parsing does not start handlers or claim that the host accepted durable work.
#[derive(Clone, Debug)]
pub struct Ingress {
    pub response: WebhookResponse,
    pub events: Vec<Event>,
}
