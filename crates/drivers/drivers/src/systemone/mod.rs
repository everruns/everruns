//! System One protocol shared by TypeSafe and OpenRouter accounts.

pub mod client;

use async_trait::async_trait;
use everruns_contracts::ProviderEndpoint;
use everruns_contracts::decision_driver::{
    DecisionDriver, DecisionDriverCapabilities, NativePrimitives,
};
use everruns_contracts::decisions::{
    DecisionAnswer, DecisionAttribution, DecisionOutcome, DecisionQuestion, DecisionRequest,
    DecisionUsage,
};
use everruns_contracts::error::{AgentLoopError, Result};
use std::collections::BTreeMap;

/// Credential-free System One transport. Authentication is resolved per call.
#[derive(Clone, Debug)]
pub struct SystemOneDecisionDriver {
    http: Option<reqwest::Client>,
}

impl Default for SystemOneDecisionDriver {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .ok(),
        }
    }
}

#[async_trait]
impl DecisionDriver for SystemOneDecisionDriver {
    fn id(&self) -> &str {
        "systemone"
    }
    fn capabilities(&self) -> DecisionDriverCapabilities {
        DecisionDriverCapabilities::new(NativePrimitives::ALL, true)
            .with_max_questions(20)
            .with_max_state_bytes(128 * 1024)
            .with_max_choice_options(255)
            .with_max_score_levels(10)
    }
    async fn evaluate(
        &self,
        endpoint: &ProviderEndpoint,
        request: DecisionRequest,
    ) -> Result<DecisionOutcome> {
        let body = encode(&request)?;
        let bytes = serde_json::to_vec(&body).map_err(|_| invalid("Invalid decision request"))?;
        let url = endpoint
            .url("systemone")
            .ok_or_else(|| invalid("Decision provider endpoint is required"))?;
        let resolved = endpoint.resolve("POST", url, &bytes).await?;
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| invalid("Decision HTTP client unavailable"))?;
        let mut outgoing = http
            .post(&resolved.url)
            .header("content-type", "application/json");
        for (name, value) in &resolved.headers {
            outgoing = outgoing.header(name, value);
        }
        // THREAT[TM-LLM-047]: send exactly the signed body; never follow redirects carrying auth.
        let mut response = outgoing
            .body(bytes)
            .send()
            .await
            .map_err(|_| invalid("Decision provider transport failed"))?;
        if !response.status().is_success() {
            return Err(invalid(format!(
                "Decision provider returned HTTP {}",
                response.status().as_u16()
            )));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| invalid("Decision response transport failed"))?
        {
            if bytes.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 {
                return Err(invalid("Decision response exceeds size limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("Invalid decision response JSON"))?;
        decode(&request, value)
    }
}

fn invalid(message: impl Into<String>) -> AgentLoopError {
    AgentLoopError::llm(message)
}

/// Encode the same body for native HTTP and policy-controlled host egress.
pub fn encode(request: &DecisionRequest) -> Result<serde_json::Value> {
    serde_json::to_value(evaluation(request)?).map_err(|_| invalid("Invalid decision request"))
}

pub fn evaluation(request: &DecisionRequest) -> Result<client::Evaluation> {
    DecisionDriverCapabilities::new(NativePrimitives::ALL, true)
        .with_max_questions(20)
        .with_max_state_bytes(128 * 1024)
        .with_max_choice_options(255)
        .with_max_score_levels(10)
        .check("systemone", request)?;
    let model = request
        .model
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid("Decision model is required"))?;
    let mut evaluation = client::Evaluation::new(request.state.clone()).model(model);
    for (id, question) in &request.questions {
        if evaluation.questions.contains_key(id) {
            return Err(invalid("Duplicate decision question id"));
        }
        let question = match question {
            DecisionQuestion::Noul {
                instructions,
                yes,
                no,
            } => client::Question::Noul {
                instructions: instructions.clone().into(),
                criteria: Some(client::NoulCriteria {
                    yes: yes.clone(),
                    no: no.clone(),
                }),
            },
            DecisionQuestion::Choice {
                instructions,
                options,
            } => {
                let criteria: BTreeMap<_, _> = options
                    .iter()
                    .map(|(key, description)| {
                        (
                            key.clone(),
                            description.clone().map(serde_json::Value::String),
                        )
                    })
                    .collect();
                if criteria.len() != options.len() {
                    return Err(invalid("Duplicate decision option"));
                }
                client::Question::Choice {
                    instructions: instructions.clone().into(),
                    criteria,
                }
            }
            DecisionQuestion::Score {
                instructions,
                levels,
            } => client::Question::Score {
                instructions: instructions.clone().into(),
                criteria: levels.iter().cloned().map(Into::into).collect(),
            },
        };
        evaluation = evaluation.ask(id.clone(), question);
    }
    evaluation
        .validate()
        .map_err(|_| invalid("Invalid decision questions"))?;
    Ok(evaluation)
}

fn probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
fn distribution<'a>(values: impl Iterator<Item = &'a f64>) -> bool {
    let values: Vec<_> = values.copied().collect();
    values.iter().all(|v| probability(*v)) && (values.iter().sum::<f64>() - 1.0).abs() <= 0.001
}

/// Validate complete calibrated outcomes before exposing them to callers.
pub fn decode(request: &DecisionRequest, value: serde_json::Value) -> Result<DecisionOutcome> {
    let judgment: client::Judgment =
        serde_json::from_value(value.clone()).map_err(|_| invalid("Invalid decision response"))?;
    if judgment.model.is_empty() || judgment.answers.len() != request.questions.len() {
        return Err(invalid("Incomplete decision response"));
    }
    let mut answers = BTreeMap::new();
    for (id, question) in &request.questions {
        let answer = judgment
            .answers
            .get(id)
            .ok_or_else(|| invalid("Missing decision answer"))?;
        let wire = &value["answers"][id];
        let answer = match (question, answer) {
            (DecisionQuestion::Noul { .. }, client::Answer::Noul(answer))
                if probability(answer.noul) =>
            {
                DecisionAnswer::Noul {
                    probability: answer.noul,
                }
            }
            (DecisionQuestion::Choice { options, .. }, client::Answer::Choice(answer))
                if wire.get("choice").is_some()
                    && wire.get("confidence").is_some()
                    && probability(answer.confidence)
                    && options.iter().any(|(key, _)| key == &answer.choice)
                    && answer.probabilities.len() == options.len()
                    && options
                        .iter()
                        .all(|(key, _)| answer.probabilities.contains_key(key))
                    && distribution(answer.probabilities.values()) =>
            {
                DecisionAnswer::Choice {
                    selected: answer.choice.clone(),
                    probabilities: answer.probabilities.clone(),
                    confidence: answer.confidence,
                }
            }
            (DecisionQuestion::Score { levels, .. }, client::Answer::Score(answer))
                if wire.get("score").is_some()
                    && wire.get("confidence").is_some()
                    && levels.len() >= 2
                    && answer.score.is_finite()
                    && (0.0..=(levels.len() - 1) as f64).contains(&answer.score)
                    && probability(answer.confidence)
                    && answer.probabilities.len() == levels.len()
                    && (0..levels.len())
                        .all(|i| answer.probabilities.contains_key(&i.to_string()))
                    && distribution(answer.probabilities.values()) =>
            {
                DecisionAnswer::Score {
                    score: answer.score,
                    probabilities: (0..levels.len())
                        .map(|i| (i, answer.probabilities[&i.to_string()]))
                        .collect(),
                    confidence: answer.confidence,
                }
            }
            _ => {
                return Err(invalid(
                    "Malformed decision answer or probability distribution",
                ));
            }
        };
        answers.insert(id.clone(), answer);
    }
    let cost = value["usage"]["cost"].as_f64();
    if value["usage"]
        .get("cost")
        .is_some_and(|value| !value.is_null() && !value.is_number())
    {
        return Err(invalid("Invalid decision cost"));
    }
    if cost.is_some_and(|cost| !cost.is_finite() || cost < 0.0) {
        return Err(invalid("Invalid decision cost"));
    }
    Ok(DecisionOutcome {
        model: judgment.model,
        answers,
        usage: DecisionUsage {
            input_tokens: judgment.usage.input_tokens,
            output_tokens: judgment.usage.output_tokens,
        },
        calibrated: true,
        attribution: Some(DecisionAttribution {
            request_id: value["id"].as_str().map(str::to_owned),
            serving_provider: value["provider"].as_str().map(str::to_owned),
            cost_usd: cost,
        }),
    })
}

/// Explicit OpenRouter aliases, independent from provider selection.
/// Unrecognized IDs remain opaque, including snapshot IDs.
pub fn openrouter_model(model: &str) -> &str {
    match model {
        "jev-1.13.0" => "typesafe/jev-1.13",
        "jev-latest" => "typesafe/jev-latest",
        other => other,
    }
}

#[derive(Clone, Debug, Default)]
pub struct OpenRouterDecisionDriver(SystemOneDecisionDriver);
#[async_trait]
impl DecisionDriver for OpenRouterDecisionDriver {
    fn id(&self) -> &str {
        "openrouter-systemone"
    }
    fn capabilities(&self) -> DecisionDriverCapabilities {
        self.0.capabilities()
    }
    async fn evaluate(
        &self,
        endpoint: &ProviderEndpoint,
        mut request: DecisionRequest,
    ) -> Result<DecisionOutcome> {
        request.model = request
            .model
            .as_deref()
            .map(openrouter_model)
            .map(str::to_owned);
        self.0.evaluate(endpoint, request).await
    }
}

#[cfg(test)]
mod tests;
