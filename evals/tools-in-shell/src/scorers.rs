//! Sample-aware scorers. Which tools ran, and with what input, comes from the
//! fake registry's call log, because a call from a script is not its own tool
//! event; how the model's shell calls were spent comes from the shared
//! friction classifier, so these numbers read like the platform study's.

use mira::scorer::scorer;
use mira::{Sample, Score, Scorer, Transcript};
use regex::Regex;
use serde_json::Value;

use crate::friction::{CallClass, Friction, Surface};
use crate::subject::CALLS_KEY;

/// The `tools` command, as the friction count reads it: help is `--help`, a
/// bare source, or `tools search`; a rejection is the builtin's own error
/// envelope for a name it does not have or input the tool's schema refused.
pub fn tools_surface() -> Surface {
    Surface {
        command: "tools",
        is_group: |path| crate::registry::SOURCES.contains(&path),
        discovery_verbs: &["search"],
        rejection_prefixes: &[],
        rejection_markers: &[
            "\"code\":\"unknown_command\"",
            "\"code\":\"invalid_input\"",
            "command not found",
        ],
    }
}

pub fn friction_of(t: &Transcript) -> Friction {
    crate::friction::analyze(&t.events, &tools_surface())
}

/// `friction.*` transcript metrics, as the platform study records them.
pub fn record_friction(t: &mut Transcript) {
    let friction = friction_of(t);
    if friction.is_empty() {
        return;
    }
    for (name, value) in friction.metrics() {
        t.record_metric(name, value);
    }
}

/// Informational: the share of `tools` calls that did real work, with the
/// breakdown as the reason. Never fails; budgets stay with `tool_budget`.
pub fn tools_friction() -> Box<dyn Scorer> {
    scorer("tools_friction", |_sample: &Sample, t: &Transcript| {
        let friction = friction_of(t);
        if friction.is_empty() {
            return Score::na("tools_friction", "no bash calls");
        }
        let cli = friction.calls.len() - friction.count(CallClass::Other);
        let share = if cli == 0 {
            1.0
        } else {
            friction.count(CallClass::Real) as f64 / cli as f64
        };
        Score::graded("tools_friction", share, 0.0, friction.summary())
    })
}

fn calls(t: &Transcript) -> Vec<Value> {
    t.metadata
        .get(CALLS_KEY)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `expect_calls`: each entry names a registry tool that must have run at
/// least once with matching input. `input` is a subset match (strings compare
/// trimmed and case-insensitively, arrays as sets); `input_regex` maps a field
/// to a pattern its string value must match; `max` caps how often it ran.
pub fn expected_calls() -> Box<dyn Scorer> {
    scorer("expected_calls", |sample: &Sample, t: &Transcript| {
        let Some(expect) = sample
            .metadata
            .get("expect_calls")
            .and_then(Value::as_array)
        else {
            return Score::na("expected_calls", "sample declares no expected calls");
        };
        let made = calls(t);
        let mut failures = Vec::new();
        for entry in expect {
            let tool = entry
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let of_tool: Vec<&Value> = made
                .iter()
                .filter(|call| call.get("tool").and_then(Value::as_str) == Some(tool))
                .collect();
            let matching = of_tool
                .iter()
                .filter(|call| input_matches(entry, &call["input"]))
                .count();
            if matching == 0 {
                let seen: Vec<String> = of_tool.iter().map(|c| c["input"].to_string()).collect();
                failures.push(format!("no matching {tool} call (saw inputs {seen:?})"));
            }
            if let Some(max) = entry.get("max").and_then(Value::as_u64)
                && of_tool.len() as u64 > max
            {
                failures.push(format!("{tool} ran {} times (max {max})", of_tool.len()));
            }
        }
        let summary: Vec<&str> = made
            .iter()
            .filter_map(|call| call.get("tool").and_then(Value::as_str))
            .collect();
        if failures.is_empty() {
            Score::pass("expected_calls", format!("ran {summary:?}"))
        } else {
            Score::fail(
                "expected_calls",
                format!("{}; ran {summary:?}", failures.join("; ")),
            )
        }
    })
}

/// `forbid_calls`: registry tools that must not have run.
pub fn forbidden_calls() -> Box<dyn Scorer> {
    scorer("forbidden_calls", |sample: &Sample, t: &Transcript| {
        let Some(forbid) = sample
            .metadata
            .get("forbid_calls")
            .and_then(Value::as_array)
        else {
            return Score::na("forbidden_calls", "sample declares no forbidden calls");
        };
        let hit: Vec<String> = calls(t)
            .iter()
            .filter_map(|call| call.get("tool").and_then(Value::as_str))
            .filter(|tool| forbid.iter().any(|f| f.as_str() == Some(tool)))
            .map(str::to_string)
            .collect();
        if hit.is_empty() {
            Score::pass("forbidden_calls", "no forbidden tool ran")
        } else {
            Score::fail("forbidden_calls", format!("ran forbidden tools {hit:?}"))
        }
    })
}

/// `expect_regex`: a pattern the final answer must match.
pub fn response_matches() -> Box<dyn Scorer> {
    scorer("response_matches", |sample: &Sample, t: &Transcript| {
        let Some(pattern) = sample.metadata.get("expect_regex").and_then(Value::as_str) else {
            return Score::na(
                "response_matches",
                "sample declares no response expectation",
            );
        };
        match Regex::new(pattern) {
            Ok(re) if re.is_match(&t.final_response) => {
                Score::pass("response_matches", format!("matched /{pattern}/"))
            }
            Ok(_) => Score::fail("response_matches", format!("did not match /{pattern}/")),
            Err(error) => Score::fail("response_matches", format!("invalid regex: {error}")),
        }
    })
}

/// `max_tool_calls`: a ceiling on the model's own tool calls (shell calls).
pub fn tool_budget() -> Box<dyn Scorer> {
    scorer("tool_budget", |sample: &Sample, t: &Transcript| {
        let Some(max) = sample
            .metadata
            .get("max_tool_calls")
            .and_then(Value::as_u64)
        else {
            return Score::na("tool_budget", "sample declares no tool budget");
        };
        if t.tool_calls_count as u64 <= max {
            Score::pass(
                "tool_budget",
                format!("{} tool calls <= {max}", t.tool_calls_count),
            )
        } else {
            Score::fail(
                "tool_budget",
                format!("{} tool calls (max {max})", t.tool_calls_count),
            )
        }
    })
}

fn input_matches(entry: &Value, input: &Value) -> bool {
    let subset_ok = entry
        .get("input")
        .and_then(Value::as_object)
        .is_none_or(|expected| {
            expected
                .iter()
                .all(|(key, want)| input.get(key).is_some_and(|got| loosely_equal(want, got)))
        });
    let regex_ok = entry
        .get("input_regex")
        .and_then(Value::as_object)
        .is_none_or(|patterns| {
            patterns.iter().all(|(key, pattern)| {
                let value = input.get(key).and_then(Value::as_str);
                match (pattern.as_str().map(Regex::new), value) {
                    (Some(Ok(re)), Some(value)) => re.is_match(value),
                    _ => false,
                }
            })
        });
    subset_ok && regex_ok
}

fn loosely_equal(want: &Value, got: &Value) -> bool {
    match (want, got) {
        (Value::String(a), Value::String(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().all(|x| b.iter().any(|y| loosely_equal(x, y)))
        }
        _ => want == got,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(metadata: Value) -> Sample {
        serde_json::from_value(json!({"id": "t", "input": ["t"], "metadata": metadata})).unwrap()
    }

    fn with_calls(made: Value) -> Transcript {
        let mut t = Transcript::default();
        t.metadata.insert(CALLS_KEY.into(), made);
        t
    }

    #[tokio::test]
    async fn expected_calls_matches_subsets_sets_and_patterns() {
        let s = sample(json!({"expect_calls": [{
            "tool": "mcp_tickets__create_ticket",
            "input": {"priority": "HIGH", "labels": ["safari", "checkout"]},
            "input_regex": {"title": "(?i)checkout fails"},
            "max": 1
        }]}));
        let good = with_calls(json!([{"tool": "mcp_tickets__create_ticket", "input": {
            "title": "Checkout fails on Safari", "priority": "high", "labels": ["checkout", "safari"]
        }}]));
        assert!(expected_calls().score(&s, &good).await.pass);

        // A label missing, or the ticket opened twice, fails.
        let partial = with_calls(json!([{"tool": "mcp_tickets__create_ticket", "input": {
            "title": "Checkout fails on Safari", "priority": "high", "labels": ["checkout"]
        }}]));
        assert!(!expected_calls().score(&s, &partial).await.pass);
        let twice = with_calls(json!([
            {"tool": "mcp_tickets__create_ticket", "input": {"title": "Checkout fails", "priority": "high", "labels": ["checkout", "safari"]}},
            {"tool": "mcp_tickets__create_ticket", "input": {"title": "Checkout fails", "priority": "high", "labels": ["checkout", "safari"]}}
        ]));
        assert!(!expected_calls().score(&s, &twice).await.pass);
    }

    #[tokio::test]
    async fn forbidden_calls_catches_the_similar_tool() {
        let s = sample(json!({"forbid_calls": ["mcp_tickets__search_tickets"]}));
        let wrong = with_calls(json!([{"tool": "mcp_tickets__search_tickets", "input": {}}]));
        assert!(!forbidden_calls().score(&s, &wrong).await.pass);
        let right = with_calls(json!([{"tool": "mcp_tickets__search_articles", "input": {}}]));
        assert!(forbidden_calls().score(&s, &right).await.pass);
    }

    /// The builtin's own outputs, so a change to its wording that the
    /// classifier stops recognising fails here instead of counting as real.
    #[test]
    fn friction_reads_the_builtins_envelopes() {
        let surface = tools_surface();
        let class = |script: &str, output: &str| {
            crate::friction::classify(script, output, &surface)
                .class
                .name()
        };
        assert_eq!(class("tools --help", "Sources:\n  crm ..."), "help");
        assert_eq!(class("tools crm", "crm tools:\n  find-contact ..."), "help");
        assert_eq!(
            class(
                "tools search password article",
                "tools tickets search-articles ..."
            ),
            "help"
        );
        assert_eq!(
            class(
                "tools tickets create-ticket --help",
                "create-ticket({ title: string })"
            ),
            "help"
        );
        assert_eq!(
            class(
                "tools crm get-contact '{\"query\":\"ada\"}'",
                "{\"error\":{\"code\":\"unknown_command\",\"message\":\"no tool `get-contact` on crm.\",\"retryable\":false}}"
            ),
            "rejected"
        );
        assert_eq!(
            class(
                "tools tickets create-ticket '{\"title\":\"x\",\"priority\":\"urgent\"}'",
                "{\"error\":{\"code\":\"invalid_input\",\"message\":\"priority: not one of low, normal, high\",\"retryable\":false}}"
            ),
            "rejected"
        );
        assert_eq!(
            class(
                "tools crm find-contact '{\"query\":\"ada\"}' | jq '.[0].id'",
                "\"ct_ada\""
            ),
            "real"
        );
        // A tool's own failure is not a refusal of the surface.
        assert_eq!(
            class(
                "tools weather-forecast '{\"city\":\"Lisbon\"}'",
                "{\"error\":{\"code\":\"tool_error\",\"message\":\"upstream\",\"retryable\":true}}"
            ),
            "real"
        );
    }
}
