// Pure scorer-rule evaluation shared by eval runs and observers
// (knowledge/evaluation/online-evals.md). Rules that need session context beyond the
// trace (file_contains reads the session filesystem) return None and are
// handled by the caller.

use everruns_platform::eval::{Score, Scorer};

/// Evaluate a scorer rule against extracted trace data.
///
/// `final_content` is the final assistant message text, `tool_calls` the
/// tool names invoked, and `turns` the turn count (eval runs) or iteration
/// count (observer turn scope). Returns None for rules that need more than
/// trace data (`file_contains`).
pub fn score_rule(
    scorer: &Scorer,
    final_content: &str,
    tool_calls: &[String],
    turns: u32,
) -> Option<Score> {
    let score = match scorer {
        Scorer::Contains { text, .. } => {
            let pass = final_content.contains(text.as_str());
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: if pass {
                    format!("Output contains '{text}'")
                } else {
                    format!("Output does not contain '{text}'")
                },
            }
        }
        Scorer::NotContains { text, .. } => {
            let pass = !final_content.contains(text.as_str());
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: if pass {
                    format!("Output does not contain '{text}'")
                } else {
                    format!("Output contains '{text}'")
                },
            }
        }
        Scorer::Regex { pattern, .. } => {
            let pass = regex::Regex::new(pattern)
                .map(|re| re.is_match(final_content))
                .unwrap_or(false);
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: if pass {
                    format!("Output matches pattern '{pattern}'")
                } else {
                    format!("Output does not match pattern '{pattern}'")
                },
            }
        }
        Scorer::ToolCalled { tool, min, .. } => {
            let count = tool_calls.iter().filter(|t| t == &tool).count() as u32;
            let pass = count >= *min;
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: format!("Tool '{tool}' called {count} times (min: {min})"),
            }
        }
        Scorer::ToolNotCalled { tool, .. } => {
            let called = tool_calls.iter().any(|t| t == tool);
            let pass = !called;
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: if pass {
                    format!("Tool '{tool}' was not called")
                } else {
                    format!("Tool '{tool}' was called")
                },
            }
        }
        Scorer::ToolCallCount { min, max, .. } => {
            let count = tool_calls.len() as u32;
            let pass_min = min.map(|m| count >= m).unwrap_or(true);
            let pass_max = max.map(|m| count <= m).unwrap_or(true);
            let pass = pass_min && pass_max;
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: format!(
                    "Total tool calls: {count} (min: {}, max: {})",
                    min.map(|v| v.to_string()).unwrap_or("-".into()),
                    max.map(|v| v.to_string()).unwrap_or("-".into())
                ),
            }
        }
        Scorer::TurnsWithin { max, .. } => {
            let pass = turns <= *max;
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: format!("Turns: {turns} (max: {max})"),
            }
        }
        Scorer::JsonSchema { schema: _, .. } => {
            // JSON schema validation requires a jsonschema crate dependency.
            // For now, verify the output is valid JSON.
            let is_json = serde_json::from_str::<serde_json::Value>(final_content).is_ok();
            Score {
                pass: is_json,
                value: if is_json { 1.0 } else { 0.0 },
                reason: if is_json {
                    "Output is valid JSON (full schema validation not yet implemented)".to_string()
                } else {
                    "Output is not valid JSON".to_string()
                },
            }
        }
        Scorer::FileContains { .. } => return None,
        // Needs the message's citation annotations, not just trace text; graded
        // in the eval runner's async path.
        Scorer::CitationFaithful { .. } => return None,
        // Needs annotations + an LLM judge; graded in the eval runner.
        Scorer::CitationJudged { .. } => return None,
    };
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_rule() {
        let scorer = Scorer::Contains {
            text: "hello".into(),
            weight: 1.0,
        };
        assert!(score_rule(&scorer, "say hello world", &[], 1).unwrap().pass);
        assert!(!score_rule(&scorer, "goodbye", &[], 1).unwrap().pass);
    }

    #[test]
    fn tool_called_rule() {
        let scorer = Scorer::ToolCalled {
            tool: "web_fetch".into(),
            min: 2,
            weight: 1.0,
        };
        let calls = vec!["web_fetch".to_string(), "web_fetch".to_string()];
        assert!(score_rule(&scorer, "", &calls, 1).unwrap().pass);
        assert!(!score_rule(&scorer, "", &calls[..1], 1).unwrap().pass);
    }

    #[test]
    fn turns_within_rule() {
        let scorer = Scorer::TurnsWithin {
            max: 3,
            weight: 1.0,
        };
        assert!(score_rule(&scorer, "", &[], 3).unwrap().pass);
        assert!(!score_rule(&scorer, "", &[], 4).unwrap().pass);
    }

    #[test]
    fn file_contains_needs_session_context() {
        let scorer = Scorer::FileContains {
            path: "/x".into(),
            text: "y".into(),
            weight: 1.0,
        };
        assert!(score_rule(&scorer, "", &[], 1).is_none());
    }
}
