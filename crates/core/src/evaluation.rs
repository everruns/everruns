//! Shared deterministic evaluation rules.
use everruns_contracts::{
    runtime::evaluation::{Score, Scorer},
    runtime::message::{TextAnnotation, VerificationStatus},
};
use uuid::Uuid;
/// The trace slice a grader sees.
#[derive(Debug, Clone, Copy, Default)]
pub struct GradeInput<'a> {
    /// The user message being answered (the last one in the slice).
    pub input_message: &'a str,
    /// What the agent answered.
    pub final_answer: &'a str,
    /// Citation annotations on the final answer.
    pub annotations: &'a [TextAnnotation],
    /// Names of the tools called, in order.
    pub tool_calls: &'a [String],
    /// Argument JSON of each call in `tool_calls`, same order. Shorter (or
    /// empty) where arguments are not part of the trace; a call with no
    /// recorded arguments never matches `arguments_contain`.
    pub tool_arguments: &'a [String],
    /// The whole conversation as "User: ... / Agent: ..." text, for multistep
    /// cases. Empty for a single exchange; judges and decisions then see only
    /// `input_message` and `final_answer`.
    pub transcript: &'a str,
    /// Turns (eval case) or iterations (observer turn).
    pub turns: u32,
    /// Input plus output tokens the agent spent.
    pub tokens: u64,
    /// Session whose files `file_contains` reads. `None` where session files
    /// are not part of the trace (observers).
    pub session_id: Option<Uuid>,
}

fn binary(pass: bool, reason: String) -> Score {
    Score {
        pass,
        value: if pass { 1.0 } else { 0.0 },
        reason,
    }
}

/// Grade file contents after the host has resolved its own evidence or file store.
pub fn score_file_contains(path: &str, text: &str, content: Option<&str>) -> Score {
    let pass = content.is_some_and(|content| content.contains(text));
    binary(
        pass,
        if pass {
            format!("File '{path}' contains '{text}'")
        } else {
            format!("File '{path}' does not contain '{text}' or not found")
        },
    )
}
/// Grade the pure rules. `None` for scorers that need more than the trace.
pub fn score_rule(scorer: &Scorer, input: &GradeInput<'_>) -> Option<Score> {
    let answer = input.final_answer;
    let tool_calls = input.tool_calls;
    let score = match scorer {
        Scorer::Contains { text, .. } => {
            let pass = answer.contains(text.as_str());
            binary(
                pass,
                if pass {
                    format!("Output contains '{text}'")
                } else {
                    format!("Output does not contain '{text}'")
                },
            )
        }
        Scorer::NotContains { text, .. } => {
            let pass = !answer.contains(text.as_str());
            binary(
                pass,
                if pass {
                    format!("Output does not contain '{text}'")
                } else {
                    format!("Output contains '{text}'")
                },
            )
        }
        Scorer::Regex { pattern, .. } => match regex::Regex::new(pattern) {
            Ok(re) => {
                let pass = re.is_match(answer);
                binary(
                    pass,
                    if pass {
                        format!("Output matches pattern '{pattern}'")
                    } else {
                        format!("Output does not match pattern '{pattern}'")
                    },
                )
            }
            Err(error) => binary(false, format!("Invalid pattern '{pattern}': {error}")),
        },
        Scorer::ToolCalled {
            tool,
            min,
            arguments_contain,
            ..
        } => {
            let count = tool_calls
                .iter()
                .enumerate()
                .filter(|(_, t)| *t == tool)
                .filter(|(index, _)| match arguments_contain {
                    None => true,
                    Some(text) => input
                        .tool_arguments
                        .get(*index)
                        .is_some_and(|args| args.contains(text.as_str())),
                })
                .count() as u32;
            let with = arguments_contain
                .as_ref()
                .map(|text| format!(" with arguments containing '{text}'"))
                .unwrap_or_default();
            binary(
                count >= *min,
                format!("Tool '{tool}' called{with} {count} times (min: {min})"),
            )
        }
        Scorer::ToolNotCalled { tool, .. } => {
            let pass = !tool_calls.iter().any(|t| t == tool);
            binary(
                pass,
                if pass {
                    format!("Tool '{tool}' was not called")
                } else {
                    format!("Tool '{tool}' was called")
                },
            )
        }
        Scorer::ToolCallCount { min, max, .. } => {
            let count = tool_calls.len() as u32;
            let pass = min.is_none_or(|m| count >= m) && max.is_none_or(|m| count <= m);
            let bound = |v: &Option<u32>| v.map(|v| v.to_string()).unwrap_or("-".into());
            binary(
                pass,
                format!(
                    "Total tool calls: {count} (min: {}, max: {})",
                    bound(min),
                    bound(max)
                ),
            )
        }
        Scorer::TurnsWithin { max, .. } => binary(
            input.turns <= *max,
            format!("Turns: {} (max: {max})", input.turns),
        ),
        Scorer::TokensWithin { max, .. } => binary(
            input.tokens <= *max,
            format!("Tokens: {} (max: {max})", input.tokens),
        ),
        Scorer::JsonSchema { schema, .. } => score_json_schema(answer, schema),
        Scorer::FileContains { .. }
        | Scorer::CitationFaithful { .. }
        | Scorer::CitationJudged { .. }
        | Scorer::Judge { .. }
        | Scorer::Decision { .. } => return None,
    };
    Some(score)
}

/// The answer must parse as JSON (optionally inside a ```json fence) and
/// validate against `schema`.
pub fn score_json_schema(answer: &str, schema: &serde_json::Value) -> Score {
    let trimmed = answer.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return binary(false, "Output is not valid JSON".to_string());
    };
    let validator = match jsonschema::validator_for(schema) {
        Ok(validator) => validator,
        Err(error) => return binary(false, format!("Scorer schema is invalid: {error}")),
    };
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .take(3)
        .map(|error| format!("{} at '{}'", error, error.instance_path()))
        .collect();
    if errors.is_empty() {
        binary(true, "Output matches the JSON schema".to_string())
    } else {
        binary(
            false,
            format!(
                "Output does not match the JSON schema: {}",
                errors.join("; ")
            ),
        )
    }
}

/// Grade citation faithfulness from the answer's annotations: coverage (enough
/// citations) plus faithfulness (fraction verified `entailed`). Verdicts are
/// stamped by the `citation_verification` capability; without it, citations
/// are unverified and score 0, and the reason says so.
pub fn score_citation_faithful(
    annotations: &[TextAnnotation],
    min_citations: u32,
    pass_threshold: f64,
) -> Score {
    let total = annotations.len();
    if total < min_citations as usize {
        return binary(false, format!("{total} citations (min {min_citations})"));
    }
    if total == 0 {
        // min_citations is 0 here, so coverage is satisfied vacuously.
        return binary(true, "no citations required or present".to_string());
    }
    let with_verdict = annotations.iter().filter(|a| a.verified.is_some()).count();
    let entailed = annotations
        .iter()
        .filter(|a| {
            a.verified
                .as_ref()
                .is_some_and(|v| v.status == VerificationStatus::Entailed)
        })
        .count();
    let fraction = entailed as f64 / total as f64;
    let reason = if with_verdict == 0 {
        format!(
            "{entailed}/{total} citations entailed, none verified (enable citation_verification)"
        )
    } else {
        format!(
            "{entailed}/{total} citations entailed ({with_verdict} verified); \
             faithfulness {fraction:.2} (threshold {pass_threshold:.2})"
        )
    };
    Score {
        pass: fraction >= pass_threshold,
        value: fraction,
        reason,
    }
}

/// Default rubric for grading cited claim/source pairs.
pub const CITATION_JUDGE_RUBRIC: &str = "You are judging citation faithfulness. Each numbered item is a CLAIM the agent \
     made and the SOURCE it cited. Decide whether each SOURCE actually supports its \
     CLAIM. `value` is the fraction of items whose source supports the claim \
     (1.0 = all supported, 0.0 = none). Penalize claims whose source is unrelated \
     or contradicts them.";

/// Render each annotation as a numbered CLAIM/SOURCE pair for the judge. The
/// claim is the answer span the citation is attached to; the source is the
/// cited snippet (and uri).
pub fn build_citation_pairs(text: &str, annotations: &[TextAnnotation]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    for (i, ann) in annotations.iter().enumerate() {
        let claim: String = chars
            .get(ann.start..ann.end.min(chars.len()))
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        let source = ann.source.snippet.as_deref().unwrap_or("(no snippet)");
        out.push_str(&format!(
            "[{}] CLAIM: {}\n    SOURCE ({}): {}\n",
            i + 1,
            claim.trim(),
            ann.source.uri,
            source.trim(),
        ));
    }
    out
}
