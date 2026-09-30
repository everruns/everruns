#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! [OpenAI's Decisions API](https://openai.com/index/devday-2026-recap/) as an
//! Everruns decision driver (preview).
//!
//! The Decisions API picks one answer from a closed set, over text context, on
//! GPT-6 Luna. This crate puts it behind `everruns_core::DecisionDriver`, so
//! it answers the same `DecisionRequest`s TypeSafe does and callers never call
//! OpenAI directly: guardrails, the `Decisions` facade, and capability
//! internals reach it through the deployment's decision router.
//!
//! Mapping, declared in [`OpenAIDecisions::capabilities`]:
//!
//! - `Choice` is native.
//! - `Noul` is asked as a `yes`/`no` choice.
//! - `Score` is asked as a choice over the levels, labeled by index.
//! - The API answers one question per call, so a request's questions are sent
//!   concurrently and their usage summed; callers keep "one request, many
//!   questions".
//! - An answer is calibrated only when the response carries a probability for
//!   every label. Otherwise it is the label, one-hot, and the outcome says
//!   `calibrated: false`. The single confidence the API reports for its pick
//!   is not turned into a distribution: spreading the remainder across the
//!   other labels would be a number nobody measured.
//!
//! **The wire shape is provisional**; see [`wire`]. The deployment enables the
//! driver only on explicit opt-in (`DECISIONS_OPENAI_PREVIEW`), and the crate
//! stays unpublished until the shape is verified.

#![warn(missing_docs)]

pub mod wire;

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use everruns_core::{
    DecisionAnswer, DecisionDriver, DecisionDriverCapabilities, DecisionOutcome, DecisionQuestion,
    DecisionRequest, NativePrimitives,
};
use everruns_provider::error::{AgentLoopError, Result};
use futures::future::try_join_all;
use serde_json::Value;

use crate::wire::{DecisionBody, DecisionResponse, ErrorEnvelope, OptionBody, parse_response};

/// Driver id for `DECISIONS_DRIVER` and `openai/...` routing.
pub const OPENAI_DECISION_DRIVER_ID: &str = "openai";

/// Model asked when neither the request nor the driver names one. The API is
/// announced as built on GPT-6 Luna; the id it expects is unconfirmed.
pub const DEFAULT_MODEL: &str = "gpt-6-luna";

/// OpenAI API root.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_BACKOFF: Duration = Duration::from_millis(250);
/// Upper bound on a honored `Retry-After`.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);
/// Upper bound on the upstream message copied into an error.
const MAX_ERROR_MESSAGE: usize = 300;
/// Questions per request. Each is its own round trip, run concurrently.
const MAX_QUESTIONS: usize = 24;

/// OpenAI's Decisions API, as a decision driver.
#[derive(Clone)]
pub struct OpenAIDecisions {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    model: Option<String>,
    max_attempts: u32,
    backoff: Duration,
}

impl std::fmt::Debug for OpenAIDecisions {
    /// Never renders the key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIDecisions")
            .field("base_url", &self.base_url)
            .field("model", &self.model.as_deref().unwrap_or(DEFAULT_MODEL))
            .field("max_attempts", &self.max_attempts)
            .finish_non_exhaustive()
    }
}

impl OpenAIDecisions {
    /// A driver over `api_key`, retrying a throttled or failed call up to
    /// three attempts in total.
    pub fn new(api_key: impl Into<String>) -> Self {
        // THREAT[TM-LLM-037]: the key is deployment- or application-owned and
        // never comes from a request, an agent, or a session.
        Self {
            http: reqwest::Client::builder()
                .timeout(DEFAULT_TIMEOUT)
                .build()
                .unwrap_or_default(),
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: None,
            max_attempts: 3,
            backoff: DEFAULT_BACKOFF,
        }
    }

    /// Point at another API root: a proxy, or a test double.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    /// The model asked when a request names none.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Total attempts per question, including the first. 1 disables retry,
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

    async fn decide(&self, body: &DecisionBody) -> Result<DecisionResponse> {
        let url = format!("{}/decisions", self.base_url);
        let mut attempt = 1;
        loop {
            let sent = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(body)
                .send()
                .await;
            let response = match sent {
                Ok(response) => response,
                Err(error) if attempt < self.max_attempts && error.is_timeout() => {
                    self.pause(attempt, None).await;
                    attempt += 1;
                    continue;
                }
                Err(error) => {
                    // reqwest errors can carry the URL but never the headers.
                    return Err(AgentLoopError::llm(format!(
                        "openai decision request failed: {error}"
                    )));
                }
            };
            let status = response.status();
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let text = response.text().await.unwrap_or_default();

            if status.is_success() {
                let json: Value = serde_json::from_str(&text)
                    .map_err(|_| AgentLoopError::llm("openai decision response is not JSON"))?;
                return parse_response(&json).ok_or_else(|| {
                    AgentLoopError::llm("openai decision response has no chosen label")
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
fn api_error(status: u16, body: &str) -> AgentLoopError {
    let envelope: ErrorEnvelope = serde_json::from_str(body).unwrap_or_default();
    let mut message = envelope.error.message;
    if message.len() > MAX_ERROR_MESSAGE {
        let mut end = MAX_ERROR_MESSAGE;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push('…');
    }
    let kind = envelope.error.kind.unwrap_or_else(|| "error".to_string());
    AgentLoopError::llm(format!(
        "openai decision request failed ({status} {kind}): {message}"
    ))
}

/// The labels a question is asked over, and how each maps back.
fn to_body(model: &str, state: &Value, question: &DecisionQuestion) -> DecisionBody {
    let input = match state {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let (instructions, options) = match question {
        DecisionQuestion::Noul {
            instructions,
            yes,
            no,
        } => (
            instructions.clone(),
            vec![
                OptionBody {
                    label: "yes".into(),
                    description: yes.clone(),
                },
                OptionBody {
                    label: "no".into(),
                    description: no.clone(),
                },
            ],
        ),
        DecisionQuestion::Choice {
            instructions,
            options,
        } => (
            instructions.clone(),
            options
                .iter()
                .map(|(label, description)| OptionBody {
                    label: label.clone(),
                    description: description.clone(),
                })
                .collect(),
        ),
        DecisionQuestion::Score {
            instructions,
            levels,
        } => (
            format!("{instructions} Pick the level that fits best; levels run lowest first."),
            levels
                .iter()
                .enumerate()
                .map(|(index, level)| OptionBody {
                    label: index.to_string(),
                    description: Some(level.clone()),
                })
                .collect(),
        ),
    };
    DecisionBody {
        model: model.to_string(),
        input,
        instructions,
        options,
    }
}

/// The typed answer, and whether it came from a full distribution.
fn to_answer(
    question: &DecisionQuestion,
    body: &DecisionBody,
    response: &DecisionResponse,
) -> Result<(DecisionAnswer, bool)> {
    let labels: Vec<&str> = body.options.iter().map(|o| o.label.as_str()).collect();
    let picked = response.label.trim();
    if !labels.contains(&picked) {
        return Err(AgentLoopError::llm(format!(
            "openai decision picked '{picked}', which is not an offered answer"
        )));
    }
    // Calibrated only when every offered label has a probability.
    let distribution: Option<BTreeMap<&str, f64>> = response.probabilities.as_ref().and_then(|p| {
        labels
            .iter()
            .map(|label| p.get(*label).map(|value| (*label, *value)))
            .collect()
    });
    let calibrated = distribution.is_some();
    let answer = match (question, distribution) {
        (DecisionQuestion::Noul { .. }, Some(dist)) => DecisionAnswer::Noul {
            probability: dist["yes"],
        },
        (DecisionQuestion::Noul { .. }, None) => DecisionAnswer::noul_label(picked == "yes"),
        (DecisionQuestion::Choice { .. }, Some(dist)) => DecisionAnswer::Choice {
            selected: picked.to_string(),
            confidence: dist.values().copied().fold(0.0, f64::max),
            probabilities: dist
                .into_iter()
                .map(|(label, p)| (label.to_string(), p))
                .collect(),
        },
        (DecisionQuestion::Choice { .. }, None) => {
            DecisionAnswer::choice_label(picked, labels.iter().copied())
        }
        (DecisionQuestion::Score { .. }, Some(dist)) => {
            let probabilities: BTreeMap<usize, f64> = dist
                .into_iter()
                .filter_map(|(label, p)| label.parse().ok().map(|index| (index, p)))
                .collect();
            DecisionAnswer::Score {
                score: probabilities
                    .iter()
                    .map(|(index, p)| *index as f64 * p)
                    .sum(),
                confidence: probabilities.values().copied().fold(0.0, f64::max),
                probabilities,
            }
        }
        (DecisionQuestion::Score { levels, .. }, None) => {
            DecisionAnswer::score_label(picked.parse().unwrap_or_default(), levels.len())
        }
    };
    Ok((answer, calibrated))
}

#[async_trait]
impl DecisionDriver for OpenAIDecisions {
    fn id(&self) -> &str {
        OPENAI_DECISION_DRIVER_ID
    }

    fn capabilities(&self) -> DecisionDriverCapabilities {
        // Calibration is per response; the declaration is the one that
        // cannot overstate. Image context is announced but the core state is
        // JSON today, so images are a follow-up.
        DecisionDriverCapabilities::new(NativePrimitives::CHOICE_ONLY, false)
            .with_max_questions(MAX_QUESTIONS)
    }

    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionOutcome> {
        if request.is_empty() {
            return Err(AgentLoopError::llm(
                "decision request must carry at least one question",
            ));
        }
        let model = request
            .model
            .clone()
            .or_else(|| self.model.clone())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let bodies: Vec<DecisionBody> = request
            .questions
            .iter()
            .map(|(_, question)| to_body(&model, &request.state, question))
            .collect();
        let responses = try_join_all(bodies.iter().map(|body| self.decide(body))).await?;

        let mut outcome = DecisionOutcome {
            model: model.clone(),
            calibrated: true,
            ..DecisionOutcome::default()
        };
        for (((id, question), body), response) in
            request.questions.iter().zip(&bodies).zip(&responses)
        {
            let (answer, calibrated) = to_answer(question, body, response)?;
            outcome.calibrated &= calibrated;
            outcome.answers.insert(id.clone(), answer);
            outcome.usage.input_tokens += response.usage.input_tokens;
            outcome.usage.output_tokens += response.usage.output_tokens;
            if let Some(served) = &response.model {
                outcome.model = served.clone();
            }
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests;
