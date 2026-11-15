//! Shared judge prompt and response parsing.
use anyhow::{Context, Result};
/// Parsed structured output from the judge model.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct JudgeOutput {
    /// Score 0.0–1.0.
    pub value: f64,
    /// Optional categorical label.
    #[serde(default)]
    pub label: Option<String>,
    /// Reasoning for the score.
    #[serde(default)]
    pub reasoning: String,
}

/// Result of a judge call: the parsed grade plus token/cost accounting.
#[derive(Debug, Clone)]
pub struct JudgeResult {
    pub value: f64,
    pub label: Option<String>,
    pub reasoning: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

/// What the judge grades: the user's question, the agent's answer, and a
/// summary of tools used in the turn.
#[derive(Debug, Clone, Default)]
pub struct TurnEvidence {
    pub input_message: String,
    pub final_answer: String,
    pub tool_names: Vec<String>,
}

const JUDGE_SYSTEM_PREAMBLE: &str = "You are an evaluation judge scoring an AI agent's response. \
Apply the rubric and respond with ONLY a JSON object of the form \
{\"value\": <number 0.0-1.0>, \"label\": <short string or null>, \"reasoning\": <string>}. \
`value` is the score where 1.0 fully satisfies the rubric and 0.0 fails it. \
`label` is an optional short category for the outcome. Do not include any text outside the JSON.";

/// Build (system, user) messages for the judge from the rubric and evidence.
pub fn build_judge_messages(rubric: &str, evidence: &TurnEvidence) -> (String, String) {
    let system = format!("{JUDGE_SYSTEM_PREAMBLE}\n\nRubric:\n{rubric}");

    let tools = if evidence.tool_names.is_empty() {
        "(none)".to_string()
    } else {
        evidence.tool_names.join(", ")
    };
    let user = format!(
        "User message:\n{}\n\nAgent's final answer:\n{}\n\nTools used: {}",
        truncate(&evidence.input_message, 4000),
        truncate(&evidence.final_answer, 8000),
        tools,
    );
    (system, user)
}

/// Parse the judge model's response into a [`JudgeOutput`]. Tolerates code
/// fences and surrounding prose by extracting the first balanced JSON object.
pub fn parse_judge_output(text: &str) -> Result<JudgeOutput> {
    let json_slice = extract_json_object(text)
        .ok_or_else(|| anyhow::anyhow!("judge response contained no JSON object"))?;
    let mut out: JudgeOutput = serde_json::from_str(json_slice)
        .context("judge response was not valid JudgeOutput JSON")?;
    // Clamp to the contract range so a misbehaving judge can't store nonsense.
    out.value = out.value.clamp(0.0, 1.0);
    out.label = out.label.filter(|l| !l.trim().is_empty());
    Ok(out)
}

/// Find the first balanced `{...}` object in `text` (handles ```json fences
/// and chatty models). Naive brace matching is sufficient for judge outputs.
fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '"' if !escaped => in_string = !in_string,
            '\\' if in_string => {
                escaped = !escaped;
                continue;
            }
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=start + i]);
                }
            }
            _ => {}
        }
        escaped = false;
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("…[truncated]");
    out
}
