//! Shared tool protocol and operation used by both execution contexts.

use everruns_capability::definition::schemars::{self, JsonSchema};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{Answer, Evaluation, Question, TypeSafeClient};

pub(crate) const TOOL_NAME: &str = "jev_evaluate";
pub(crate) const TOOL_DESCRIPTION: &str = "Ask TypeSafe's System One model typed questions about \
    some content and get calibrated numbers back: a probability for a yes/no question (noul), a \
    selected option with its full distribution (choice), or a position along ordered levels \
    (score). Use it to verify or rate something instead of judging it yourself — for example \
    whether a joke lands, whether an answer is supported by its source, or how severe a report is. \
    All questions in one call are answered together over the same content and cannot see each \
    other's answers, so ask everything you need at once.";

/// Hard cap on questions per call: the API charges per question, and a model
/// that asks fifty is not making a decision.
const MAX_QUESTIONS: usize = 20;
/// Hard cap on the content being judged.
const MAX_STATE_BYTES: usize = 32_768;
/// Hard cap on one question's instructions.
const MAX_INSTRUCTIONS_LEN: usize = 2_000;
/// Hard cap on options/levels for one question.
const MAX_CRITERIA: usize = 20;

/// Input to the `jev_evaluate` tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluateInput {
    /// The content to judge: text, or a JSON object/array.
    pub state: Value,
    /// The questions to ask about it.
    pub questions: Vec<InputQuestion>,
}

/// One question in an [`EvaluateInput`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputQuestion {
    /// Key the answer comes back under. Not sent to the model.
    pub id: String,
    /// `noul`, `choice`, or `score`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The judgment to make.
    pub instructions: String,
    /// What a yes means (`noul` only).
    #[serde(default)]
    pub yes: Option<String>,
    /// What a no means (`noul` only).
    #[serde(default)]
    pub no: Option<String>,
    /// Option name to its description (`choice` only). Descriptions may be null.
    #[serde(default)]
    pub options: Option<serde_json::Map<String, Value>>,
    /// Ordered level descriptions, lowest first (`score` only).
    #[serde(default)]
    pub levels: Option<Vec<String>>,
}

impl JsonSchema for EvaluateInput {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TypeSafeEvaluateInput".into()
    }
    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema().try_into().expect("evaluate schema is an object")
    }
}

pub(crate) fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "state": {
                "description": "The content to judge. A string for text, or an object/array for structured data such as a chat log or a record.",
                "type": ["string", "object", "array"]
            },
            "questions": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_QUESTIONS,
                "description": "One narrow judgment per question. Split independent dimensions into separate questions.",
                "items": {
                    "type": "object",
                    "required": ["id", "type", "instructions"],
                    "additionalProperties": false,
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Key the answer comes back under. Never shown to the model, so the instructions must carry the full meaning on their own."
                        },
                        "type": {
                            "type": "string",
                            "enum": ["noul", "choice", "score"],
                            "description": "noul: whether a condition holds, answered as the probability of yes. choice: exactly one option from `options`. score: a position along the ordered `levels`."
                        },
                        "instructions": {
                            "type": "string",
                            "maxLength": MAX_INSTRUCTIONS_LEN,
                            "description": "The judgment to make, e.g. 'Would a general audience laugh at this joke?'"
                        },
                        "yes": {"type": "string", "description": "What a yes means (noul only)."},
                        "no": {"type": "string", "description": "What a no means (noul only)."},
                        "options": {
                            "type": "object",
                            "description": "choice only: option name to a description of it (null when the name says enough). At least two.",
                            "additionalProperties": {"type": ["string", "null"]}
                        },
                        "levels": {
                            "type": "array",
                            "items": {"type": "string"},
                            "minItems": 2,
                            "maxItems": MAX_CRITERIA,
                            "description": "score only: ordered level descriptions, lowest first. Each must describe a concrete situation and stand on its own."
                        }
                    }
                }
            }
        },
        "required": ["state", "questions"],
        "additionalProperties": false
    })
}

/// Translate tool input into an [`Evaluation`], rejecting what the API would.
pub(crate) fn build_evaluation(input: EvaluateInput) -> Result<Evaluation, String> {
    if input.questions.is_empty() {
        return Err("questions must not be empty".to_string());
    }
    if input.questions.len() > MAX_QUESTIONS {
        return Err(format!(
            "too many questions: {} (limit {MAX_QUESTIONS})",
            input.questions.len()
        ));
    }
    let state_bytes = match &input.state {
        Value::String(text) => text.len(),
        other => other.to_string().len(),
    };
    if state_bytes > MAX_STATE_BYTES {
        return Err(format!(
            "state is {state_bytes} bytes, over the {MAX_STATE_BYTES}-byte limit; summarize or \
             select the relevant part first"
        ));
    }

    let mut evaluation = Evaluation::new(input.state);
    for question in input.questions {
        let id = question.id.trim().to_string();
        if id.is_empty() {
            return Err("every question needs a non-empty id".to_string());
        }
        if question.instructions.trim().is_empty() {
            return Err(format!("question '{id}' has empty instructions"));
        }
        if question.instructions.len() > MAX_INSTRUCTIONS_LEN {
            return Err(format!(
                "question '{id}' instructions exceed {MAX_INSTRUCTIONS_LEN} characters"
            ));
        }
        let built = match question.kind.as_str() {
            "noul" => {
                let mut built = Question::noul(question.instructions);
                if let (Some(yes), Some(no)) = (&question.yes, &question.no) {
                    built = built.criteria(yes, no);
                }
                built
            }
            "choice" => {
                let options = question.options.ok_or_else(|| {
                    format!("choice question '{id}' requires an `options` object")
                })?;
                if options.len() < 2 {
                    return Err(format!(
                        "choice question '{id}' needs at least two options, got {}",
                        options.len()
                    ));
                }
                if options.len() > MAX_CRITERIA {
                    return Err(format!(
                        "choice question '{id}' has {} options, over the {MAX_CRITERIA} limit",
                        options.len()
                    ));
                }
                Question::choice(
                    question.instructions,
                    options.into_iter().collect::<Vec<_>>(),
                )
            }
            "score" => {
                let levels = question
                    .levels
                    .ok_or_else(|| format!("score question '{id}' requires a `levels` array"))?;
                if levels.len() < 2 {
                    return Err(format!(
                        "score question '{id}' needs at least two levels, got {}",
                        levels.len()
                    ));
                }
                if levels.len() > MAX_CRITERIA {
                    return Err(format!(
                        "score question '{id}' has {} levels, over the {MAX_CRITERIA} limit",
                        levels.len()
                    ));
                }
                Question::score(question.instructions, levels)
            }
            other => {
                return Err(format!(
                    "question '{id}' has unknown type '{other}'; expected noul, choice, or score"
                ));
            }
        };
        evaluation = evaluation.ask(id, built);
    }
    Ok(evaluation)
}

/// Run one evaluation and render the answers as decision-ready JSON.
pub(crate) async fn evaluate(
    client: &TypeSafeClient,
    input: EvaluateInput,
) -> Result<Value, String> {
    let evaluation = build_evaluation(input)?;
    let judgment = client
        .evaluate(evaluation)
        .await
        .map_err(|error| error.to_string())?;

    let answers: serde_json::Map<String, Value> = judgment
        .answers
        .iter()
        .map(|(id, answer)| (id.clone(), render_answer(answer)))
        .collect();
    Ok(json!({
        "model": judgment.model,
        "answers": answers,
        "usage": {
            "input_tokens": judgment.usage.input_tokens,
            "output_tokens": judgment.usage.output_tokens,
        },
    }))
}

/// Render one answer, keeping the distribution the model actually returned.
///
/// The extra `normalized`/`level`/`label` fields on a score exist so a caller
/// can threshold without re-deriving the level count.
fn render_answer(answer: &Answer) -> Value {
    match answer {
        Answer::Noul(noul) => json!({
            "type": "noul",
            "probability_yes": noul.noul,
        }),
        Answer::Choice(choice) => json!({
            "type": "choice",
            "choice": choice.choice,
            "probabilities": choice.probabilities,
            "confidence": choice.confidence,
        }),
        Answer::Score(score) => json!({
            "type": "score",
            "score": score.score,
            "normalized": score.normalized(),
            "level": score.nearest_level(),
            "label": score.nearest_label(),
            "legend": score.legend,
            "probabilities": score.probabilities,
            "confidence": score.confidence,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(value: Value) -> EvaluateInput {
        serde_json::from_value(value).expect("valid input")
    }

    #[test]
    fn builds_all_three_primitives() {
        let evaluation = build_evaluation(input(json!({
            "state": "Why did the chicken cross the road?",
            "questions": [
                {"id": "funny", "type": "noul", "instructions": "Is it funny?",
                 "yes": "Would make someone laugh", "no": "Falls flat"},
                {"id": "kind", "type": "choice", "instructions": "What kind of joke?",
                 "options": {"pun": "Wordplay", "observational": null}},
                {"id": "quality", "type": "score", "instructions": "How good is it?",
                 "levels": ["Bad", "Fine", "Great"]}
            ]
        })))
        .expect("builds");
        assert_eq!(evaluation.len(), 3);
        assert_eq!(evaluation.questions["funny"].kind(), "noul");
        assert_eq!(evaluation.questions["kind"].kind(), "choice");
        assert_eq!(evaluation.questions["quality"].kind(), "score");
        let wire = serde_json::to_value(&evaluation).expect("serializes");
        assert_eq!(
            wire["questions"]["funny"]["criteria"]["true"],
            "Would make someone laugh"
        );
        assert_eq!(
            wire["questions"]["kind"]["criteria"]["observational"],
            Value::Null
        );
        assert_eq!(wire["model"], crate::client::DEFAULT_MODEL);
    }

    #[test]
    fn rejects_malformed_questions_before_spending_a_request() {
        let cases = [
            (json!({"state": "x", "questions": []}), "must not be empty"),
            (
                json!({"state": "x", "questions": [{"id": "", "type": "noul", "instructions": "q"}]}),
                "non-empty id",
            ),
            (
                json!({"state": "x", "questions": [{"id": "a", "type": "noul", "instructions": "  "}]}),
                "empty instructions",
            ),
            (
                json!({"state": "x", "questions": [{"id": "a", "type": "choice", "instructions": "q"}]}),
                "requires an `options`",
            ),
            (
                json!({"state": "x", "questions": [{"id": "a", "type": "choice", "instructions": "q", "options": {"only": null}}]}),
                "at least two options",
            ),
            (
                json!({"state": "x", "questions": [{"id": "a", "type": "score", "instructions": "q"}]}),
                "requires a `levels`",
            ),
            (
                json!({"state": "x", "questions": [{"id": "a", "type": "vibes", "instructions": "q"}]}),
                "unknown type",
            ),
        ];
        for (value, expected) in cases {
            let error = build_evaluation(input(value)).expect_err("must reject");
            assert!(
                error.contains(expected),
                "{error} should mention {expected}"
            );
        }
    }

    #[test]
    fn rejects_oversized_state_and_question_count() {
        let error = build_evaluation(input(json!({
            "state": "x".repeat(MAX_STATE_BYTES + 1),
            "questions": [{"id": "a", "type": "noul", "instructions": "q"}]
        })))
        .expect_err("must reject");
        assert!(error.contains("over the"), "{error}");

        let questions: Vec<Value> = (0..=MAX_QUESTIONS)
            .map(|i| json!({"id": format!("q{i}"), "type": "noul", "instructions": "q"}))
            .collect();
        let error = build_evaluation(input(json!({"state": "x", "questions": questions})))
            .expect_err("must reject");
        assert!(error.contains("too many questions"), "{error}");
    }

    #[test]
    fn score_answers_render_thresholdable_fields() {
        let answer: Answer = serde_json::from_value(json!({
            "type": "score",
            "score": 1.5,
            "legend": {"0": "Calm", "1": "Frustrated", "2": "Very angry"},
            "probabilities": {"0": 0.1, "1": 0.3, "2": 0.6},
            "confidence": 0.7
        }))
        .expect("answer");
        let rendered = render_answer(&answer);
        assert_eq!(rendered["level"], 2);
        assert_eq!(rendered["label"], "Very angry");
        assert_eq!(rendered["normalized"], 0.75);
    }

    #[test]
    fn structured_state_is_passed_through_unchanged() {
        let evaluation = build_evaluation(input(json!({
            "state": {"ticket": {"messages": [{"text": "hello"}]}},
            "questions": [{"id": "a", "type": "noul", "instructions": "Is it a greeting?"}]
        })))
        .expect("builds");
        assert_eq!(evaluation.state["ticket"]["messages"][0]["text"], "hello");
    }
}
