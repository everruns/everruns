// Shared streaming tool-call accumulator (EVE-672)
//
// Native streaming drivers assemble tool calls from fragments spread across many
// SSE chunks: OpenAI Chat Completions keys fragments by a numeric `index`,
// OpenAI Open Responses keys them by an `item_id` string, and Gemini pushes
// whole calls. Each driver previously open-coded this into an
// `Arc<Mutex<Vec<...>>>` with subtly different growth, argument-append, and
// finalize rules. This module centralizes the accumulation so the rules live in
// one tested place and the drivers share a single `StreamToolCallAccumulator`.
//
// The accumulator keeps arguments as an un-parsed `String` fragment buffer and
// parses the JSON exactly once at finalize (EVE-636: `push_str` is amortized
// O(total), versus re-parsing a `Value` per delta which was O(n^2)). It exposes
// two finalize modes matching the historical driver behavior:
//
// - `take_finalized`: normal finish path — malformed argument JSON degrades to
//   an empty object (`{}`), because the provider signalled a real tool-call
//   completion.
// - `take_pending_strict`: fallback flush at `[DONE]`/end-of-stream without a
//   tool-call finish — malformed argument JSON causes the call to be dropped,
//   since there was no explicit completion to trust.

use crate::tool_types::ToolCall;
use serde_json::{Value, json};

/// One in-progress tool call being assembled from streamed fragments.
#[derive(Debug, Clone)]
struct PartialToolCall {
    /// Fragment key: the numeric `index` (Chat Completions) or the stream
    /// `item_id` (Open Responses). Whole-call providers (Gemini) leave it empty.
    key: String,
    /// Provider call id, applied to the finalized [`ToolCall::id`].
    id: String,
    /// Function name.
    name: String,
    /// Accumulated JSON argument fragments (parsed once at finalize).
    arguments: String,
}

/// Accumulates streamed tool-call fragments into finalized [`ToolCall`]s.
///
/// Fragments are addressed by a string `key` (the provider's per-call index or
/// item id). Calls are emitted in first-seen order.
#[derive(Debug, Default)]
pub struct StreamToolCallAccumulator {
    calls: Vec<PartialToolCall>,
}

impl StreamToolCallAccumulator {
    /// Create an empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any fragments have been accumulated.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    fn slot(&mut self, key: &str) -> &mut PartialToolCall {
        if let Some(pos) = self.calls.iter().position(|c| c.key == key) {
            return &mut self.calls[pos];
        }
        self.calls.push(PartialToolCall {
            key: key.to_string(),
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
        });
        self.calls.last_mut().expect("just pushed")
    }

    /// Apply an OpenAI Chat Completions streamed tool-call delta, keyed by the
    /// chunk's numeric `index`. Any of id/name/arguments may be absent in a
    /// given delta; argument fragments are appended in place.
    pub fn apply_indexed_delta(
        &mut self,
        index: u32,
        id: Option<&str>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) {
        let slot = self.slot(&index.to_string());
        if let Some(id) = id {
            slot.id = id.to_string();
        }
        if let Some(name) = name {
            slot.name = name.to_string();
        }
        if let Some(args) = arguments {
            slot.arguments.push_str(args);
        }
    }

    /// Append an Open Responses `function_call_arguments.delta` fragment, keyed
    /// by `item_id`. Creates the slot if the item was not yet announced.
    pub fn append_arguments(&mut self, item_id: &str, delta: &str) {
        self.slot(item_id).arguments.push_str(delta);
    }

    /// Record an Open Responses `output_item.added` function call, keyed by
    /// `item_id`, setting its name and provider `call_id`.
    pub fn set_item(&mut self, item_id: &str, call_id: &str, name: &str) {
        let slot = self.slot(item_id);
        slot.id = call_id.to_string();
        slot.name = name.to_string();
    }

    /// Push a fully-formed tool call (Gemini emits whole `functionCall` parts
    /// with already-parsed arguments, so there is nothing to reassemble).
    pub fn push_complete(&mut self, id: String, name: String, arguments: Value) {
        self.calls.push(PartialToolCall {
            key: String::new(),
            id,
            name,
            // Store as a compact JSON string so the single finalize path parses
            // it back identically to the fragment-assembled calls.
            arguments: arguments.to_string(),
        });
    }

    /// Drain all accumulated calls, parsing each argument string once. Malformed
    /// argument JSON degrades to an empty object — the normal finish path, where
    /// the provider explicitly signalled tool-call completion.
    pub fn take_finalized(&mut self) -> Vec<ToolCall> {
        std::mem::take(&mut self.calls)
            .into_iter()
            .map(|c| ToolCall {
                id: c.id,
                name: c.name,
                arguments: parse_arguments(&c.arguments).unwrap_or_else(|| json!({})),
            })
            .collect()
    }

    /// Drain all accumulated calls for a *fallback* flush (end-of-stream without
    /// an explicit tool-call finish). Malformed argument JSON drops the call
    /// rather than fabricating an empty object, since no completion was signalled.
    pub fn take_pending_strict(&mut self) -> Vec<ToolCall> {
        std::mem::take(&mut self.calls)
            .into_iter()
            .filter_map(|c| {
                Some(ToolCall {
                    id: c.id,
                    name: c.name,
                    arguments: parse_arguments(&c.arguments)?,
                })
            })
            .collect()
    }

    /// Drain accumulated calls, keeping only those with a non-empty name and
    /// parsing arguments once (empty/malformed → `{}`). Matches the Open
    /// Responses finalize path, which skips announced-but-unnamed items.
    pub fn take_named(&mut self) -> Vec<ToolCall> {
        std::mem::take(&mut self.calls)
            .into_iter()
            .filter(|c| !c.name.is_empty())
            .map(|c| ToolCall {
                id: c.id,
                name: c.name,
                arguments: parse_arguments(&c.arguments).unwrap_or_else(|| json!({})),
            })
            .collect()
    }
}

/// Parse an accumulated argument fragment buffer into JSON. An empty buffer is
/// treated as an empty object; a non-empty buffer that fails to parse returns
/// `None` so callers can choose their fallback.
fn parse_arguments(buffer: &str) -> Option<Value> {
    if buffer.is_empty() {
        return Some(json!({}));
    }
    serde_json::from_str(buffer).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_delta_reassembles_fragmented_arguments() {
        let mut acc = StreamToolCallAccumulator::new();
        acc.apply_indexed_delta(0, Some("call_a"), Some("get_weather"), Some(""));
        acc.apply_indexed_delta(0, None, None, Some("{\"city\":"));
        acc.apply_indexed_delta(0, None, None, Some("\"Paris\"}"));

        let calls = acc.take_finalized();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments, json!({"city": "Paris"}));
        assert!(acc.is_empty());
    }

    #[test]
    fn indexed_delta_grows_to_index_and_preserves_order() {
        let mut acc = StreamToolCallAccumulator::new();
        // Deltas can arrive for index 1 before index 0 has all fields, but the
        // first-seen order is preserved.
        acc.apply_indexed_delta(0, Some("c0"), Some("first"), Some("{}"));
        acc.apply_indexed_delta(1, Some("c1"), Some("second"), Some("{}"));
        let calls = acc.take_finalized();
        assert_eq!(calls[0].name, "first");
        assert_eq!(calls[1].name, "second");
    }

    #[test]
    fn empty_arguments_finalize_to_empty_object() {
        let mut acc = StreamToolCallAccumulator::new();
        acc.apply_indexed_delta(0, Some("c"), Some("noop"), None);
        let calls = acc.take_finalized();
        assert_eq!(calls[0].arguments, json!({}));
    }

    #[test]
    fn take_finalized_degrades_malformed_json_to_empty_object() {
        let mut acc = StreamToolCallAccumulator::new();
        acc.apply_indexed_delta(0, Some("c"), Some("bad"), Some("{not json"));
        let calls = acc.take_finalized();
        assert_eq!(calls[0].arguments, json!({}));
    }

    #[test]
    fn take_pending_strict_drops_malformed_json() {
        let mut acc = StreamToolCallAccumulator::new();
        acc.apply_indexed_delta(0, Some("c"), Some("bad"), Some("{not json"));
        acc.apply_indexed_delta(1, Some("d"), Some("good"), Some("{\"ok\":true}"));
        let calls = acc.take_pending_strict();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "good");
    }

    #[test]
    fn item_keyed_flow_matches_open_responses() {
        let mut acc = StreamToolCallAccumulator::new();
        acc.set_item("fc_1", "call_1", "get_weather");
        acc.append_arguments("fc_1", "{\"city\":");
        acc.append_arguments("fc_1", "\"Paris\"}");
        let calls = acc.take_named();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments, json!({"city": "Paris"}));
    }

    #[test]
    fn take_named_skips_unnamed_items() {
        let mut acc = StreamToolCallAccumulator::new();
        // An arguments delta arrived before the item was announced with a name.
        acc.append_arguments("fc_orphan", "{}");
        acc.set_item("fc_1", "call_1", "named");
        acc.append_arguments("fc_1", "{}");
        let calls = acc.take_named();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "named");
    }

    #[test]
    fn push_complete_serializes_and_reparses_identically() {
        let mut acc = StreamToolCallAccumulator::new();
        acc.push_complete("call_0".into(), "f".into(), json!({"a": 1, "b": "x"}));
        let calls = acc.take_finalized();
        assert_eq!(calls[0].arguments, json!({"a": 1, "b": "x"}));
    }
}
