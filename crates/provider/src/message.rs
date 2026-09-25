//! The provider-agnostic message a driver sends, and what it is made of.
//!
//! [`Message`] is the one message shape every driver converts from, whatever
//! its vendor's wire format. Callers holding OpenAI chat JSON convert with
//! [`openai_wire`](crate::openai_wire) rather than building these by hand.

use crate::tool_types::ToolCall;
use serde::{Deserialize, Serialize};

/// Provider-native assistant content retained for lossless replay.
///
/// Drivers use this only when a provider requires its response content to be
/// sent back without reconstruction. The portable text, reasoning, and tool
/// call fields remain the fallback for other providers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProviderOpaqueContent {
    /// Provider that owns and can replay this content.
    pub provider: String,
    /// Original provider-native assistant content.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub content: serde_json::Value,
}

impl ProviderOpaqueContent {
    pub fn new(provider: impl Into<String>, content: serde_json::Value) -> Self {
        Self {
            provider: provider.into(),
            content,
        }
    }
}

/// Internal marker that asks a capable driver to expire a system reminder at
/// the next user message. Drivers must remove it before serialization.
pub const TURN_SCOPED_SYSTEM_MARKER: &str = "\u{0}everruns:turn-scoped-system\u{0}";

/// Message format for LLM calls (provider-agnostic): the request-shaped view a
/// driver turns into provider wire format. Distinct from the lossless stored
/// `everruns_core::message::RuntimeMessage`, which `llm_conversions` maps here.
#[derive(Debug, Clone)]
pub struct Message {
    /// Provider-native call identities, retained alongside portable fallbacks.
    pub native_tool_calls: Vec<crate::native_async::NativeToolCall>,
    pub role: MessageRole,
    pub content: MessageContent,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    /// Execution phase for assistant messages.
    /// Helps models distinguish between intermediate working commentary (`Commentary`)
    /// and completed answers (`FinalAnswer`) in multi-step tool-calling flows.
    /// Only set on assistant messages. Must be preserved when replaying conversation history.
    pub phase: Option<crate::execution_phase::ExecutionPhase>,
    /// Provider reasoning artifacts for this assistant turn, in emission order.
    ///
    /// Drivers replay these verbatim in the position the provider issued them:
    /// each keeps its own signature, id and encrypted payload, so interleaved
    /// thinking and per-call thought signatures survive a round trip. Empty for
    /// messages without reasoning.
    pub reasoning: Vec<crate::reasoning::ReasoningContentPart>,
    /// Astra effort transition immediately before this message. Other protocols
    /// ignore it; it is never rendered as conversation text.
    pub configuration_update: Option<crate::model::ReasoningEffort>,
}

impl Message {
    /// Create a message with text content
    pub fn text(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            native_tool_calls: Vec::new(),
            role,
            content: MessageContent::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
        }
    }

    /// Create a message with content parts (text, images, audio)
    pub fn parts(role: MessageRole, parts: Vec<LlmContentPart>) -> Self {
        Self {
            native_tool_calls: Vec::new(),
            role,
            content: MessageContent::Parts(parts),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
        }
    }

    /// Get content as plain text string (for simple cases)
    pub fn content_as_text(&self) -> String {
        self.content.to_text()
    }

    /// Prepend a prefix to the first text content.
    ///
    /// Used by ReasonAtom to inject external actor identity (e.g. `"[Alice] "`)
    /// into user messages from external channels.
    pub fn prepend_text_prefix(&mut self, prefix: &str) {
        match &mut self.content {
            MessageContent::Text(text) => {
                *text = format!("{}{}", prefix, text);
            }
            MessageContent::Parts(parts) => {
                for part in parts.iter_mut() {
                    if let LlmContentPart::Text { text } = part {
                        *text = format!("{}{}", prefix, text);
                        return;
                    }
                }
                // No text part found — prepend one
                parts.insert(
                    0,
                    LlmContentPart::Text {
                        text: prefix.to_string(),
                    },
                );
            }
        }
    }

    /// Mark this system message as scoped to the current model turn.
    pub fn mark_turn_scoped_system(&mut self) {
        debug_assert_eq!(self.role, MessageRole::System);
        self.prepend_text_prefix(TURN_SCOPED_SYSTEM_MARKER);
    }
}

/// Fold every `System`-role message into a single string, joined in order with
/// blank lines.
///
/// Multiple system messages legitimately occur in one request: the agent system
/// prompt plus, e.g., `infinity_context`'s hidden-history notice or
/// `compaction`'s `[CONVERSATION_SUMMARY]`. Drivers that map the system role into
/// a dedicated top-level field (Anthropic `system`, Gemini `system_instruction`,
/// OpenResponses `instructions`) must accumulate rather than overwrite — otherwise
/// the real agent system prompt is silently dropped and only the last notice
/// survives. Returns `None` when there are no system messages.
pub fn fold_system_messages(messages: &[Message]) -> Option<String> {
    let mut system: Option<String> = None;
    for msg in messages {
        if msg.role == MessageRole::System {
            let text = msg.content.to_text();
            system = Some(match system.take() {
                Some(existing) if !existing.is_empty() => format!("{existing}\n\n{text}"),
                _ => text,
            });
        }
    }
    system
}

/// Message content - either a simple string or array of content parts
#[derive(Debug, Clone)]
pub enum MessageContent {
    /// Simple text content
    Text(String),
    /// Array of content parts (text, images, audio)
    Parts(Vec<LlmContentPart>),
}

impl MessageContent {
    /// Convert to plain text (concatenates text parts, ignores media)
    pub fn to_text(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    LlmContentPart::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    /// Check if content is simple text
    pub fn is_text(&self) -> bool {
        matches!(self, MessageContent::Text(_))
    }

    /// Check if content has multiple parts
    pub fn is_parts(&self) -> bool {
        matches!(self, MessageContent::Parts(_))
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        MessageContent::Text(s)
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        MessageContent::Text(s.to_string())
    }
}

/// A single content part within a message
///
/// `#[non_exhaustive]` for the same reason as [`LlmStreamEvent`]: new content
/// kinds are additive and must not break downstream `match`es.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LlmContentPart {
    /// Text content
    Text { text: String },
    /// Image content (base64 data URL or HTTP URL)
    Image { url: String },
    /// Audio content (base64 data URL)
    Audio { url: String },
    /// File content, e.g. a PDF document (base64 data URL or file URL)
    File {
        url: String,
        filename: Option<String>,
    },
    /// Provider-native assistant content used only by the issuing provider.
    ProviderOpaque(ProviderOpaqueContent),
}

impl LlmContentPart {
    /// Create a text content part
    pub fn text(text: impl Into<String>) -> Self {
        LlmContentPart::Text { text: text.into() }
    }

    /// Create an image content part from URL (can be data URL or HTTP URL)
    pub fn image(url: impl Into<String>) -> Self {
        LlmContentPart::Image { url: url.into() }
    }

    /// Create an audio content part from URL (typically a data URL)
    pub fn audio(url: impl Into<String>) -> Self {
        LlmContentPart::Audio { url: url.into() }
    }

    /// Create a file content part from URL (typically a data URL)
    pub fn file(url: impl Into<String>, filename: Option<String>) -> Self {
        LlmContentPart::File {
            url: url.into(),
            filename,
        }
    }
}

/// Message role for LLM calls
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

// The names these types had before 0.31. Aliases rather than removals so an
// embedder whose own code defines a `Message` upgrades with a warning instead
// of a compile error.
#[deprecated(since = "0.32.0", note = "renamed to `Message`")]
pub type LlmMessage = Message;
#[deprecated(since = "0.32.0", note = "renamed to `MessageContent`")]
pub type LlmMessageContent = MessageContent;
#[deprecated(since = "0.32.0", note = "renamed to `MessageRole`")]
pub type LlmMessageRole = MessageRole;
