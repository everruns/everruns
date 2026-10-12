// Prompt-cache reset detection for the session trace.
//
// Decision: detection runs at read time against the stored `llm.generation`
// payloads of a call and the call before it, not in the runtime. Both payloads
// carry the model, tools and messages that were sent, so the trace can name
// what changed instead of only noticing that the cache read dropped. Providers
// report token counts only, so the cause is the first difference found in
// wire order (model, tools, system prompt, history), then an idle gap longer
// than the requested retention; a reset with none of those is `unknown`.

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::types::{TraceCacheReset, TraceRequestUsage};

/// Caches shorter than this are below every provider's minimum cacheable
/// prefix (1,024 tokens on the smallest), so losing one is not a reset.
const MIN_ESTABLISHED_TOKENS: i64 = 1024;
/// A prompt this long sent with caching off, and nothing read or written,
/// earns a hint that caching would help. Twice the largest common minimum.
pub(super) const LONG_UNCACHED_PROMPT_TOKENS: i64 = 4096;

/// One model call as stored: when it ran and what it sent.
pub(super) struct CallSnapshot<'a> {
    pub model: Option<&'a str>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub data: &'a Value,
}

/// Tokens the call left in the provider cache: what it read plus what it wrote.
fn established(usage: &TraceRequestUsage) -> i64 {
    usage.cache_read_tokens + usage.cache_creation_tokens
}

/// Whether the current call lost most of the prefix the previous one cached,
/// and the likely cause.
pub(super) fn detect(
    previous: &CallSnapshot<'_>,
    previous_usage: &TraceRequestUsage,
    current: &CallSnapshot<'_>,
    current_usage: &TraceRequestUsage,
) -> Option<TraceCacheReset> {
    let previous_cached = established(previous_usage);
    if previous_cached < MIN_ESTABLISHED_TOKENS
        || current_usage.cache_read_tokens * 2 >= previous_cached
    {
        return None;
    }
    let idle_seconds = (current.started_at - previous.ended_at)
        .num_seconds()
        .max(0);
    Some(TraceCacheReset {
        previous_cached_tokens: previous_cached,
        lost_tokens: previous_cached - current_usage.cache_read_tokens,
        reason: reason(previous, current, idle_seconds).to_string(),
        idle_seconds,
    })
}

fn reason(
    previous: &CallSnapshot<'_>,
    current: &CallSnapshot<'_>,
    idle_seconds: i64,
) -> &'static str {
    if previous.model != current.model {
        return "model_changed";
    }
    if previous.data.get("tools") != current.data.get("tools") {
        return "tools_changed";
    }
    let before = messages(previous.data);
    let after = messages(current.data);
    if leading_system(before) != leading_system(after) {
        return "system_changed";
    }
    // The previous call's last message may be a volatile tail (turn facts) that
    // is never replayed, so only the history before it must carry over.
    let carried = before.len().saturating_sub(1);
    if after.len() < carried || before[..carried] != after[..carried] {
        return "history_changed";
    }
    if idle_seconds > retention_seconds(current.data) {
        return "expired";
    }
    "unknown"
}

fn messages(data: &Value) -> &[Value] {
    data.get("messages")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn leading_system(messages: &[Value]) -> &[Value] {
    let end = messages
        .iter()
        .position(|m| m.get("role").and_then(Value::as_str) != Some("system"))
        .unwrap_or(messages.len());
    &messages[..end]
}

/// Retention the driver asked for, from the generation's request options.
/// Providers without a reported retention keep the common 5 minute default.
fn retention_seconds(data: &Value) -> i64 {
    let retention = data
        .pointer("/metadata/request_options/prompt_cache/retention")
        .and_then(Value::as_str);
    match retention {
        Some("1h") => 3600,
        Some("30m") => 1800,
        _ => 300,
    }
}

/// A long prompt sent with caching off that neither read nor wrote cache.
pub(super) fn caching_would_help(usage: &TraceRequestUsage) -> bool {
    usage.prompt_cache.is_none()
        && usage.cache_read_tokens == 0
        && usage.cache_creation_tokens == 0
        && usage.input_tokens >= LONG_UNCACHED_PROMPT_TOKENS
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use serde_json::json;

    fn usage(read: i64, write: i64) -> TraceRequestUsage {
        TraceRequestUsage {
            input_tokens: 100,
            cache_read_tokens: read,
            cache_creation_tokens: write,
            output_tokens: 10,
            cached_through: None,
            prompt_cache: Some("cache_control".into()),
            cache_reset: None,
            caching_would_help: false,
        }
    }

    fn data(tools: Value, messages: Value) -> Value {
        json!({ "tools": tools, "messages": messages })
    }

    fn base_messages() -> Value {
        json!([
            {"role": "system", "content": "stable"},
            {"role": "user", "content": "one"},
            {"role": "agent", "content": "two"},
            {"role": "system", "content": "<facts>volatile</facts>"}
        ])
    }

    fn next_messages() -> Value {
        json!([
            {"role": "system", "content": "stable"},
            {"role": "user", "content": "one"},
            {"role": "agent", "content": "two"},
            {"role": "user", "content": "three"}
        ])
    }

    fn classify(
        previous: (&str, &Value),
        current: (&str, &Value),
        idle: i64,
        read_after: i64,
    ) -> Option<TraceCacheReset> {
        let t0 = Utc::now();
        let prev = CallSnapshot {
            model: Some(previous.0),
            started_at: t0,
            ended_at: t0 + Duration::seconds(2),
            data: previous.1,
        };
        let cur = CallSnapshot {
            model: Some(current.0),
            started_at: t0 + Duration::seconds(2 + idle),
            ended_at: t0 + Duration::seconds(4 + idle),
            data: current.1,
        };
        detect(&prev, &usage(8000, 2000), &cur, &usage(read_after, 500))
    }

    #[test]
    fn a_warm_cache_is_not_a_reset() {
        let before = data(json!([{"name": "a"}]), base_messages());
        let after = data(json!([{"name": "a"}]), next_messages());
        assert!(classify(("m", &before), ("m", &after), 10, 9000).is_none());
        // Half the prefix still read is a partial hit, not a reset.
        assert!(classify(("m", &before), ("m", &after), 10, 5000).is_none());
    }

    #[test]
    fn small_caches_never_count_as_resets() {
        let before = data(json!([]), base_messages());
        let t0 = Utc::now();
        let call = CallSnapshot {
            model: Some("m"),
            started_at: t0,
            ended_at: t0,
            data: &before,
        };
        assert!(detect(&call, &usage(500, 400), &call, &usage(0, 0)).is_none());
    }

    #[test]
    fn the_cause_is_the_first_difference_in_wire_order() {
        let before = data(json!([{"name": "a"}]), base_messages());
        let reason = |current: (&str, &Value), idle| {
            classify(("m", &before), current, idle, 0).unwrap().reason
        };

        assert_eq!(reason(("other", &before), 1), "model_changed");
        let tools = data(json!([{"name": "a"}, {"name": "b"}]), next_messages());
        assert_eq!(reason(("m", &tools), 1), "tools_changed");
        let mut system = next_messages();
        system[0]["content"] = json!("changed");
        assert_eq!(
            reason(("m", &data(json!([{"name": "a"}]), system)), 1),
            "system_changed"
        );
        let mut history = next_messages();
        history[1]["content"] = json!("rewritten");
        assert_eq!(
            reason(("m", &data(json!([{"name": "a"}]), history)), 1),
            "history_changed"
        );
        let same = data(json!([{"name": "a"}]), next_messages());
        assert_eq!(reason(("m", &same), 301), "expired");
        assert_eq!(reason(("m", &same), 60), "unknown");
    }

    #[test]
    fn requested_retention_decides_when_an_idle_gap_explains_a_reset() {
        let before = data(json!([]), base_messages());
        let mut after = data(json!([]), next_messages());
        after["metadata"] = json!({"request_options": {"prompt_cache": {"retention": "1h"}}});
        let reset = classify(("m", &before), ("m", &after), 900, 0).unwrap();
        assert_eq!(reset.reason, "unknown");
        assert_eq!(reset.idle_seconds, 900);
        assert_eq!(reset.previous_cached_tokens, 10_000);
        assert_eq!(reset.lost_tokens, 10_000);
        assert_eq!(
            classify(("m", &before), ("m", &after), 3700, 0)
                .unwrap()
                .reason,
            "expired"
        );
    }

    #[test]
    fn long_uncached_prompts_get_a_hint() {
        let mut off = usage(0, 0);
        off.prompt_cache = None;
        off.input_tokens = LONG_UNCACHED_PROMPT_TOKENS;
        assert!(caching_would_help(&off));
        off.input_tokens -= 1;
        assert!(!caching_would_help(&off));
        // Implicit provider caching already reads cache without the capability.
        let mut implicit = usage(3000, 0);
        implicit.prompt_cache = None;
        implicit.input_tokens = 10_000;
        assert!(!caching_would_help(&implicit));
    }
}
