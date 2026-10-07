#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! [OpenAI's Decisions API](https://developers.openai.com/api/docs/guides/decisions)
//! as an Everruns decision driver.
//!
//! The Decisions API answers typed questions about text, on GPT-6 Luna. This
//! crate puts it behind `everruns_contracts::decision_driver::DecisionDriver`,
//! so it answers the same `DecisionRequest`s TypeSafe does and callers never
//! call OpenAI directly: guardrails, the `Decisions` facade, and capability
//! internals reach it through the deployment's decision router.
//!
//! Mapping, declared in [`OpenAIDecisionDriver::capabilities`]: all three
//! primitives are native. `Noul` is a `predicate`, `Choice` a `choice`, and
//! `Score` a `score`; every answer carries a measured probability or
//! distribution, so outcomes are calibrated. A request's questions go out in
//! one call under positional names (`q0`, `q1`, ...), so caller ids never
//! reach the vendor. A `refusal` answer fails the request rather than
//! inventing a default.

#![warn(missing_docs)]

pub mod wire;

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::decisions::{
    DecisionAnswer, DecisionOutcome, DecisionQuestion, DecisionRequest,
};
use everruns_contracts::error::{AgentLoopError, Result};
use serde_json::Value;

use self::wire::{
    AnswerBody, ChoiceBody, DecisionBody, DecisionResponse, LevelBody, QuestionBody, parse_response,
};
use everruns_contracts::decision_driver::{
    DecisionDriver, DecisionDriverCapabilities, NativePrimitives,
};

/// Driver id for `UTILITY_DECISION_DRIVER` and `openai/...` routing.
pub const OPENAI_DECISION_DRIVER_ID: &str = "openai";

/// Model asked when neither the request nor the driver names one; the only
/// model the endpoint serves today.
pub const DEFAULT_MODEL: &str = "gpt-6-luna";

/// OpenAI API root.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_BACKOFF: Duration = Duration::from_millis(250);
/// Upper bound on a honored `Retry-After`.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);
/// Questions per request. The endpoint has no documented cap and accepted 33
/// in probing; this keeps one call's latency and cost bounded.
const MAX_QUESTIONS: usize = 24;
/// Levels per score question, enforced by the endpoint.
const MAX_SCORE_LEVELS: usize = 10;

/// OpenAI's Decisions API, as a decision driver.
#[derive(Clone)]
pub struct OpenAIDecisionDriver {
    http: Option<reqwest::Client>,
    max_attempts: u32,
    backoff: Duration,
}

impl std::fmt::Debug for OpenAIDecisionDriver {
    /// Never renders the key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIDecisionDriver")
            .field("max_attempts", &self.max_attempts)
            .finish_non_exhaustive()
    }
}

impl OpenAIDecisionDriver {
    /// A driver over `api_key`, retrying a throttled or failed call up to
    /// three attempts in total.
    pub fn new() -> Self {
        // THREAT[TM-LLM-037]: the key is deployment- or application-owned and
        // never comes from a request, an agent, or a session.
        Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(DEFAULT_TIMEOUT)
                .build()
                .ok(),
            max_attempts: 3,
            backoff: DEFAULT_BACKOFF,
        }
    }

    /// Total attempts per request, including the first. 1 disables retry,
    /// which is what the deployment's guardrail path uses.
    pub fn max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts.max(1);
        self
    }

    /// Delay before the second attempt; doubles after that.
    pub fn backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    async fn decide(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
        body: &DecisionBody,
    ) -> Result<DecisionResponse> {
        let url = endpoint
            .url("decisions")
            .ok_or_else(|| AgentLoopError::llm("Decision endpoint required"))?;
        let bytes =
            serde_json::to_vec(body).map_err(|_| AgentLoopError::llm("Invalid decision body"))?;
        let mut attempt = 1;
        loop {
            let resolved = endpoint.resolve("POST", &url, &bytes).await?;
            let mut outgoing = self
                .http
                .as_ref()
                .ok_or_else(|| AgentLoopError::llm("Decision HTTP transport unavailable"))?
                .post(&resolved.url)
                .header("content-type", "application/json");
            for (name, value) in resolved.headers {
                outgoing = outgoing.header(name, value);
            }
            let sent = outgoing.body(bytes.clone()).send().await;
            let response = match sent {
                Ok(response) => response,
                Err(error) if attempt < self.max_attempts && error.is_timeout() => {
                    self.pause(attempt, None).await;
                    attempt += 1;
                    continue;
                }
                Err(_error) => {
                    return Err(AgentLoopError::llm("openai decision request failed"));
                }
            };
            let status = response.status();
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let mut response = response;
            let mut buffer = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| AgentLoopError::llm("Decision response transport failed"))?
            {
                if buffer.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 {
                    return Err(AgentLoopError::llm("Decision response exceeds size limit"));
                }
                buffer.extend_from_slice(&chunk);
            }
            let text = String::from_utf8(buffer)
                .map_err(|_| AgentLoopError::llm("Invalid decision response encoding"))?;

            if status.is_success() {
                let json: Value = serde_json::from_str(&text)
                    .map_err(|_| AgentLoopError::llm("openai decision response is not JSON"))?;
                return parse_response(&json).ok_or_else(|| {
                    AgentLoopError::llm("openai decision response is not a decision")
                });
            }
            let retryable = status.as_u16() == 429 || status.is_server_error();
            if retryable && attempt < self.max_attempts {
                self.pause(attempt, retry_after).await;
                attempt += 1;
                continue;
            }
            return Err(api_error(status.as_u16(), &text));
        }
    }

    async fn pause(&self, attempt: u32, retry_after: Option<Duration>) {
        let backoff = self.backoff * 2u32.saturating_pow(attempt - 1);
        tokio::time::sleep(retry_after.map_or(backoff, |after| after.min(MAX_RETRY_AFTER))).await;
    }
}

/// An API failure as an error that never echoes the request.
fn api_error(status: u16, _text: &str) -> AgentLoopError {
    AgentLoopError::llm(format!("OpenAI decision provider returned HTTP {status}"))
}

/// The vendor-side name of the question at `index`.
fn wire_name(index: usize) -> String {
    format!("q{index}")
}

fn to_body(model: &str, state: &Value, request: &DecisionRequest) -> DecisionBody {
    let input = match state {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let questions = request
        .questions
        .iter()
        .enumerate()
        .map(|(index, (_, question))| to_question(wire_name(index), question))
        .collect();
    DecisionBody {
        model: model.to_string(),
        input,
        questions,
    }
}

fn to_question(name: String, question: &DecisionQuestion) -> QuestionBody {
    match question {
        DecisionQuestion::Noul {
            instructions,
            yes,
            no,
        } => {
            // A predicate has no slots for what yes and no mean, so the
            // criteria ride in the instructions.
            let mut instructions = instructions.clone();
            if let Some(yes) = yes {
                instructions.push_str(&format!(" True when: {yes}."));
            }
            if let Some(no) = no {
                instructions.push_str(&format!(" False when: {no}."));
            }
            QuestionBody::Predicate { name, instructions }
        }
        DecisionQuestion::Choice {
            instructions,
            options,
        } => QuestionBody::Choice {
            name,
            instructions: instructions.clone(),
            choices: options
                .iter()
                .map(|(value, description)| ChoiceBody {
                    value: value.clone(),
                    description: description.clone(),
                })
                .collect(),
        },
        DecisionQuestion::Score {
            instructions,
            levels,
        } => QuestionBody::Score {
            name,
            instructions: instructions.clone(),
            levels: levels
                .iter()
                .map(|level| LevelBody {
                    label: level.clone(),
                    description: None,
                })
                .collect(),
        },
    }
}

/// The typed answer for `question` from the vendor's `answer`.
fn to_answer(question: &DecisionQuestion, answer: &AnswerBody) -> Result<DecisionAnswer> {
    let mismatch = || {
        AgentLoopError::llm(format!(
            "openai decision answered '{}' with the wrong answer type",
            answer.name()
        ))
    };
    match (question, answer) {
        (_, AnswerBody::Refusal { .. }) => Err(AgentLoopError::llm(
            "openai decision refused to answer a question",
        )),
        (DecisionQuestion::Noul { .. }, AnswerBody::Predicate { probability, .. }) => {
            Ok(DecisionAnswer::Noul {
                probability: probability.clamp(0.0, 1.0),
            })
        }
        (
            DecisionQuestion::Choice { options, .. },
            AnswerBody::Choice {
                choice,
                probabilities,
                confidence,
                ..
            },
        ) => {
            if !options.iter().any(|(value, _)| value == choice) {
                return Err(AgentLoopError::llm(format!(
                    "openai decision picked '{choice}', which is not an offered answer"
                )));
            }
            let probabilities: BTreeMap<String, f64> = options
                .iter()
                .map(|(value, _)| {
                    let p = probabilities
                        .iter()
                        .find(|entry| &entry.value == value)
                        .map_or(0.0, |entry| entry.probability);
                    (value.clone(), p)
                })
                .collect();
            Ok(DecisionAnswer::Choice {
                selected: choice.clone(),
                confidence: confidence
                    .unwrap_or_else(|| probabilities.values().copied().fold(0.0, f64::max)),
                probabilities,
            })
        }
        (
            DecisionQuestion::Score { levels, .. },
            AnswerBody::Score {
                score,
                probabilities,
                confidence,
                ..
            },
        ) => {
            let probabilities: BTreeMap<usize, f64> = (0..levels.len())
                .map(|index| {
                    let p = probabilities
                        .iter()
                        .find(|entry| entry.value == index)
                        .map_or(0.0, |entry| entry.probability);
                    (index, p)
                })
                .collect();
            Ok(DecisionAnswer::Score {
                score: *score,
                confidence: confidence
                    .unwrap_or_else(|| probabilities.values().copied().fold(0.0, f64::max)),
                probabilities,
            })
        }
        _ => Err(mismatch()),
    }
}

#[async_trait]
impl DecisionDriver for OpenAIDecisionDriver {
    fn id(&self) -> &str {
        OPENAI_DECISION_DRIVER_ID
    }

    fn capabilities(&self) -> DecisionDriverCapabilities {
        // Image input exists on the API, but the core state is JSON today, so
        // images are a follow-up.
        DecisionDriverCapabilities::new(NativePrimitives::ALL, true)
            .with_max_questions(MAX_QUESTIONS)
            .with_max_score_levels(MAX_SCORE_LEVELS)
    }

    async fn evaluate(
        &self,
        endpoint: &everruns_contracts::runtime_provider::ProviderEndpoint,
        request: DecisionRequest,
    ) -> Result<DecisionOutcome> {
        if request.is_empty() {
            return Err(AgentLoopError::llm(
                "decision request must carry at least one question",
            ));
        }
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let body = to_body(&model, &request.state, &request);
        let response = self.decide(endpoint, &body).await?;

        let mut outcome = DecisionOutcome {
            model: response.model.clone().unwrap_or(model),
            calibrated: true,
            attribution: None,
            ..DecisionOutcome::default()
        };
        outcome.usage.input_tokens = response.usage.input_tokens;
        outcome.usage.output_tokens = response.usage.output_tokens;
        for (index, (id, question)) in request.questions.iter().enumerate() {
            let name = wire_name(index);
            let answer = response
                .answers
                .iter()
                .find(|answer| answer.name() == name)
                .ok_or_else(|| AgentLoopError::llm("openai decision response missed a question"))?;
            outcome
                .answers
                .insert(id.clone(), to_answer(question, answer)?);
        }
        Ok(outcome)
    }
}

impl Default for OpenAIDecisionDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
