use crate::compact::CompactOutputItem;
use serde::{Deserialize, Serialize};

/// Ordered provider-owned context returned by a native compaction operation.
///
/// The runtime carries this value without interpreting or exposing its opaque
/// payload. The matching provider driver is responsible for putting the items
/// back on the wire exactly as returned.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderOpaqueContext {
    /// Standalone `output` returned by OpenAI `/responses/compact`.
    OpenResponsesCompact {
        output: Vec<CompactOutputItem>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_state: Option<crate::reasoning_updates::ReasoningState>,
    },
    /// Exact Anthropic `messages` prefix through a completed compaction
    /// response. The matching driver owns validation and replay.
    AnthropicMessagesPrefix { messages_json: String },
}

impl std::fmt::Debug for ProviderOpaqueContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenResponsesCompact { output, .. } => f
                .debug_struct("OpenResponsesCompact")
                .field("item_count", &output.len())
                .finish_non_exhaustive(),
            Self::AnthropicMessagesPrefix { messages_json } => f
                .debug_struct("AnthropicMessagesPrefix")
                .field("bytes", &messages_json.len())
                .finish_non_exhaustive(),
        }
    }
}

/// Provider-owned checkpoint candidate produced by a completed response.
#[derive(Debug, Clone)]
pub struct ProviderCheckpointCandidate {
    pub format_version: u32,
    pub context: ProviderOpaqueContext,
}
