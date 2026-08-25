// Message types
//
// Message is a DB-agnostic message type that represents
// a single message in the conversation history.
//
// Content is stored as Vec<ContentPart> for unified representation
// across storage and runtime layers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::{ImageId, MessageId, ModelId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use everruns_provider::execution_phase::{ExecutionPhase, PhaseSource};
use everruns_provider::reasoning::ReasoningContentPart;
/// Message role in the conversation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
// Published as `RuntimeMessageRole`. The REST API exposes only `user` and
// `agent` (`api::messages::MessageRole`); publishing this four-variant runtime
// enum under the plain name made clients model roles the API never returns.
#[cfg_attr(feature = "openapi", schema(as = RuntimeMessageRole))]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// System message (instructions)
    System,
    /// User message
    User,
    /// Agent response (may contain tool calls in content)
    Agent,
    /// Tool execution result
    ToolResult,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::System => write!(f, "system"),
            MessageRole::User => write!(f, "user"),
            MessageRole::Agent => write!(f, "agent"),
            MessageRole::ToolResult => write!(f, "tool_result"),
        }
    }
}

impl From<&str> for MessageRole {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "system" => MessageRole::System,
            "user" => MessageRole::User,
            // Accept both "agent" and legacy "assistant"
            "agent" | "assistant" => MessageRole::Agent,
            "tool_result" => MessageRole::ToolResult,
            _ => MessageRole::User,
        }
    }
}

// ============================================
// External Actor (channel-agnostic user identity)
// ============================================

/// External actor identity for messages originating from external channels
/// (Slack, Discord, Teams, etc.).
///
/// Channel adapters populate this to identify the sender without coupling
/// core logic to any specific channel. The ReasonAtom uses this to prefix
/// user messages so the LLM knows who is speaking.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExternalActor {
    /// Opaque actor identifier from the source channel (e.g. Slack user ID "U0123456789")
    pub actor_id: String,
    /// Resolved display name (e.g. "Alice"). Falls back to actor_id if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_name: Option<String>,
    /// Source channel identifier (e.g. "slack", "discord")
    pub source: String,
    /// Channel-specific metadata (e.g. team_id, channel_id)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, String>>,
}

impl ExternalActor {
    /// Human-readable label: display name if available, otherwise actor_id.
    pub fn display_label(&self) -> &str {
        self.actor_name.as_deref().unwrap_or(&self.actor_id)
    }
}

// ============================================
// Controls (runtime options for message processing)
// ============================================

/// Reasoning configuration for the model
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasoningConfig {
    /// Effort level for reasoning.
    ///
    /// Typed rather than free-form: the effort taxonomy is closed, and each
    /// driver previously re-parsed the string with its own case handling, which
    /// let `minimal` silently mean "no reasoning" on budget-based models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<everruns_provider::model::ReasoningEffort>,
}

/// Runtime controls for message processing
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Controls {
    /// Model ID to use for this message (format: model_{32-hex}).
    /// Overrides session and agent model settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub model_id: Option<ModelId>,

    /// Locale override for this message turn (BCP 47, e.g. `uk-UA`).
    /// Overrides the session locale for backend-authored strings and prompts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,

    /// Reasoning configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,

    /// Speed (service tier) for this message turn: "flex", "default", or
    /// "priority". Only sent to providers whose model profile advertises a
    /// speed config (OpenAI `service_tier`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,

    /// Verbosity for this message turn: "low", "medium", or "high". Only sent
    /// to providers whose model profile advertises a verbosity config (OpenAI
    /// `verbosity`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,

    /// Error disclosure override for this turn: "generic", "standard", or
    /// "detailed". Clamped to at most the mode allowed by the agent's
    /// `error_disclosure` capability (capability absent => "standard"), so a
    /// client can narrow but never widen disclosure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_disclosure: Option<String>,

    /// Generic client hints — arbitrary key-value pairs declared by the client.
    /// Session-level defaults are set at session creation; per-message values
    /// override session hints key-by-key (shallow merge).
    ///
    /// Examples: `{"setup_connection": true, "rich_media": true}`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub hints: Option<std::collections::HashMap<String, serde_json::Value>>,
}

impl Controls {
    /// Resolve effective hints by shallow-merging session-level defaults with
    /// per-message overrides. Per-message hints take precedence key-by-key.
    pub fn resolve_hints(
        session_hints: Option<&std::collections::HashMap<String, serde_json::Value>>,
        message_hints: Option<&std::collections::HashMap<String, serde_json::Value>>,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        match (session_hints, message_hints) {
            (None, None) => std::collections::HashMap::new(),
            (Some(s), None) => s.clone(),
            (None, Some(m)) => m.clone(),
            (Some(s), Some(m)) => {
                let mut merged = s.clone();
                merged.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
                merged
            }
        }
    }
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
// Published as `RuntimeMessage`: this is the canonical runtime/event message,
// distinct from the REST resource in `api::messages::Message` (which adds
// `session_id` and `sequence`). Both previously claimed the name `Message` in
// one OpenAPI document, so generated clients saw whichever won.
#[cfg_attr(feature = "openapi", schema(as = RuntimeMessage))]
pub struct Message {
    /// Unique message ID (format: message_{32-hex})
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_01933b5a00007000800000000000001"))]
    pub id: MessageId,

    /// Message role
    pub role: MessageRole,

    /// Message content as array of content parts (text, images, tool calls, tool results)
    pub content: Vec<ContentPart>,

    /// Execution phase for this message.
    ///
    /// Helps LLMs distinguish between intermediate working commentary and completed
    /// answers in multi-step tool-calling flows. Only set on agent (assistant) messages.
    /// Providers with native phase support (OpenAI GPT-5.x) send this value in the API
    /// request; others derive it from state but don't send it to the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ExecutionPhase>,

    /// Whether [`Self::phase`] was reported by the provider or inferred from
    /// tool-call presence. A derived phase carries no information beyond
    /// "this message called tools", so consumers that need a real
    /// classification must be able to tell the two apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_source: Option<PhaseSource>,

    /// Runtime controls (model, reasoning, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,

    /// Message-level metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub metadata: Option<std::collections::HashMap<String, serde_json::Value>>,

    /// External actor identity (for messages from external channels like Slack)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_actor: Option<ExternalActor>,

    /// Timestamp when the message was created
    pub created_at: DateTime<Utc>,
}

// ============================================
// Content Type Enum
// ============================================

/// Content type discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Text,
    Image,
    ImageFile,
    ToolCall,
    ToolResult,
    Reasoning,
}

impl std::fmt::Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentType::Text => write!(f, "text"),
            ContentType::Image => write!(f, "image"),
            ContentType::ImageFile => write!(f, "image_file"),
            ContentType::ToolCall => write!(f, "tool_call"),
            ContentType::ToolResult => write!(f, "tool_result"),
            ContentType::Reasoning => write!(f, "reasoning"),
        }
    }
}

impl From<&str> for ContentType {
    fn from(s: &str) -> Self {
        match s {
            "image" => ContentType::Image,
            "image_file" => ContentType::ImageFile,
            "tool_call" => ContentType::ToolCall,
            "tool_result" => ContentType::ToolResult,
            "reasoning" => ContentType::Reasoning,
            _ => ContentType::Text,
        }
    }
}

// ============================================
// Content Part Structs
// ============================================

/// Text content part
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TextContentPart {
    pub text: String,
    /// Claim-level citations attached to spans of `text`.
    ///
    /// The narrow render contract shared by all citation capabilities (see
    /// `knowledge/runtime-resources/citations.md`). Empty for non-cited text, so the wire shape of
    /// existing messages is unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<TextAnnotation>,
}

impl TextContentPart {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            annotations: Vec::new(),
        }
    }

    /// Attach citation annotations, replacing any existing ones.
    pub fn with_annotations(mut self, annotations: Vec<TextAnnotation>) -> Self {
        self.annotations = annotations;
        self
    }
}

/// A claim-level citation attached to a span of generated text.
///
/// The single shared type across every citation capability: a text span linked
/// to a source. Producers agree only on this render contract — each capability
/// keeps its own richer representation (e.g. `KnowledgeIndexCitation`) and maps
/// into this envelope at emit time. See `knowledge/runtime-resources/citations.md`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TextAnnotation {
    /// 0-indexed start char offset into the enclosing `TextContentPart.text`.
    #[cfg_attr(feature = "openapi", schema(example = 0))]
    pub start: usize,
    /// Exclusive end char offset.
    #[cfg_attr(feature = "openapi", schema(example = 19))]
    pub end: usize,
    /// Capability id that produced this annotation (e.g. `citation_retrieval`).
    /// Lets the UI and evals attribute and filter each citation by feed.
    #[cfg_attr(feature = "openapi", schema(example = "citation_retrieval"))]
    pub origin: String,
    /// The cited source.
    pub source: AnnotationSource,
    /// Opaque producer id (e.g. `kchk_…`, `kbe_…`, a URL hash). Not interpreted
    /// by the render contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "kchk_01j9y3q8w2"))]
    pub external_id: Option<String>,
    /// Verification verdict, filled by the `citation_verification` capability.
    /// Absent means unverified (not "unsupported").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<VerificationVerdict>,
}

/// The source a [`TextAnnotation`] points to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AnnotationSource {
    /// Stable, linkable locator (e.g. `github://owner/repo@main/docs/x.md` or an
    /// `https://` URL).
    #[cfg_attr(
        feature = "openapi",
        schema(example = "github://owner/repo@main/docs/x.md")
    )]
    pub uri: String,
    /// Human-readable source title, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Architecture Overview"))]
    pub title: Option<String>,
    /// Trimmed passage that backs the claim. Display-only; never relied on for
    /// prompt reconstruction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "The control plane owns durable state.")
    )]
    pub snippet: Option<String>,
    /// Provenance within the document (line / char / page / block ranges),
    /// reusing the retrieval `location` JSONB shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<serde_json::Value>,
}

/// Outcome of citation verification (see the `citation_verification` capability).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VerificationVerdict {
    /// Whether the cited source supports the claim.
    pub status: VerificationStatus,
    /// Entailment confidence in `[0, 1]`, when the verifier produced one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 0.92))]
    pub score: Option<f32>,
}

/// Whether a cited source entails the claim it is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "entailed"))]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    /// The source supports the claim.
    Entailed,
    /// The source does not support the claim.
    Unsupported,
    /// The verifier could not decide.
    Uncertain,
}

/// Image content part (base64 or URL)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ImageContentPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl ImageContentPart {
    pub fn from_url(url: impl Into<String>) -> Self {
        Self {
            url: Some(url.into()),
            base64: None,
            media_type: None,
        }
    }

    pub fn from_base64(base64: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self {
            url: None,
            base64: Some(base64.into()),
            media_type: Some(media_type.into()),
        }
    }
}

/// Image file content part (reference to uploaded image)
///
/// This is used for images uploaded via the /images API.
/// The image data is stored separately and referenced by ID.
/// Note: Currently filtered out before sending to LLM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ImageFileContentPart {
    /// ID of the uploaded image (format: img_{32-hex})
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "img_01933b5a00007000800000000000001"))]
    pub image_id: ImageId,
    /// Original filename (for display)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

impl ImageFileContentPart {
    pub fn new(image_id: ImageId) -> Self {
        Self {
            image_id,
            filename: None,
        }
    }

    pub fn with_filename(image_id: ImageId, filename: impl Into<String>) -> Self {
        Self {
            image_id,
            filename: Some(filename.into()),
        }
    }
}

/// Tool call content part (assistant requesting tool execution)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallContentPart {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

impl ToolCallContentPart {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

/// Tool result content part (result of tool execution)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolResultContentPart {
    /// ID of the tool call this result corresponds to
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResultContentPart {
    pub fn new(
        tool_call_id: impl Into<String>,
        result: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            result,
            error,
        }
    }

    pub fn success(tool_call_id: impl Into<String>, result: serde_json::Value) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            result: Some(result),
            error: None,
        }
    }

    pub fn error(tool_call_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            result: None,
            error: Some(error.into()),
        }
    }
}

// ============================================
// Content Part Enums
// ============================================

/// A part of message content - can be text, image, image_file, tool_call, or tool_result
///
/// This is the canonical content part type used across the system.
/// API layer enables the "openapi" feature to add ToSchema derive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Text content
    Text(TextContentPart),
    /// Image content (base64 or URL)
    Image(ImageContentPart),
    /// Image file content (reference to uploaded image by ID)
    ImageFile(ImageFileContentPart),
    /// Tool call content (assistant requesting tool execution)
    ToolCall(ToolCallContentPart),
    /// Tool result content (result of tool execution)
    ToolResult(ToolResultContentPart),
    /// Provider reasoning artifact, ordered against the text and tool calls it
    /// was interleaved with.
    Reasoning(ReasoningContentPart),
}

impl ContentPart {
    /// Create a text content part
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text(TextContentPart::new(text))
    }

    /// Convert a JSON tool result into text without JSON-quoting string values.
    /// Structured values retain their JSON representation for transport and details views.
    pub fn tool_result_text(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::String(text) => Self::text(text.clone()),
            other => Self::text(other.to_string()),
        }
    }

    /// Create an image content part from URL
    pub fn image_url(url: impl Into<String>) -> Self {
        ContentPart::Image(ImageContentPart::from_url(url))
    }

    /// Create an image file content part (reference to uploaded image)
    pub fn image_file(image_id: ImageId) -> Self {
        ContentPart::ImageFile(ImageFileContentPart::new(image_id))
    }

    /// Create a tool call content part
    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        ContentPart::ToolCall(ToolCallContentPart::new(id, name, arguments))
    }

    /// Create a tool result content part
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        result: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Self {
        ContentPart::ToolResult(ToolResultContentPart::new(tool_call_id, result, error))
    }

    /// Create a reasoning content part
    pub fn reasoning(part: ReasoningContentPart) -> Self {
        ContentPart::Reasoning(part)
    }

    /// Get the reasoning artifact if this is a reasoning part
    pub fn as_reasoning(&self) -> Option<&ReasoningContentPart> {
        match self {
            ContentPart::Reasoning(r) => Some(r),
            _ => None,
        }
    }

    /// Whether this part is a reasoning artifact.
    pub fn is_reasoning(&self) -> bool {
        matches!(self, ContentPart::Reasoning(_))
    }

    /// Get text if this is a text part
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentPart::Text(t) => Some(&t.text),
            _ => None,
        }
    }

    /// Check if this is an ImageFile part
    pub fn is_image_file(&self) -> bool {
        matches!(self, ContentPart::ImageFile(_))
    }

    /// Get the content type
    pub fn content_type(&self) -> ContentType {
        match self {
            ContentPart::Text(_) => ContentType::Text,
            ContentPart::Image(_) => ContentType::Image,
            ContentPart::ImageFile(_) => ContentType::ImageFile,
            ContentPart::ToolCall(_) => ContentType::ToolCall,
            ContentPart::ToolResult(_) => ContentType::ToolResult,
            ContentPart::Reasoning(_) => ContentType::Reasoning,
        }
    }

    /// Convert content part to OpenAI-compatible format
    ///
    /// Returns `None` for content types that aren't valid in user/system messages
    /// (ImageFile, ToolCall, ToolResult are handled at message level).
    pub fn to_openai_format(&self) -> Option<serde_json::Value> {
        match self {
            ContentPart::Text(t) => Some(serde_json::json!({
                "type": "text",
                "text": t.text
            })),
            ContentPart::Image(img) => {
                if let Some(url) = &img.url {
                    Some(serde_json::json!({
                        "type": "image_url",
                        "image_url": { "url": url }
                    }))
                } else if let Some(b64) = &img.base64 {
                    let media_type = img.media_type.as_deref().unwrap_or("image/png");
                    Some(serde_json::json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", media_type, b64) }
                    }))
                } else {
                    None
                }
            }
            // ImageFile, ToolCall, ToolResult handled at message level
            _ => None,
        }
    }
}

/// Input content part - text, image, and image_file (for user input)
///
/// This is a subset of ContentPart that users can send.
/// Tool calls and results are system-generated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContentPart {
    /// Text content
    Text(TextContentPart),
    /// Image content (base64 or URL)
    Image(ImageContentPart),
    /// Image file content (reference to uploaded image by ID)
    ImageFile(ImageFileContentPart),
}

impl From<InputContentPart> for ContentPart {
    fn from(input: InputContentPart) -> Self {
        match input {
            InputContentPart::Text(t) => ContentPart::Text(t),
            InputContentPart::Image(i) => ContentPart::Image(i),
            InputContentPart::ImageFile(f) => ContentPart::ImageFile(f),
        }
    }
}

impl InputContentPart {
    /// Create a text content part
    pub fn text(text: impl Into<String>) -> Self {
        InputContentPart::Text(TextContentPart::new(text))
    }

    /// Create an image content part from URL
    pub fn image_url(url: impl Into<String>) -> Self {
        InputContentPart::Image(ImageContentPart::from_url(url))
    }

    /// Create an image file content part (reference to uploaded image)
    pub fn image_file(image_id: ImageId) -> Self {
        InputContentPart::ImageFile(ImageFileContentPart::new(image_id))
    }

    /// Get text content if this is a Text part
    pub fn as_text(&self) -> Option<&str> {
        match self {
            InputContentPart::Text(t) => Some(&t.text),
            _ => None,
        }
    }

    /// Get the content type
    pub fn content_type(&self) -> ContentType {
        match self {
            InputContentPart::Text(_) => ContentType::Text,
            InputContentPart::Image(_) => ContentType::Image,
            InputContentPart::ImageFile(_) => ContentType::ImageFile,
        }
    }
}

impl Message {
    /// Reasoning artifacts carried by this message, in emission order.
    pub fn reasoning_parts(&self) -> impl Iterator<Item = &ReasoningContentPart> {
        self.content.iter().filter_map(ContentPart::as_reasoning)
    }

    /// Whether this message carries any provider reasoning artifact.
    pub fn has_reasoning(&self) -> bool {
        self.content.iter().any(ContentPart::is_reasoning)
    }

    /// Readable reasoning across every artifact, joined for display.
    ///
    /// Display only. Replay must walk [`Message::reasoning_parts`] so each
    /// artifact keeps its own signature and position.
    pub fn reasoning_display_text(&self) -> Option<String> {
        let joined = self
            .reasoning_parts()
            .filter_map(ReasoningContentPart::display_text)
            .collect::<Vec<_>>()
            .join("\n\n");
        (!joined.is_empty()).then_some(joined)
    }

    /// Replace every reasoning part with its publishable projection, dropping
    /// opaque provider replay state. Used at API boundaries.
    pub fn into_public(mut self) -> Self {
        for part in &mut self.content {
            if let ContentPart::Reasoning(r) = part {
                *r = r.to_public();
            }
        }
        self
    }

    /// Override the generated message id.
    ///
    /// Streaming producers use this to allocate a public id before emitting
    /// `output.message.started`, then reuse it on the completed message.
    pub fn with_id(mut self, id: MessageId) -> Self {
        self.id = id;
        self
    }

    /// Create a new user message
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            role: MessageRole::User,
            content: vec![ContentPart::text(content)],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new assistant message
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            role: MessageRole::Agent,
            content: vec![ContentPart::text(content)],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new assistant message with tool calls
    ///
    /// Tool calls are stored as ContentPart::ToolCall in the content array
    /// alongside the text content. Empty text content is omitted to avoid
    /// LLM API errors (e.g., Anthropic requires non-empty text blocks).
    pub fn assistant_with_tools(
        content: impl Into<String>,
        tool_calls: Vec<crate::tool_types::ToolCall>,
    ) -> Self {
        let text_content = content.into();
        let mut parts = Vec::new();
        // Only include text part if non-empty
        if !text_content.is_empty() {
            parts.push(ContentPart::text(text_content));
        }
        for tc in tool_calls {
            parts.push(ContentPart::ToolCall(ToolCallContentPart {
                id: tc.id,
                name: tc.name,
                arguments: tc.arguments,
            }));
        }
        Self {
            id: MessageId::new(),
            role: MessageRole::Agent,
            content: parts,
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new system message
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            role: MessageRole::System,
            content: vec![ContentPart::text(content)],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        }
    }

    /// Create a tool result message
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        result: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Self {
        let tool_call_id = tool_call_id.into();
        Self {
            id: MessageId::new(),
            role: MessageRole::ToolResult,
            content: vec![ContentPart::ToolResult(ToolResultContentPart::new(
                tool_call_id,
                result,
                error,
            ))],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        }
    }

    /// Create a tool result message with images.
    ///
    /// Images are included as `ContentPart::Image` alongside the `ToolResult` part.
    /// When converted to `LlmMessage`, images become native image content blocks
    /// that the LLM can see visually (not just stringified base64).
    pub fn tool_result_with_images(
        tool_call_id: impl Into<String>,
        result: Option<serde_json::Value>,
        images: Vec<everruns_provider::tool_types::ToolResultImage>,
    ) -> Self {
        let tool_call_id = tool_call_id.into();
        let mut content = vec![ContentPart::ToolResult(ToolResultContentPart::new(
            tool_call_id,
            result,
            None,
        ))];
        for img in images {
            content.push(ContentPart::Image(ImageContentPart::from_base64(
                img.base64,
                img.media_type,
            )));
        }
        Self {
            id: MessageId::new(),
            role: MessageRole::ToolResult,
            content,
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        }
    }

    /// Set the execution phase on this message and return self.
    pub fn with_phase(mut self, phase: ExecutionPhase) -> Self {
        self.phase = Some(phase);
        self
    }

    /// Set the phase together with where it came from.
    pub fn with_phase_from(mut self, phase: ExecutionPhase, source: PhaseSource) -> Self {
        self.phase = Some(phase);
        self.phase_source = Some(source);
        self
    }

    /// Get the tool_call_id from a tool result message
    ///
    /// Returns the tool_call_id from the first ToolResult content part, if any.
    pub fn tool_call_id(&self) -> Option<&str> {
        self.content.iter().find_map(|p| match p {
            ContentPart::ToolResult(tr) => Some(tr.tool_call_id.as_str()),
            _ => None,
        })
    }

    /// Get first text content from the message
    pub fn text(&self) -> Option<&str> {
        self.content.iter().find_map(|p| p.as_text())
    }

    /// Get all tool calls from the message content
    pub fn tool_calls(&self) -> Vec<&ToolCallContentPart> {
        self.content
            .iter()
            .filter_map(|p| match p {
                ContentPart::ToolCall(tc) => Some(tc),
                _ => None,
            })
            .collect()
    }

    /// Check if this message has tool calls
    pub fn has_tool_calls(&self) -> bool {
        self.content
            .iter()
            .any(|p| matches!(p, ContentPart::ToolCall(_)))
    }

    /// Get the first tool result from the message content
    pub fn tool_result_content(&self) -> Option<&ToolResultContentPart> {
        self.content.iter().find_map(|p| match p {
            ContentPart::ToolResult(tr) => Some(tr),
            _ => None,
        })
    }

    /// Convert content to LLM-compatible string representation
    pub fn content_to_llm_string(&self) -> String {
        self.content
            .iter()
            .map(|part| match part {
                ContentPart::Text(t) => t.text.clone(),
                // Reasoning is replayed as provider-native artifacts on
                // `LlmMessage::reasoning`; it must never be flattened into
                // prompt text. Filtered out below.
                ContentPart::Reasoning(_) => String::new(),
                ContentPart::Image(_) => "[Image]".to_string(),
                ContentPart::ImageFile(_) => "[Image File]".to_string(),
                ContentPart::ToolCall(tc) => {
                    format!(
                        "Tool call: {} with arguments: {}",
                        tc.name,
                        serde_json::to_string(&tc.arguments).unwrap_or_default()
                    )
                }
                ContentPart::ToolResult(tr) => {
                    if let Some(err) = &tr.error {
                        format!("Tool error: {}", err)
                    } else if let Some(res) = &tr.result {
                        serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string())
                    } else {
                        "{}".to_string()
                    }
                }
            })
            .filter(|rendered| !rendered.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Convert message to OpenAI-compatible format
    ///
    /// Transforms internal message format to OpenAI API format:
    /// - `agent` role → `assistant`
    /// - `tool_result` role → `tool` (with tool_call_id at message level)
    /// - Tool calls formatted as `{id, type: "function", function: {name, arguments}}`
    ///
    /// Used by observability backends (e.g., Braintrust) that expect OpenAI format.
    pub fn to_openai_format(&self) -> serde_json::Value {
        let role = match self.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Agent => "assistant",
            MessageRole::ToolResult => "tool",
        };

        // Handle tool result messages (need tool_call_id at message level)
        if self.role == MessageRole::ToolResult {
            let tool_call_id = self.tool_call_id().unwrap_or("");
            let content = self
                .content
                .iter()
                .find_map(|p| match p {
                    ContentPart::ToolResult(tr) => {
                        if let Some(error) = &tr.error {
                            Some(format!("Error: {}", error))
                        } else if let Some(result) = &tr.result {
                            Some(serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string()))
                        } else {
                            Some("{}".to_string())
                        }
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "{}".to_string());

            return serde_json::json!({
                "role": role,
                "content": content,
                "tool_call_id": tool_call_id
            });
        }

        // Handle assistant messages with tool calls
        if self.role == MessageRole::Agent {
            let tool_calls: Vec<serde_json::Value> = self
                .content
                .iter()
                .filter_map(|p| match p {
                    ContentPart::ToolCall(tc) => Some(serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string())
                        }
                    })),
                    _ => None,
                })
                .collect();

            let text_content: String = self
                .content
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            if tool_calls.is_empty() {
                return serde_json::json!({
                    "role": role,
                    "content": text_content
                });
            } else {
                let mut result = serde_json::json!({
                    "role": role,
                    "tool_calls": tool_calls
                });
                if !text_content.is_empty() {
                    result["content"] = serde_json::json!(text_content);
                }
                return result;
            }
        }

        // For system/user messages, convert content parts
        let content = self.content_to_openai_format();
        serde_json::json!({
            "role": role,
            "content": content
        })
    }

    /// Convert content parts to OpenAI-compatible format
    fn content_to_openai_format(&self) -> serde_json::Value {
        // Single text content → string
        if self.content.len() == 1
            && let ContentPart::Text(t) = &self.content[0]
        {
            return serde_json::json!(t.text);
        }

        // Convert each content part
        let parts: Vec<serde_json::Value> = self
            .content
            .iter()
            .filter_map(|part| part.to_openai_format())
            .collect();

        if parts.is_empty() {
            return serde_json::json!("");
        }

        // Single text part after filtering → string
        if parts.len() == 1
            && let Some(text) = parts[0].get("text")
        {
            return text.clone();
        }

        serde_json::json!(parts)
    }
}

/// Patch dangling tool calls by adding synthetic "cancelled" results.
///
/// This ensures every tool call has a corresponding tool result,
/// preventing LLM API errors (e.g., OpenAI requires every tool_call to have a result).
///
/// This is the simple, store-free patcher used by out-of-band completions
/// (see `crate::command_host`). The main reason path uses the durable-store-aware
/// the execution kernel's transcript-repair path instead (EVE-533),
/// which can replay settled results rather than synthesizing cancellations.
pub fn patch_dangling_tool_calls(messages: &[Message]) -> Vec<Message> {
    let mut result = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        result.push(msg.clone());

        // After an assistant message with tool calls, add cancelled results for any missing ones
        if msg.role == MessageRole::Agent && msg.has_tool_calls() {
            for tc in msg.tool_calls() {
                // Look for a matching tool result in ALL subsequent messages
                let has_result = messages[(i + 1)..]
                    .iter()
                    .any(|m| m.role == MessageRole::ToolResult && m.tool_call_id() == Some(&tc.id));

                if !has_result {
                    result.push(Message::tool_result(
                        &tc.id,
                        None,
                        Some(
                            "cancelled - another message came in before it could be completed"
                                .to_string(),
                        ),
                    ));
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::ToolCall;

    #[test]
    fn test_patch_dangling_tool_calls_no_tool_calls() {
        let messages = vec![Message::user("Hello"), Message::assistant("Hi there!")];
        let patched = patch_dangling_tool_calls(&messages);
        assert_eq!(patched.len(), 2);
    }

    #[test]
    fn test_patch_dangling_tool_calls_with_result() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "NYC"}),
        };

        let messages = vec![
            Message::user("What's the weather?"),
            Message::assistant_with_tools("Let me check", vec![tool_call]),
            Message::tool_result("call_123", Some(serde_json::json!({"temp": 72})), None),
        ];

        let patched = patch_dangling_tool_calls(&messages);
        assert_eq!(patched.len(), 3);
    }

    #[test]
    fn test_patch_dangling_tool_calls_missing_result() {
        let tool_call = ToolCall {
            id: "call_456".to_string(),
            name: "search_web".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };

        let messages = vec![
            Message::user("Search for rust"),
            Message::assistant_with_tools("Searching...", vec![tool_call]),
            Message::user("Actually, never mind"),
        ];

        let patched = patch_dangling_tool_calls(&messages);
        // Should have added a cancelled result
        assert_eq!(patched.len(), 4);
        assert_eq!(patched[2].role, MessageRole::ToolResult);
        assert_eq!(patched[2].tool_call_id(), Some("call_456"));
    }

    #[test]
    fn test_user_message() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.text(), Some("Hello"));
    }

    #[test]
    fn test_assistant_message() {
        let msg = Message::assistant("Hi there!");
        assert_eq!(msg.role, MessageRole::Agent);
        assert_eq!(msg.text(), Some("Hi there!"));
    }

    #[test]
    fn test_tool_result_message() {
        let msg = Message::tool_result(
            "call_123",
            Some(serde_json::json!({"result": "success"})),
            None,
        );
        assert_eq!(msg.role, MessageRole::ToolResult);
        assert_eq!(msg.tool_call_id(), Some("call_123"));
    }

    #[test]
    fn test_assistant_with_tools_and_text() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"location": "Tokyo"}),
        };
        let msg = Message::assistant_with_tools("Let me check the weather.", vec![tool_call]);

        assert_eq!(msg.role, MessageRole::Agent);
        assert_eq!(msg.text(), Some("Let me check the weather."));
        assert_eq!(msg.tool_calls().len(), 1);
        assert_eq!(msg.tool_calls()[0].name, "get_weather");
    }

    #[test]
    fn test_assistant_with_tools_empty_text() {
        // When LLM returns only tool calls without text, we shouldn't include an empty text block
        // This is important for Anthropic API which rejects empty text content blocks
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };
        let msg = Message::assistant_with_tools("", vec![tool_call]);

        assert_eq!(msg.role, MessageRole::Agent);
        // Empty text should result in None, not Some("")
        assert_eq!(msg.text(), None);
        // But tool calls should still be present
        assert_eq!(msg.tool_calls().len(), 1);
        assert_eq!(msg.tool_calls()[0].name, "search");
        // Content should only have tool_call, no empty text part
        assert_eq!(msg.content.len(), 1);
        assert!(matches!(msg.content[0], ContentPart::ToolCall(_)));
    }

    #[test]
    fn test_assistant_with_tools_whitespace_text() {
        // Whitespace-only text is not empty (could be intentional)
        let tool_call = ToolCall {
            id: "call_456".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({}),
        };
        let msg = Message::assistant_with_tools("   ", vec![tool_call]);

        // Whitespace text is preserved (not treated as empty)
        assert_eq!(msg.text(), Some("   "));
        assert_eq!(msg.content.len(), 2); // Text + ToolCall
    }

    #[test]
    fn test_assistant_with_multiple_tool_calls() {
        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({"q": "a"}),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "fetch".to_string(),
                arguments: serde_json::json!({"url": "http://example.com"}),
            },
        ];
        let msg = Message::assistant_with_tools("", tool_calls);

        assert_eq!(msg.tool_calls().len(), 2);
        // Only tool calls, no empty text
        assert_eq!(msg.content.len(), 2);
    }

    // =========================================================================
    // OpenAI Format Conversion Tests
    // =========================================================================

    #[test]
    fn test_to_openai_format_user_message() {
        let msg = Message::user("Hello, world!");
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "user");
        assert_eq!(converted["content"], "Hello, world!");
    }

    #[test]
    fn test_to_openai_format_system_message() {
        let msg = Message::system("You are a helpful assistant.");
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "system");
        assert_eq!(converted["content"], "You are a helpful assistant.");
    }

    #[test]
    fn test_to_openai_format_assistant_role_mapping() {
        // Internal "agent" role → "assistant"
        let msg = Message::assistant("Hi there!");
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "assistant");
        assert_eq!(converted["content"], "Hi there!");
    }

    #[test]
    fn test_to_openai_format_assistant_with_tool_calls() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"location": "Tokyo"}),
        };
        let msg = Message::assistant_with_tools("Let me check.", vec![tool_call]);
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "assistant");
        assert_eq!(converted["content"], "Let me check.");

        let tool_calls = converted["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "call_123");
        assert_eq!(tool_calls[0]["type"], "function");
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
        assert_eq!(
            tool_calls[0]["function"]["arguments"],
            r#"{"location":"Tokyo"}"#
        );
    }

    #[test]
    fn test_to_openai_format_assistant_tool_calls_only() {
        // Assistant message with only tool calls (no text)
        let tool_call = ToolCall {
            id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };
        let msg = Message::assistant_with_tools("", vec![tool_call]);
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "assistant");
        // No content field when text is empty
        assert!(converted.get("content").is_none());
        assert!(converted["tool_calls"].is_array());
    }

    #[test]
    fn test_to_openai_format_tool_result_role_mapping() {
        // Internal "tool_result" role → "tool"
        let msg = Message::tool_result(
            "call_123",
            Some(serde_json::json!({"temperature": 72})),
            None,
        );
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "tool");
        assert_eq!(converted["tool_call_id"], "call_123");
        assert_eq!(converted["content"], r#"{"temperature":72}"#);
    }

    #[test]
    fn test_to_openai_format_tool_result_error() {
        let msg = Message::tool_result("call_456", None, Some("API timeout".to_string()));
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "tool");
        assert_eq!(converted["tool_call_id"], "call_456");
        assert_eq!(converted["content"], "Error: API timeout");
    }

    #[test]
    fn test_to_openai_format_full_conversation() {
        // Full conversation: user → assistant (tool call) → tool result → assistant
        let tool_call = ToolCall {
            id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };

        let messages = [
            Message::user("Search for rust"),
            Message::assistant_with_tools("", vec![tool_call]),
            Message::tool_result(
                "call_abc",
                Some(serde_json::json!({"results": ["rust-lang.org"]})),
                None,
            ),
            Message::assistant("Here are the search results."),
        ];
        let converted: Vec<_> = messages.iter().map(|m| m.to_openai_format()).collect();

        assert_eq!(converted.len(), 4);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[1]["role"], "assistant");
        assert!(converted[1]["tool_calls"].is_array());
        assert_eq!(converted[2]["role"], "tool");
        assert_eq!(converted[2]["tool_call_id"], "call_abc");
        assert_eq!(converted[3]["role"], "assistant");
    }

    // =========================================================================
    // ContentPart::to_openai_format Tests
    // =========================================================================

    #[test]
    fn test_content_part_to_openai_format_text() {
        let part = ContentPart::text("Hello");
        let converted = part.to_openai_format().unwrap();

        assert_eq!(converted["type"], "text");
        assert_eq!(converted["text"], "Hello");
    }

    #[test]
    fn test_content_part_to_openai_format_image_url() {
        let part = ContentPart::image_url("https://example.com/img.png");
        let converted = part.to_openai_format().unwrap();

        assert_eq!(converted["type"], "image_url");
        assert_eq!(converted["image_url"]["url"], "https://example.com/img.png");
    }

    #[test]
    fn test_content_part_to_openai_format_image_base64() {
        let part = ContentPart::Image(ImageContentPart::from_base64("abc123", "image/jpeg"));
        let converted = part.to_openai_format().unwrap();

        assert_eq!(converted["type"], "image_url");
        assert_eq!(
            converted["image_url"]["url"],
            "data:image/jpeg;base64,abc123"
        );
    }

    #[test]
    fn test_content_part_to_openai_format_tool_call_returns_none() {
        // ToolCall parts are handled at message level, not content part level
        let part = ContentPart::tool_call("call_1", "search", serde_json::json!({}));
        assert!(part.to_openai_format().is_none());
    }

    #[test]
    fn test_content_part_to_openai_format_tool_result_returns_none() {
        // ToolResult parts are handled at message level
        let part = ContentPart::tool_result("call_1", Some(serde_json::json!({})), None);
        assert!(part.to_openai_format().is_none());
    }

    #[test]
    fn test_execution_phase_from_has_tool_calls() {
        assert_eq!(
            ExecutionPhase::from_has_tool_calls(true),
            ExecutionPhase::Commentary
        );
        assert_eq!(
            ExecutionPhase::from_has_tool_calls(false),
            ExecutionPhase::FinalAnswer
        );
    }

    #[test]
    fn test_execution_phase_refine_streamed_hint_monotonic() {
        use ExecutionPhase::{Commentary, FinalAnswer};
        // None advances to the first observed value.
        assert_eq!(
            ExecutionPhase::refine_streamed_hint(None, Commentary),
            Some(Commentary)
        );
        assert_eq!(
            ExecutionPhase::refine_streamed_hint(None, FinalAnswer),
            Some(FinalAnswer)
        );
        // First classification wins: a later hint never flips it...
        assert_eq!(
            ExecutionPhase::refine_streamed_hint(Some(Commentary), FinalAnswer),
            Some(Commentary)
        );
        assert_eq!(
            ExecutionPhase::refine_streamed_hint(Some(FinalAnswer), Commentary),
            Some(FinalAnswer)
        );
        // ...and never reverts to None (the input is never None-valued, but a
        // repeated identical hint is a no-op).
        assert_eq!(
            ExecutionPhase::refine_streamed_hint(Some(Commentary), Commentary),
            Some(Commentary)
        );
    }

    #[test]
    fn test_execution_phase_serde_roundtrip() {
        let commentary = ExecutionPhase::Commentary;
        let json = serde_json::to_string(&commentary).unwrap();
        assert_eq!(json, "\"commentary\"");
        let deserialized: ExecutionPhase = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ExecutionPhase::Commentary);

        let final_answer = ExecutionPhase::FinalAnswer;
        let json = serde_json::to_string(&final_answer).unwrap();
        assert_eq!(json, "\"final_answer\"");
        let deserialized: ExecutionPhase = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ExecutionPhase::FinalAnswer);
    }

    #[test]
    fn test_execution_phase_deserialize_legacy() {
        let legacy_in_progress: ExecutionPhase = serde_json::from_str("\"in_progress\"").unwrap();
        assert_eq!(legacy_in_progress, ExecutionPhase::Commentary);

        let legacy_completed: ExecutionPhase = serde_json::from_str("\"completed\"").unwrap();
        assert_eq!(legacy_completed, ExecutionPhase::FinalAnswer);
    }

    #[test]
    fn test_execution_phase_deserialize_unknown_fails() {
        let result = serde_json::from_str::<ExecutionPhase>("\"bogus\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_message_with_phase() {
        let msg = Message::assistant("Hello").with_phase(ExecutionPhase::Commentary);
        assert_eq!(msg.phase, Some(ExecutionPhase::Commentary));
    }

    #[test]
    fn test_message_phase_skipped_when_none() {
        let msg = Message::assistant("Hello");
        let json = serde_json::to_value(&msg).unwrap();
        assert!(json.get("phase").is_none());
    }

    #[test]
    fn test_message_phase_included_when_set() {
        let msg = Message::assistant("Hello").with_phase(ExecutionPhase::FinalAnswer);
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json.get("phase").unwrap(), "final_answer");
    }

    #[test]
    fn test_resolve_hints_both_none() {
        let result = Controls::resolve_hints(None, None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_resolve_hints_session_only() {
        let mut session = std::collections::HashMap::new();
        session.insert("key1".into(), serde_json::json!("val1"));
        session.insert("key2".into(), serde_json::json!(42));

        let result = Controls::resolve_hints(Some(&session), None);
        assert_eq!(result.len(), 2);
        assert_eq!(result["key1"], serde_json::json!("val1"));
        assert_eq!(result["key2"], serde_json::json!(42));
    }

    #[test]
    fn test_resolve_hints_message_only() {
        let mut message = std::collections::HashMap::new();
        message.insert("key1".into(), serde_json::json!(true));

        let result = Controls::resolve_hints(None, Some(&message));
        assert_eq!(result.len(), 1);
        assert_eq!(result["key1"], serde_json::json!(true));
    }

    #[test]
    fn test_resolve_hints_message_overrides_session() {
        let mut session = std::collections::HashMap::new();
        session.insert("shared".into(), serde_json::json!("session_val"));
        session.insert("session_only".into(), serde_json::json!(1));

        let mut message = std::collections::HashMap::new();
        message.insert("shared".into(), serde_json::json!("message_val"));
        message.insert("message_only".into(), serde_json::json!(2));

        let result = Controls::resolve_hints(Some(&session), Some(&message));
        assert_eq!(result.len(), 3);
        assert_eq!(result["shared"], serde_json::json!("message_val"));
        assert_eq!(result["session_only"], serde_json::json!(1));
        assert_eq!(result["message_only"], serde_json::json!(2));
    }

    #[test]
    fn test_controls_hints_serde_roundtrip() {
        let mut hints = std::collections::HashMap::new();
        hints.insert("setup_connection".into(), serde_json::json!(true));
        hints.insert("theme".into(), serde_json::json!("dark"));

        let controls = Controls {
            hints: Some(hints),
            ..Default::default()
        };

        let json = serde_json::to_value(&controls).unwrap();
        let deserialized: Controls = serde_json::from_value(json).unwrap();
        let h = deserialized.hints.unwrap();
        assert_eq!(h["setup_connection"], serde_json::json!(true));
        assert_eq!(h["theme"], serde_json::json!("dark"));
    }

    #[test]
    fn tool_result_text_preserves_strings_without_json_escaping() {
        let value = serde_json::json!("{\n  \"count\": 1\n}");
        assert_eq!(
            ContentPart::tool_result_text(&value).as_text(),
            Some("{\n  \"count\": 1\n}")
        );
    }

    #[test]
    fn tool_result_text_serializes_structured_values() {
        let value = serde_json::json!({"count": 1});
        assert_eq!(
            ContentPart::tool_result_text(&value).as_text(),
            Some("{\"count\":1}")
        );
    }
}
