//! Provider reasoning artifacts.
//!
//! Reasoning is a provider-wire concept, like [`crate::execution_phase`]: the
//! driver types and core's `Message` both need it, so it lives in the provider
//! abstraction rather than being redefined on either side.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Readable reasoning text, in the form the provider actually exposes.
///
/// Providers differ in *what* they are willing to show, and collapsing that
/// difference loses the one thing a consumer needs to know: whether it is
/// looking at the model's own words or a curated gloss of them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReasoningText {
    /// Verbatim chain-of-thought exposed by the provider (Anthropic extended
    /// thinking, Gemini thought parts, Chat Completions `reasoning_content`).
    Plain { text: String },
    /// Provider-curated summary segments, not raw chain-of-thought (OpenAI
    /// Responses `summary_text`). Safe to display; never the model's own words.
    Summary { parts: Vec<String> },
    /// The provider withheld the content (Anthropic `redacted_thinking`). The
    /// artifact must still be replayed verbatim, so the part keeps its
    /// signature/encrypted payload while carrying no readable text.
    Redacted,
}

impl ReasoningText {
    /// Text safe to render on a reasoning channel, if any.
    pub fn display_text(&self) -> Option<String> {
        match self {
            Self::Plain { text } if !text.is_empty() => Some(text.clone()),
            Self::Summary { parts } if !parts.is_empty() => Some(parts.join("\n\n")),
            _ => None,
        }
    }

    /// Whether this is raw chain-of-thought rather than a curated summary.
    pub fn is_raw_chain_of_thought(&self) -> bool {
        matches!(self, Self::Plain { .. })
    }
}

/// One provider-issued reasoning artifact, ordered in `Message.content`
/// alongside text and tool calls.
///
/// Ordering is the point. Providers interleave reasoning with text and tool
/// calls, and every current provider requires its artifacts replayed in the
/// position it issued them: Anthropic verifies each thinking block against its
/// own `signature`, OpenAI keys reasoning items by the `item_id` it issued and
/// expects them adjacent to the item they precede, and Gemini binds a
/// `thoughtSignature` to a specific function call. A flattened per-message
/// field cannot express any of that, so this is a content part.
///
/// `signature` and `encrypted` are opaque provider artifacts. They are carried
/// verbatim and never interpreted, never rendered, and never published on an
/// API surface — see [`ReasoningContentPart::to_public`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasoningContentPart {
    /// Provider that produced this artifact (e.g. `anthropic`, `openai`).
    /// Replay is only valid against the provider that issued it.
    pub provider: String,

    /// Provider-assigned identifier, carried verbatim (e.g. OpenAI `rs_…`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,

    /// Provider signature over this specific block (Anthropic thinking
    /// signature, Gemini `thoughtSignature`). Opaque.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,

    /// Provider-encrypted reasoning context (OpenAI `encrypted_content`).
    /// Opaque.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<String>,

    /// Readable reasoning, when the provider exposes any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<ReasoningText>,

    /// Reasoning tokens attributed to this artifact, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,

    /// Id of the tool call this artifact is bound to, when the provider scopes
    /// it that way (Gemini attaches a thought signature to one function call).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_tool_call_id: Option<String>,
}

impl ReasoningContentPart {
    /// A reasoning part carrying only opaque replay state.
    pub fn opaque(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            item_id: None,
            signature: None,
            encrypted: None,
            text: None,
            tokens: None,
            bound_tool_call_id: None,
        }
    }

    pub fn with_item_id(mut self, item_id: impl Into<String>) -> Self {
        self.item_id = Some(item_id.into());
        self
    }

    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }

    pub fn with_encrypted(mut self, encrypted: impl Into<String>) -> Self {
        self.encrypted = Some(encrypted.into());
        self
    }

    pub fn with_text(mut self, text: ReasoningText) -> Self {
        self.text = Some(text);
        self
    }

    pub fn with_tokens(mut self, tokens: u32) -> Self {
        self.tokens = Some(tokens);
        self
    }

    pub fn with_bound_tool_call_id(mut self, tool_call_id: impl Into<String>) -> Self {
        self.bound_tool_call_id = Some(tool_call_id.into());
        self
    }

    /// Text safe to render on a reasoning channel, if any.
    pub fn display_text(&self) -> Option<String> {
        self.text.as_ref().and_then(ReasoningText::display_text)
    }

    /// Whether this part carries provider state that must be replayed.
    pub fn has_replay_state(&self) -> bool {
        self.signature.is_some() || self.encrypted.is_some() || self.item_id.is_some()
    }

    /// Projection safe to publish on an API surface: opaque provider artifacts
    /// removed, readable reasoning kept.
    ///
    /// `signature` and `encrypted` are replay state, not content. Publishing
    /// them leaks provider-internal material through an API and invites clients
    /// to round-trip values they cannot validate.
    pub fn to_public(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            item_id: self.item_id.clone(),
            signature: None,
            encrypted: None,
            text: self.text.clone(),
            tokens: self.tokens,
            bound_tool_call_id: self.bound_tool_call_id.clone(),
        }
    }
}
