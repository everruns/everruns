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

    fn calls_json(calls: Vec<ToolCall>) -> Value {
        serde_json::to_value(calls).unwrap()
    }

    #[test]
    fn interleaved_sparse_indexes_keep_first_seen_order_and_complete_payloads() {
        let mut acc = StreamToolCallAccumulator::new();
        acc.apply_indexed_delta(42, None, None, Some("{\"city\":"));
        acc.apply_indexed_delta(2, Some("second"), Some("other"), Some("{\"n\":2}"));
        acc.apply_indexed_delta(42, Some("first"), Some("weather"), Some("\"Paris 🦀\"}"));
        assert_eq!(
            calls_json(acc.take_finalized()),
            json!([
                {"id":"first","name":"weather","arguments":{"city":"Paris 🦀"}},
                {"id":"second","name":"other","arguments":{"n":2}}
            ])
        );
        assert!(acc.is_empty());
        assert!(acc.take_finalized().is_empty());
        acc.apply_indexed_delta(42, Some("fresh"), Some("again"), Some("null"));
        assert_eq!(
            calls_json(acc.take_finalized()),
            json!([{"id":"fresh","name":"again","arguments":null}])
        );
    }

    #[test]
    fn finalization_modes_have_distinct_malformed_and_unnamed_contracts() {
        for mode in ["normal", "strict", "named"] {
            let mut acc = StreamToolCallAccumulator::new();
            acc.apply_indexed_delta(0, Some("empty"), Some("noop"), None);
            acc.apply_indexed_delta(1, Some("bad"), Some("broken"), Some("{bad"));
            acc.apply_indexed_delta(2, Some("valid"), Some("good"), Some("{\"ok\":true}"));
            acc.apply_indexed_delta(3, Some("unnamed"), None, Some("[]"));
            let actual = match mode {
                "normal" => acc.take_finalized(),
                "strict" => acc.take_pending_strict(),
                _ => acc.take_named(),
            };
            let expected = match mode {
                "normal" => json!([
                    {"id":"empty","name":"noop","arguments":{}}, {"id":"bad","name":"broken","arguments":{}},
                    {"id":"valid","name":"good","arguments":{"ok":true}}, {"id":"unnamed","name":"","arguments":[]}
                ]),
                "strict" => json!([
                    {"id":"empty","name":"noop","arguments":{}},
                    {"id":"valid","name":"good","arguments":{"ok":true}}, {"id":"unnamed","name":"","arguments":[]}
                ]),
                _ => json!([
                    {"id":"empty","name":"noop","arguments":{}}, {"id":"bad","name":"broken","arguments":{}},
                    {"id":"valid","name":"good","arguments":{"ok":true}}
                ]),
            };
            assert_eq!(calls_json(actual), expected, "{mode}");
            assert!(acc.is_empty(), "{mode} must drain rejected entries too");
            assert!(acc.take_finalized().is_empty());
        }
    }

    #[test]
    fn item_fragments_can_precede_metadata_without_losing_order_or_arguments() {
        let mut acc = StreamToolCallAccumulator::new();
        acc.append_arguments("late", "{\"city\":");
        acc.set_item("second", "call_2", "second_tool");
        acc.append_arguments("second", "{}");
        acc.set_item("late", "call_1", "weather");
        acc.append_arguments("late", "\"Paris\"}");
        acc.append_arguments("orphan", "{}");
        assert_eq!(
            calls_json(acc.take_named()),
            json!([
                {"id":"call_1","name":"weather","arguments":{"city":"Paris"}},
                {"id":"call_2","name":"second_tool","arguments":{}}
            ])
        );
        assert!(acc.is_empty());
    }

    #[test]
    fn complete_calls_preserve_every_json_shape_and_distinct_empty_keys() {
        let mut acc = StreamToolCallAccumulator::new();
        acc.push_complete("a".into(), "first".into(), json!({"a":1,"b":"x"}));
        acc.push_complete(
            "b".into(),
            "second".into(),
            json!([null, true, 9007199254740993_u64, "🦀"]),
        );
        acc.push_complete("c".into(), "third".into(), Value::Null);
        assert_eq!(
            calls_json(acc.take_finalized()),
            json!([
                {"id":"a","name":"first","arguments":{"a":1,"b":"x"}},
                {"id":"b","name":"second","arguments":[null,true,9007199254740993_u64,"🦀"]},
                {"id":"c","name":"third","arguments":null}
            ])
        );
        assert!(acc.is_empty());
    }
}
