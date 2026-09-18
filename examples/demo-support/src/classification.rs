//! Readable rendering for the classifier's tool payloads.
//!
//! `jev_evaluate` sends a list of typed questions and gets distributions back,
//! so both halves of the call are JSON a reader should not have to parse by eye.
//! [`questions`] and [`answers`] render them as aligned lines. Both return
//! `None` for anything not shaped like a classification, so every other tool
//! keeps the default preview.
//!
//! This is presentation only: the agent sees the full JSON either way.

use serde_json::{Map, Value};

/// Longest line these renderers print, chosen to fit the recorded terminal.
const WIDTH: usize = 96;
/// Characters kept from the content being judged.
const STATE_CHARS: usize = 220;

/// Render the questions an agent asked: the state it judged, then one line per
/// question with its id, primitive, and instructions.
///
/// Returns `None` unless the arguments carry a `questions` array of typed
/// questions, which is what makes the payload a classification.
pub fn questions(arguments: &Map<String, Value>) -> Option<String> {
    let questions = arguments.get("questions")?.as_array()?;
    if questions.is_empty() || !questions.iter().all(|question| question["id"].is_string()) {
        return None;
    }

    let mut lines = Vec::new();
    if let Some(state) = arguments.get("state") {
        let state = state
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| state.to_string());
        lines.push(format!("  state: {}", clip(&state, STATE_CHARS)));
    }
    let width = questions
        .iter()
        .filter_map(|question| question["id"].as_str())
        .map(str::len)
        .max()
        .unwrap_or(0);
    for question in questions {
        let id = question["id"].as_str().unwrap_or("?");
        let kind = question["type"].as_str().unwrap_or("?");
        let instructions = question["instructions"].as_str().unwrap_or("");
        let detail = match kind {
            // The options and levels are part of the question: a choice between
            // four teams is a different question from a choice between two.
            "choice" => question["options"]
                .as_object()
                .map(|options| format!(" [{}]", keys(options))),
            "score" => question["levels"]
                .as_array()
                .map(|levels| format!(" [{} levels]", levels.len())),
            _ => None,
        };
        let line = format!(
            "  {id:width$}  {kind:<6}  {instructions}{}",
            detail.unwrap_or_default()
        );
        lines.push(clip(&line, WIDTH));
    }
    Some(lines.join("\n"))
}

/// Render classification answers: a probability for a `noul`, the selected
/// option and its probability for a `choice`, and the position plus nearest
/// label for a `score`.
///
/// Returns `None` unless the result carries an `answers` object, so an ordinary
/// tool result falls through to the default preview.
pub fn answers(result: &Value) -> Option<String> {
    let answers = result.get("answers")?.as_object()?;
    if answers.is_empty() {
        return None;
    }

    let width = answers.keys().map(String::len).max().unwrap_or(0);
    let mut lines: Vec<String> = answers
        .iter()
        .map(|(id, answer)| {
            let kind = answer["type"].as_str().unwrap_or("?");
            let value = match kind {
                "noul" => format!("{:.2}", number(&answer["probability_yes"])),
                "choice" => {
                    let choice = answer["choice"].as_str().unwrap_or("?");
                    let probability = answer["probabilities"][choice].clone();
                    format!("{choice} ({:.2})", number(&probability))
                }
                "score" => {
                    // Levels are zero-based, so the top of a three-level score
                    // is 2: print the span the number lives on, not just the
                    // number.
                    let top = answer["legend"]
                        .as_object()
                        .map(|legend| legend.len().saturating_sub(1))
                        .unwrap_or_default();
                    format!(
                        "{:.2} of {top}  {}",
                        number(&answer["score"]),
                        answer["label"].as_str().unwrap_or("")
                    )
                }
                _ => answer.to_string(),
            };
            clip(&format!("  {id:width$}  {kind:<6}  {value}"), WIDTH)
        })
        .collect();
    if let Some(model) = result["model"].as_str() {
        lines.push(format!("  model: {model}"));
    }
    Some(lines.join("\n"))
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or(f64::NAN)
}

fn keys(options: &Map<String, Value>) -> String {
    options.keys().cloned().collect::<Vec<_>>().join(" | ")
}

fn clip(line: &str, limit: usize) -> String {
    if line.chars().count() <= limit {
        return line.to_owned();
    }
    format!("{}…", line.chars().take(limit - 1).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().expect("object").clone()
    }

    #[test]
    fn questions_render_one_line_each_with_their_option_sets() {
        let rendered = questions(&object(json!({
            "state": "Two hours on hold.",
            "questions": [
                {"id": "urgent", "type": "noul", "instructions": "Does this convey urgency?"},
                {"id": "queue", "type": "choice", "instructions": "Which team handles it?",
                 "options": {"billing": null, "technical": null}},
                {"id": "severity", "type": "score", "instructions": "How severe is it?",
                 "levels": ["Minor", "Real", "Serious"]}
            ]
        })))
        .expect("classification arguments");

        assert!(rendered.contains("state: Two hours on hold."));
        assert!(rendered.contains("urgent    noul    Does this convey urgency?"));
        assert!(rendered.contains("[billing | technical]"));
        assert!(rendered.contains("[3 levels]"));
    }

    #[test]
    fn answers_render_as_numbers_a_reader_can_check_against_a_threshold() {
        let rendered = answers(&json!({
            "model": "jev-1.13.0",
            "answers": {
                "urgent": {"type": "noul", "probability_yes": 0.88},
                "queue": {"type": "choice", "choice": "billing",
                          "probabilities": {"billing": 0.91, "technical": 0.09}},
                "severity": {"type": "score", "score": 1.34, "label": "Real",
                             "legend": {"0": "Minor", "1": "Real", "2": "Serious"}}
            }
        }))
        .expect("classification result");

        assert!(rendered.contains("urgent    noul    0.88"));
        assert!(rendered.contains("queue     choice  billing (0.91)"));
        assert!(rendered.contains("severity  score   1.34 of 2  Real"));
        assert!(rendered.contains("model: jev-1.13.0"));
    }

    #[test]
    fn other_payloads_fall_through_to_the_default_preview() {
        assert!(answers(&json!({"content": "a fetched page"})).is_none());
        assert!(questions(&object(json!({"customer_id": "cust_mfa"}))).is_none());
        // A tool whose own arguments happen to carry a `questions` list, but not
        // typed questions, is not a classification.
        assert!(questions(&object(json!({"questions": ["why?", "how?"]}))).is_none());
    }
}
