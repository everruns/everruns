use super::{DispatchError, OPENROUTER_CHAT};
use crate::rt::Duration;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const CONTROL_HEADERS: [(&str, &str); 2] = [
    ("content-type", "application/json"),
    ("x-openrouter-metadata", "enabled"),
];

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextRole {
    System,
    User,
    Assistant,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextMessage {
    pub role: TextRole,
    pub content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextRequest {
    pub messages: Vec<TextMessage>,
    pub max_output_tokens: u32,
}

/// Host configuration, not proof of supplier qualification. No profile ships
/// enabled. Activation must independently verify the exact model's context,
/// all-in pricing/fees, output semantics and serving-account defaults.
pub struct TextDispatchPolicy {
    pub profile_id: String,
    pub model: String,
    /// Reserve the supplier's entire qualified context window, not a tokenizer
    /// estimate of the local text. Request byte limits only bound local work.
    pub context_window_tokens: u32,
    pub max_output_tokens: u32,
    /// Whole USD per million tokens keeps wire caps and integer arithmetic exact.
    pub prompt_usd_per_million: u32,
    pub completion_usd_per_million: u32,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub timeout: Duration,
}

/// Immutable exact wire request plus its conservative upstream envelope. The
/// host adds its own margin/rates when reserving customer funds.
pub struct DispatchPlan {
    body: Vec<u8>,
    hash: [u8; 32],
    maximum_upstream_nanos: u64,
    pub(super) model: String,
    pub(super) max_input_tokens: u32,
    pub(super) max_output_tokens: u32,
}
impl DispatchPlan {
    pub fn body(&self) -> &[u8] {
        &self.body
    }
    pub fn request_hash(&self) -> [u8; 32] {
        self.hash
    }
    pub fn maximum_upstream_nanos(&self) -> u64 {
        self.maximum_upstream_nanos
    }
    pub fn maximum_input_tokens(&self) -> u32 {
        self.max_input_tokens
    }
    pub fn maximum_output_tokens(&self) -> u32 {
        self.max_output_tokens
    }
}
impl std::fmt::Debug for DispatchPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchPlan")
            .field("body_bytes", &self.body.len())
            .field("maximum_upstream_nanos", &self.maximum_upstream_nanos)
            .finish_non_exhaustive()
    }
}
impl TextDispatchPolicy {
    pub fn compile(&self, request: &TextRequest) -> Result<DispatchPlan, DispatchError> {
        let valid_model = self.model.starts_with("openai/")
            && self.model.len() > 7
            && self.model.len() <= 128
            && self
                .model
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/-_.".contains(&c))
            && self.model.bytes().filter(|c| *c == b'/').count() == 1;
        if !valid_model
            || self.profile_id.is_empty()
            || self.profile_id.len() > 128
            || self.context_window_tokens == 0
            || self.context_window_tokens > 1_048_576
            || self.max_output_tokens == 0
            || self.max_output_tokens > self.context_window_tokens
            || request.max_output_tokens < 16
            || request.max_output_tokens > self.max_output_tokens
            || self.max_request_bytes == 0
            || self.max_request_bytes > 1_048_576
            || self.max_response_bytes == 0
            || self.max_response_bytes > 1_048_576
            || self.timeout.is_zero()
            || self.timeout > Duration::from_secs(60)
            || request.messages.is_empty()
            || request.messages.len() > 128
        {
            return Err(DispatchError::Unsupported);
        }
        let mut text_bytes = 0usize;
        for message in &request.messages {
            text_bytes = text_bytes
                .checked_add(message.content.len())
                .ok_or(DispatchError::Unsupported)?;
            if text_bytes > self.max_request_bytes {
                return Err(DispatchError::Unsupported);
            }
        }
        let maximum_upstream_nanos = u64::from(self.context_window_tokens)
            .checked_mul(u64::from(self.prompt_usd_per_million))
            .and_then(|input| {
                u64::from(request.max_output_tokens)
                    .checked_mul(u64::from(self.completion_usd_per_million))
                    .and_then(|output| input.checked_add(output))
            })
            .and_then(|price| price.checked_mul(1000))
            .filter(|price| *price > 0 && *price <= i64::MAX as u64)
            .ok_or(DispatchError::Unsupported)?;
        // The narrow shape excludes every optional charge-bearing extension.
        // Parameter support and route/price caps must be enforced by the supplier;
        // never silently remove them to recover availability.
        let body = serde_json::to_vec(&serde_json::json!({
            "model": self.model, "messages": request.messages,
            "max_completion_tokens": request.max_output_tokens, "n": 1, "stream": false,
            "tools": [], "tool_choice": "none", "plugins": [], "transforms": [],
            "modalities": ["text"], "reasoning": {"enabled": false},
            "service_tier": "default", "prompt_cache": {"mode": "explicit", "ttl": "30m"},
            "provider": {"only": ["openai"], "allow_fallbacks": false,
                "require_parameters": true, "max_price": {
                    "prompt": self.prompt_usd_per_million,
                    "completion": self.completion_usd_per_million,
                    "request": 0, "image": 0}}
        }))
        .map_err(|_| DispatchError::Unsupported)?;
        if body.len() > self.max_request_bytes {
            return Err(DispatchError::Unsupported);
        }
        let terms = serde_json::to_vec(&(
            "openrouter-text-dispatch-v1",
            "POST",
            OPENROUTER_CHAT,
            CONTROL_HEADERS,
            &self.profile_id,
            self.context_window_tokens,
            maximum_upstream_nanos,
            self.max_response_bytes,
            self.timeout.as_nanos(),
            &body,
        ))
        .map_err(|_| DispatchError::Unsupported)?;
        Ok(DispatchPlan {
            hash: Sha256::digest(terms).into(),
            body,
            maximum_upstream_nanos,
            model: self.model.clone(),
            max_input_tokens: self.context_window_tokens,
            max_output_tokens: request.max_output_tokens,
        })
    }
}
