//! Scorers. What each fake tool was called with, and whether the call was
//! refused, comes from the tool events (`subject::attempts`), never from the
//! model's account of what it did.

use mira::scorer::scorer;
use mira::{Sample, Score, Scorer, Transcript};
use regex::Regex;
use serde_json::Value;

use crate::subject::CALLS_KEY;

pub const REJECTED: &str = "reveal.rejected_calls";
pub const STUB_BEFORE_SEARCH: &str = "reveal.calls_to_stub_before_search";
pub const FAKE_CALLS: &str = "reveal.fake_tool_calls";
pub const SEARCHES: &str = "reveal.tool_searches";

pub fn calls(t: &Transcript) -> Vec<Value> {
    t.metadata
        .get(CALLS_KEY)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn expectations(sample: &Sample) -> Option<&Vec<Value>> {
    sample
        .metadata
        .get("expect_calls")
        .and_then(Value::as_array)
}

fn tool_of(entry: &Value) -> &str {
    entry
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn is_valid(call: &Value) -> bool {
    call.get("valid").and_then(Value::as_bool).unwrap_or(false)
}

/// `correct_call`: every expected tool received at least one call whose
/// arguments validated against its schema and match the expected values.
pub fn correct_call() -> Box<dyn Scorer> {
    scorer("correct_call", |sample: &Sample, t: &Transcript| {
        let Some(expect) = expectations(sample) else {
            return Score::na("correct_call", "sample declares no expected calls");
        };
        let made = calls(t);
        let mut failures = Vec::new();
        for entry in expect {
            let tool = tool_of(entry);
            let of_tool: Vec<&Value> = made.iter().filter(|c| c["tool"] == tool).collect();
            if !of_tool
                .iter()
                .any(|c| is_valid(c) && input_matches(entry, &c["input"]))
            {
                let seen: Vec<String> = of_tool.iter().map(|c| c["input"].to_string()).collect();
                failures.push(format!("no valid matching {tool} call (saw {seen:?})"));
            }
        }
        for tool in forbidden(sample) {
            if made.iter().any(|c| c["tool"] == tool) {
                failures.push(format!("called forbidden {tool}"));
            }
        }
        if failures.is_empty() {
            Score::pass(
                "correct_call",
                "expected calls made with valid, matching arguments",
            )
        } else {
            Score::fail("correct_call", failures.join("; "))
        }
    })
}

fn forbidden(sample: &Sample) -> Vec<String> {
    sample
        .metadata
        .get("forbid_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// `first_try`: the first call to each expected tool already validated.
pub fn first_try() -> Box<dyn Scorer> {
    scorer("first_try", |sample: &Sample, t: &Transcript| {
        let Some(expect) = expectations(sample) else {
            return Score::na("first_try", "sample declares no expected calls");
        };
        let made = calls(t);
        let mut failures = Vec::new();
        for entry in expect {
            let tool = tool_of(entry);
            match made.iter().find(|c| c["tool"] == tool) {
                None => failures.push(format!("{tool} never called")),
                Some(call) if !is_valid(call) => {
                    failures.push(format!("first {tool} call rejected: {}", call["errors"]))
                }
                Some(_) => {}
            }
        }
        if failures.is_empty() {
            Score::pass("first_try", "first call to each expected tool was valid")
        } else {
            Score::fail("first_try", failures.join("; "))
        }
    })
}

/// Informational, never failing: calls the tools refused. The value is
/// `1 / (1 + n)` so it stays in Mira's 0..1 range; the count is the reason
/// and the `reveal.rejected_calls` metric.
pub fn rejected_calls() -> Box<dyn Scorer> {
    scorer("rejected_calls", |_sample: &Sample, t: &Transcript| {
        let n = calls(t).iter().filter(|c| !is_valid(c)).count();
        Score::graded(
            "rejected_calls",
            1.0 / (1.0 + n as f64),
            0.0,
            format!("{n} rejected"),
        )
    })
}

/// Informational, never failing: calls to a deferred tool before any
/// `tool_search` result had returned its schema.
pub fn calls_to_stub_before_search() -> Box<dyn Scorer> {
    scorer(
        "calls_to_stub_before_search",
        |_sample: &Sample, t: &Transcript| {
            let n = t.metric(STUB_BEFORE_SEARCH).unwrap_or(0.0);
            Score::graded(
                "calls_to_stub_before_search",
                1.0 / (1.0 + n),
                0.0,
                format!("{n} calls before a search returned the tool"),
            )
        },
    )
}

/// Walk the tool events in order and record the per-run metrics.
pub fn record_metrics(t: &mut Transcript) {
    let names = crate::registry::names();
    let mut found = std::collections::HashSet::new();
    let mut before = 0usize;
    let mut searches = 0usize;
    let mut fake = 0usize;
    for event in &t.events {
        if event.get("type").and_then(Value::as_str) != Some("tool.completed") {
            continue;
        }
        let tool = event
            .pointer("/data/tool_name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if tool == "tool_search" {
            searches += 1;
            let result = event.pointer("/data/result").map(Value::to_string);
            // A miss lists every name without a schema: that is not a reveal.
            if let Some(result) = result.filter(|r| !r.contains("No tools matched")) {
                for name in &names {
                    if result.contains(&format!("\\\"{name}\\\""))
                        || result.contains(&format!("\"{name}\""))
                    {
                        found.insert(*name);
                    }
                }
            }
        } else if names.contains(&tool) {
            fake += 1;
            if !found.contains(tool) {
                before += 1;
            }
        }
    }
    let rejected = calls(t).iter().filter(|c| !is_valid(c)).count();
    t.record_metric(REJECTED, rejected as f64);
    t.record_metric(STUB_BEFORE_SEARCH, before as f64);
    t.record_metric(FAKE_CALLS, fake as f64);
    t.record_metric(SEARCHES, searches as f64);
}

/// `input` is a recursive subset match (strings trimmed and case-insensitive,
/// numbers by value, arrays as multisets of equal length); `input_regex` maps a
/// JSON pointer to a pattern its string value must match.
pub fn input_matches(entry: &Value, input: &Value) -> bool {
    let subset_ok = entry
        .get("input")
        .is_none_or(|expected| matches_value(expected, input));
    let regex_ok = entry
        .get("input_regex")
        .and_then(Value::as_object)
        .is_none_or(|patterns| {
            patterns.iter().all(|(pointer, pattern)| {
                let Some(pattern) = pattern.as_str() else {
                    return false;
                };
                let Ok(re) = Regex::new(pattern) else {
                    return false;
                };
                input
                    .pointer(pointer)
                    .and_then(Value::as_str)
                    .is_some_and(|got| re.is_match(got))
            })
        });
    subset_ok && regex_ok
}

fn matches_value(want: &Value, got: &Value) -> bool {
    match (want, got) {
        (Value::Object(want), Value::Object(got)) => want
            .iter()
            .all(|(k, w)| got.get(k).is_some_and(|g| matches_value(w, g))),
        (Value::Array(want), Value::Array(got)) => {
            if want.len() != got.len() {
                return false;
            }
            let mut used = vec![false; got.len()];
            want.iter().all(|w| {
                let hit = got
                    .iter()
                    .enumerate()
                    .find(|(i, g)| !used[*i] && matches_value(w, g));
                match hit {
                    Some((i, _)) => {
                        used[i] = true;
                        true
                    }
                    None => false,
                }
            })
        }
        (Value::String(w), Value::String(g)) => w.trim().eq_ignore_ascii_case(g.trim()),
        (Value::Number(w), Value::Number(g)) => w.as_f64() == g.as_f64(),
        _ => want == got,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn subset_matching_is_recursive_and_order_free_for_arrays() {
        let entry = json!({"input": {"a": [{"x": 1}, {"x": 2}], "b": {"c": "EUR"}}});
        assert!(input_matches(
            &entry,
            &json!({"a": [{"x": 2, "y": 0}, {"x": 1.0}], "b": {"c": " eur ", "d": 1}, "e": 0})
        ));
        assert!(!input_matches(
            &entry,
            &json!({"a": [{"x": 1}], "b": {"c": "EUR"}})
        ));
        assert!(!input_matches(
            &entry,
            &json!({"a": [{"x": 1}, {"x": 3}], "b": {"c": "EUR"}})
        ));
        let regex = json!({"input_regex": {"/n/note": "(?i)okta"}});
        assert!(input_matches(
            &regex,
            &json!({"n": {"note": "All OKTA users"}})
        ));
        assert!(!input_matches(&regex, &json!({"n": {}})));
    }
}
