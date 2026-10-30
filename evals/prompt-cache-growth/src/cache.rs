//! Per-call prompt-cache accounting from a session's `llm.generation` events.
//!
//! Each model call reports how much of its prompt the provider served from
//! cache. A call that reuses less than half of the previous call's prompt is a
//! cache break. A break gets a cause by diffing the two stored request bodies in
//! wire order (model, cache key, instructions, tools, history). The first part
//! that changed is the cause, since everything after it is a new prefix to the
//! provider. A break with no visible change is the provider's (eviction,
//! routing to another cache, or a prefix under the minimum cacheable size).

use serde_json::{Value, json};

/// Prompts below this many tokens are not cached by most providers, so a miss
/// after one says nothing about prefix stability.
const MIN_CACHEABLE_PROMPT: u64 = 1024;

/// A call keeps the cache when it reads at least this share of the previous
/// call's prompt from cache.
const KEPT_SHARE: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// A different model: a cache never spans models.
    Model,
    /// The provider's cache routing key changed.
    CacheKey,
    /// System prompt or instructions changed.
    Instructions,
    /// The tool list changed somewhere other than its end.
    ToolsReordered,
    /// Tools were only appended. On models that receive the tool list before
    /// the conversation this still invalidates the whole history.
    ToolsAppended,
    /// An earlier message changed (a re-rendered fact, a masked tool output).
    HistoryRewritten,
    /// The request only appended to the previous one; the provider still missed.
    Provider,
    /// A request body was not captured, so the cause cannot be read.
    Unknown,
}

impl Cause {
    pub const ALL: [Cause; 8] = [
        Cause::Model,
        Cause::CacheKey,
        Cause::Instructions,
        Cause::ToolsReordered,
        Cause::ToolsAppended,
        Cause::HistoryRewritten,
        Cause::Provider,
        Cause::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Cause::Model => "model",
            Cause::CacheKey => "cache_key",
            Cause::Instructions => "instructions",
            Cause::ToolsReordered => "tools_reordered",
            Cause::ToolsAppended => "tools_appended",
            Cause::HistoryRewritten => "history_rewritten",
            Cause::Provider => "provider",
            Cause::Unknown => "unknown",
        }
    }

    /// Breaks Everruns can avoid by keeping the request append-only.
    /// `tools_appended` is excluded: on models without inline tool changes it is
    /// the cost of revealing a tool, not a defect.
    pub fn is_prefix_defect(self) -> bool {
        matches!(
            self,
            Cause::CacheKey | Cause::Instructions | Cause::ToolsReordered | Cause::HistoryRewritten
        )
    }
}

#[derive(Debug, Clone)]
pub struct Call {
    pub sequence: i64,
    /// Uncached input tokens.
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub break_cause: Option<Cause>,
    /// Where the break starts, for reading a failure (`history[12]`, a tool name).
    pub break_detail: Option<String>,
    request: Option<Value>,
}

impl Call {
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub calls: Vec<Call>,
}

impl Report {
    pub fn from_events(events: &[Value]) -> Self {
        let mut calls: Vec<Call> = events
            .iter()
            .filter(|event| event["type"] == "llm.generation")
            .map(call_of)
            .collect();
        calls.sort_by_key(|call| call.sequence);
        for index in 1..calls.len() {
            let (before, after) = calls.split_at_mut(index);
            let (previous, call) = (&before[index - 1], &mut after[0]);
            if previous.prompt_tokens() < MIN_CACHEABLE_PROMPT {
                continue;
            }
            let kept = call.cache_read_tokens as f64 / previous.prompt_tokens() as f64;
            if kept >= KEPT_SHARE {
                continue;
            }
            let (cause, detail) = match (&previous.request, &call.request) {
                (Some(a), Some(b)) => diff(a, b),
                _ => (Cause::Unknown, None),
            };
            call.break_cause = Some(cause);
            call.break_detail = detail;
        }
        Self { calls }
    }

    /// Share of prompt tokens read from cache, skipping the first call, which
    /// can only write.
    pub fn hit_rate(&self) -> Option<f64> {
        rate(self.calls.iter().skip(1))
    }

    /// The same share over the later half of the calls: how the cache holds up
    /// once the session has grown.
    pub fn late_hit_rate(&self) -> Option<f64> {
        let half = self.calls.len() / 2;
        rate(self.calls.iter().skip(half.max(1)))
    }

    pub fn breaks(&self) -> impl Iterator<Item = &Call> {
        self.calls.iter().filter(|call| call.break_cause.is_some())
    }

    pub fn count(&self, cause: Cause) -> usize {
        self.breaks()
            .filter(|call| call.break_cause == Some(cause))
            .count()
    }

    /// One line per break, for a failure reason or the runs report.
    pub fn describe_breaks(&self) -> Vec<String> {
        self.breaks()
            .map(|call| {
                let cause = call.break_cause.map(Cause::as_str).unwrap_or_default();
                match &call.break_detail {
                    Some(detail) => format!("seq {}: {cause} at {detail}", call.sequence),
                    None => format!("seq {}: {cause}", call.sequence),
                }
            })
            .collect()
    }

    pub fn to_json(&self) -> Value {
        Value::Array(
            self.calls
                .iter()
                .map(|call| {
                    json!({
                        "sequence": call.sequence,
                        "input_tokens": call.input_tokens,
                        "cache_read_tokens": call.cache_read_tokens,
                        "cache_write_tokens": call.cache_write_tokens,
                        "break": call.break_cause.map(Cause::as_str),
                        "break_detail": call.break_detail,
                    })
                })
                .collect(),
        )
    }
}

fn rate<'a>(calls: impl Iterator<Item = &'a Call>) -> Option<f64> {
    let (mut cached, mut total) = (0u64, 0u64);
    for call in calls {
        cached += call.cache_read_tokens;
        total += call.prompt_tokens();
    }
    (total > 0).then(|| cached as f64 / total as f64)
}

fn call_of(event: &Value) -> Call {
    let data = &event["data"];
    let usage = data
        .pointer("/metadata/usage")
        .filter(|usage| usage.is_object())
        .unwrap_or(&data["usage"]);
    let number = |key: &str| usage[key].as_u64().unwrap_or(0);
    // Stored either as an object or as its JSON text.
    let request = match &data["request_body"] {
        Value::String(text) => serde_json::from_str(text).ok(),
        Value::Object(_) => Some(data["request_body"].clone()),
        _ => None,
    };
    Call {
        sequence: event["sequence"].as_i64().unwrap_or(0),
        input_tokens: number("input_tokens"),
        cache_read_tokens: number("cache_read_tokens"),
        cache_write_tokens: number("cache_creation_tokens"),
        break_cause: None,
        break_detail: None,
        request,
    }
}

/// The first part of the request that changed, in the order the provider reads
/// it. Covers the Responses (`instructions`, `input`), Chat Completions
/// (`messages`) and Anthropic Messages (`system`, `messages`) shapes.
fn diff(a: &Value, b: &Value) -> (Cause, Option<String>) {
    if a["model"] != b["model"] {
        return (Cause::Model, None);
    }
    if a["prompt_cache_key"] != b["prompt_cache_key"] {
        return (Cause::CacheKey, None);
    }
    if a["instructions"] != b["instructions"] || a["system"] != b["system"] {
        return (Cause::Instructions, None);
    }
    let empty = Vec::new();
    let tools = |body: &Value| body["tools"].as_array().unwrap_or(&empty).clone();
    let (tools_a, tools_b) = (tools(a), tools(b));
    if tools_a != tools_b {
        if let Some(at) = first_difference(&tools_a, &tools_b)
            && at < tools_a.len()
        {
            let name = tool_name(&tools_b.get(at).cloned().unwrap_or(Value::Null));
            return (Cause::ToolsReordered, Some(format!("tools[{at}] {name}")));
        }
        let added: Vec<String> = tools_b[tools_a.len()..].iter().map(tool_name).collect();
        return (Cause::ToolsAppended, Some(added.join(", ")));
    }
    // A request chained to the previous response carries only the new items,
    // so its history cannot be compared item by item.
    if !a["previous_response_id"].is_null() || !b["previous_response_id"].is_null() {
        return (Cause::Provider, Some("a chained request".to_string()));
    }
    let history = |body: &Value| {
        body.get("input")
            .or_else(|| body.get("messages"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (history_a, history_b) = (history(a), history(b));
    // The last item of the previous request may be a volatile trailer (a
    // continuation's facts block) that the next request drops on purpose; the
    // cache holds everything before it.
    if let Some(at) = first_difference(&history_a, &history_b)
        && at + 1 < history_a.len()
    {
        return (Cause::HistoryRewritten, Some(format!("history[{at}]")));
    }
    (Cause::Provider, None)
}

/// Index of the first item where `b` stops extending `a`; `None` when `a` is a
/// prefix of `b`.
fn first_difference(a: &[Value], b: &[Value]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or((a.len() > b.len()).then_some(b.len()))
}

fn tool_name(tool: &Value) -> String {
    tool.pointer("/function/name")
        .or_else(|| tool.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation(sequence: i64, input: u64, cached: u64, request: Value) -> Value {
        json!({
            "type": "llm.generation",
            "sequence": sequence,
            "data": {
                "metadata": { "usage": { "input_tokens": input, "cache_read_tokens": cached } },
                "request_body": request,
            },
        })
    }

    fn body(tools: &[&str], history: &[&str]) -> Value {
        json!({
            "model": "m",
            "prompt_cache_key": "k",
            "instructions": "be brief",
            "tools": tools.iter().map(|name| json!({ "type": "function", "name": name })).collect::<Vec<_>>(),
            "input": history.iter().map(|text| json!({ "role": "user", "content": text })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn an_append_only_session_has_no_breaks() {
        let events = [
            generation(1, 4000, 0, body(&["a"], &["u1"])),
            generation(2, 200, 4000, body(&["a"], &["u1", "r1", "u2"])),
            generation(3, 200, 4200, body(&["a"], &["u1", "r1", "u2", "r2", "u3"])),
        ];
        let report = Report::from_events(&events);
        assert_eq!(report.breaks().count(), 0);
        assert!(report.hit_rate().unwrap() > 0.9);
    }

    #[test]
    fn a_rewritten_earlier_message_is_a_history_break() {
        let events = [
            generation(1, 4000, 0, body(&["a"], &["u1", "tool output", "u2"])),
            generation(
                2,
                4200,
                100,
                body(&["a"], &["u1", "[masked]", "u2", "r2", "u3"]),
            ),
        ];
        let report = Report::from_events(&events);
        assert_eq!(report.count(Cause::HistoryRewritten), 1);
        assert_eq!(
            report.describe_breaks(),
            ["seq 2: history_rewritten at history[1]"]
        );
    }

    #[test]
    fn a_dropped_volatile_trailer_is_not_a_rewrite() {
        let events = [
            generation(1, 4000, 0, body(&["a"], &["u1", "facts at 10:00"])),
            generation(2, 4100, 0, body(&["a"], &["u1", "r1", "u2"])),
        ];
        let report = Report::from_events(&events);
        assert_eq!(report.count(Cause::Provider), 1);
    }

    #[test]
    fn tools_inserted_mid_list_are_a_reorder_and_appended_tools_are_not() {
        let events = [
            generation(1, 4000, 0, body(&["b", "d"], &["u1"])),
            generation(2, 4100, 0, body(&["b", "d", "a"], &["u1", "r1"])),
            generation(3, 4300, 0, body(&["a", "b", "d"], &["u1", "r1", "u2"])),
        ];
        let report = Report::from_events(&events);
        let causes: Vec<_> = report.breaks().map(|c| c.break_detail.clone()).collect();
        assert_eq!(report.count(Cause::ToolsAppended), 1);
        assert_eq!(report.count(Cause::ToolsReordered), 1);
        assert_eq!(
            causes,
            [Some("a".to_string()), Some("tools[0] a".to_string())]
        );
        assert!(!Cause::ToolsAppended.is_prefix_defect());
        assert!(Cause::ToolsReordered.is_prefix_defect());
    }

    #[test]
    fn a_changed_cache_key_is_reported_before_the_tools() {
        let mut next = body(&["b", "a"], &["u1", "r1"]);
        next["prompt_cache_key"] = json!("other");
        let events = [
            generation(1, 4000, 0, body(&["b"], &["u1"])),
            generation(2, 4100, 0, next),
        ];
        assert_eq!(Report::from_events(&events).count(Cause::CacheKey), 1);
    }

    #[test]
    fn a_chained_request_is_not_read_as_a_rewrite() {
        let mut chained = body(&["a"], &["tool output"]);
        chained["previous_response_id"] = json!("resp_1");
        let events = [
            generation(1, 4000, 0, chained),
            generation(2, 4100, 0, body(&["a"], &["u1", "r1", "tool output", "u2"])),
        ];
        let report = Report::from_events(&events);
        assert_eq!(report.count(Cause::Provider), 1);
        assert_eq!(report.count(Cause::HistoryRewritten), 0);
    }

    #[test]
    fn small_prompts_and_missing_bodies_are_handled() {
        let events = [
            generation(1, 500, 0, body(&[], &["u1"])),
            generation(2, 4000, 0, Value::Null),
            generation(3, 4100, 0, Value::Null),
        ];
        let report = Report::from_events(&events);
        // The first prompt is too small to cache; the third has no body to diff.
        assert_eq!(report.breaks().count(), 1);
        assert_eq!(report.count(Cause::Unknown), 1);
    }

    #[test]
    fn late_hit_rate_covers_the_second_half() {
        let events = [
            generation(1, 4000, 0, body(&["a"], &["u1"])),
            generation(2, 4000, 0, body(&["a"], &["u1", "r1"])),
            generation(3, 100, 4000, body(&["a"], &["u1", "r1", "u2"])),
            generation(4, 100, 4100, body(&["a"], &["u1", "r1", "u2", "r2"])),
        ];
        let report = Report::from_events(&events);
        let late = report.late_hit_rate().unwrap();
        assert!(late > report.hit_rate().unwrap());
        assert!((late - 8100.0 / 8300.0).abs() < 1e-9);
    }
}
