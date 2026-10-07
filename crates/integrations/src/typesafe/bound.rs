//! Tenant model execution reuses host credential, egress and usage boundaries.
use crate::typesafe::evaluate::{EvaluateInput, build_evaluation};
use everruns_contracts::runtime::connection_services::DecisionModelBinding;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::{
    DecisionAnswer, DecisionQuestion, DecisionRequest, EgressRequest, EgressRequestKind,
    EventRequest, LlmGenerationData, TokenUsage,
};
use futures::StreamExt;
use serde_json::{Value, json};

/// The request and response shape a decision provider speaks.
#[derive(Clone, Copy)]
enum Wire {
    SystemOne,
    OpenAi,
}

impl Wire {
    fn path(self) -> &'static str {
        match self {
            Self::SystemOne => "systemone",
            Self::OpenAi => "decisions",
        }
    }
}

pub async fn evaluate(
    binding: DecisionModelBinding,
    input: EvaluateInput,
    context: &ToolContext,
) -> Result<Value, String> {
    let mut request = decision_request(input, &binding.model)?;
    if let Some(checker) = &context.budget_checker {
        let budget = checker
            .check_budgets(&context.session_id.to_string())
            .await
            .map_err(|_| "Decision budget check failed")?;
        if matches!(budget.status.as_str(), "paused" | "exhausted") {
            return Err("Decision budget exhausted".into());
        }
    } else {
        return Err("Decision budget checking is unavailable".into());
    }
    let egress = context
        .egress_service
        .as_ref()
        .ok_or("Decision egress is unavailable")?;
    let emitter = context
        .event_emitter
        .as_ref()
        .ok_or("Decision usage tracking is unavailable")?;
    let event_context = context
        .event_context
        .clone()
        .ok_or("Decision event context is unavailable")?;
    // TypeSafe and OpenRouter speak System One; OpenAI has its own Decisions
    // API. Both go through the same egress, budget, and usage path below.
    let (provider, wire) = match binding.provider_type.as_str() {
        "typesafe" => (
            everruns_drivers::typesafe::provider(
                binding.provider_id.clone(),
                binding.api_key.clone(),
            ),
            Wire::SystemOne,
        ),
        "openrouter" => (
            everruns_drivers::openrouter::provider(
                binding.provider_id.clone(),
                binding.api_key.clone(),
            ),
            Wire::SystemOne,
        ),
        "openai" => (
            everruns_drivers::openai::provider(
                binding.provider_id.clone(),
                binding.api_key.clone(),
            ),
            Wire::OpenAi,
        ),
        _ => return Err("Unsupported decision provider".into()),
    };
    let provider = if let Some(url) = binding.base_url.clone() {
        provider.base_url(url)
    } else {
        provider
    };
    let mut provider = provider;
    for (name, value) in &binding.headers {
        provider = provider.header(name, value);
    }
    let requested_model = request.model.clone().unwrap_or_default();
    if binding.provider_type == "openrouter" {
        request.model =
            Some(everruns_drivers::systemone::openrouter_model(&requested_model).into());
    }
    let body = match wire {
        Wire::SystemOne => everruns_drivers::systemone::encode(&request),
        Wire::OpenAi => everruns_drivers::openai::decisions::encode(&request),
    }
    .map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(&body).map_err(|_| "Invalid decision input")?;
    let endpoint = provider.endpoint();
    let resolved = endpoint
        .resolve(
            "POST",
            endpoint.url(wire.path()).ok_or("Missing endpoint")?,
            &bytes,
        )
        .await
        .map_err(|_| "Decision authentication failed")?;
    let mut outgoing = EgressRequest::new("POST", resolved.url, EgressRequestKind::Provider)
        .body(bytes)
        .header("content-type", "application/json")
        .network_access(context.network_access.clone())
        .timeout_ms(10_000)
        .require_dns_pinning();
    for (name, value) in resolved.headers {
        outgoing.headers.insert(name.clone(), value.clone());
    }
    // THREAT[TM-LLM-047]: account and model are trusted config, never tool arguments.
    let response = egress.send_stream(outgoing).await;
    let body = match response {
        Ok(mut response) if (200..300).contains(&response.status) => {
            let mut body = Vec::new();
            while let Some(chunk) = response.body.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(_) => {
                        record_usage(
                            &binding,
                            &requested_model,
                            None,
                            false,
                            emitter,
                            context.session_id,
                            event_context,
                        )
                        .await?;
                        return Err("Decision response transport failed".into());
                    }
                };
                if body.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 {
                    record_usage(
                        &binding,
                        &requested_model,
                        None,
                        false,
                        emitter,
                        context.session_id,
                        event_context,
                    )
                    .await?;
                    return Err("Decision response exceeds size limit".into());
                }
                body.extend_from_slice(&chunk);
            }
            body
        }
        _ => {
            record_usage(
                &binding,
                &requested_model,
                None,
                false,
                emitter,
                context.session_id,
                event_context,
            )
            .await?;
            return Err("Decision provider request failed".into());
        }
    };
    let value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            record_usage(
                &binding,
                &requested_model,
                None,
                false,
                emitter,
                context.session_id,
                event_context,
            )
            .await?;
            return Err("Invalid decision JSON".into());
        }
    };
    let outcome = match wire {
        Wire::SystemOne => everruns_drivers::systemone::decode(&request, value.clone()),
        Wire::OpenAi => everruns_drivers::openai::decisions::decode(&request, &value),
    };
    // Record provider-reported usage even if answer validation rejects the outcome.
    record_usage(
        &binding,
        &requested_model,
        Some(&value),
        outcome.is_ok(),
        emitter,
        context.session_id,
        event_context,
    )
    .await?;
    let outcome = outcome.map_err(|_| "Invalid calibrated decision response")?;
    render_outcome(outcome, &request)
}

fn text(value: Value) -> String {
    match value {
        Value::String(text) => text,
        value => value.to_string(),
    }
}

pub(crate) fn decision_request(
    input: EvaluateInput,
    model: &str,
) -> Result<DecisionRequest, String> {
    let evaluation = build_evaluation(input)?;
    let mut request = DecisionRequest::new(evaluation.state).model(model);
    for (id, question) in evaluation.questions {
        let question = match question {
            crate::typesafe::client::Question::Noul {
                instructions,
                criteria,
            } => DecisionQuestion::Noul {
                instructions: text(instructions),
                yes: criteria.as_ref().and_then(|c| c.yes.clone()),
                no: criteria.and_then(|c| c.no),
            },
            crate::typesafe::client::Question::Choice {
                instructions,
                criteria,
            } => DecisionQuestion::Choice {
                instructions: text(instructions),
                options: criteria
                    .into_iter()
                    .map(|(id, description)| (id, description.map(text)))
                    .collect(),
            },
            crate::typesafe::client::Question::Score {
                instructions,
                criteria,
            } => DecisionQuestion::Score {
                instructions: text(instructions),
                levels: criteria.into_iter().map(text).collect(),
            },
        };
        request.questions.push((id, question));
    }
    Ok(request)
}
pub(crate) fn render_outcome(
    outcome: everruns_contracts::runtime::DecisionOutcome,
    request: &DecisionRequest,
) -> Result<Value, String> {
    let mut answers = serde_json::Map::new();
    for (id, answer) in outcome.answers {
        let value = match answer {
            DecisionAnswer::Noul { probability } => {
                json!({"type":"noul","probability_yes":probability})
            }
            DecisionAnswer::Choice {
                selected,
                probabilities,
                confidence,
            } => {
                json!({"type":"choice","choice":selected,"probabilities":probabilities,"confidence":confidence})
            }
            DecisionAnswer::Score {
                score,
                probabilities,
                confidence,
            } => {
                let levels = request
                    .questions
                    .iter()
                    .find_map(|(key, q)| match q {
                        DecisionQuestion::Score { levels, .. } if key == &id => Some(levels),
                        _ => None,
                    })
                    .filter(|levels| levels.len() >= 2)
                    .ok_or("Invalid score question")?;
                if !score.is_finite() || !(0.0..=(levels.len() - 1) as f64).contains(&score) {
                    return Err("Invalid score response".into());
                }
                let level = score.round() as usize;
                let label = levels.get(level).ok_or("Invalid score level")?;
                json!({"type":"score","score":score,"normalized":score/(levels.len()-1) as f64,"level":level,"label":label,"legend":levels.iter().enumerate().map(|(i,s)| (i.to_string(),s)).collect::<std::collections::BTreeMap<_,_>>(),"probabilities":probabilities,"confidence":confidence})
            }
        };
        answers.insert(id, value);
    }
    Ok(json!({"model":outcome.model,"answers":answers,"usage":outcome.usage}))
}

async fn record_usage(
    binding: &DecisionModelBinding,
    requested: &str,
    value: Option<&Value>,
    success: bool,
    emitter: &std::sync::Arc<dyn everruns_contracts::runtime::EventEmitter>,
    session_id: everruns_contracts::typed_id::SessionId,
    event_context: everruns_contracts::runtime::EventContext,
) -> Result<(), String> {
    let value = value.unwrap_or(&Value::Null);
    let input = value["usage"]["input_tokens"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok());
    let output = value["usage"]["output_tokens"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok());
    let actual = value["usage"]["cost"]
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0);
    let estimated = input.zip(output).and_then(|(input, output)| {
        everruns_contracts::model_profile_data::get_model_profile_by_key(&binding.profile_key)
            .and_then(|p| p.cost)
            .map(|c| (input as f64 * c.input + output as f64 * c.output) / 1_000_000.0)
    });
    let mut generation = LlmGenerationData::success(
        vec![],
        vec![],
        None,
        vec![],
        requested.into(),
        Some(binding.provider_id.clone()),
        Some(TokenUsage {
            input_tokens: input.unwrap_or_default(),
            output_tokens: output.unwrap_or_default(),
            actual_cost_usd: actual,
            estimated_cost_usd: estimated,
            ..Default::default()
        }),
        None,
        None,
    );
    generation.metadata.success = success;
    if !success {
        generation.metadata.error =
            Some("Decision provider failed or returned an invalid outcome".into());
    }
    generation.metadata.response_model = value["model"]
        .as_str()
        .filter(|s| s.len() <= 256)
        .map(str::to_owned);
    generation.metadata.response_id = value["id"]
        .as_str()
        .filter(|s| s.len() <= 256)
        .map(str::to_owned);
    if actual.is_none() && estimated.is_none() {
        generation
            .metadata
            .cost_components
            .push(everruns_contracts::runtime::LlmCostComponent {
                kind: "model_tokens".into(),
                name: requested.into(),
                quantity: None,
                cost_usd: None,
            });
    }
    let mut event = EventRequest::new(session_id, event_context, generation);
    event.metadata = Some(
        json!({"service":"decisions","model_id":binding.model_id,"profile_key":binding.profile_key,"serving_provider":value["provider"].as_str().filter(|s| s.len() <= 256)}),
    );
    emitter
        .emit(event)
        .await
        .map_err(|_| "Decision usage recording failed".to_owned())?;
    Ok(())
}

#[cfg(test)]
#[path = "bound_tests.rs"]
mod tests;
