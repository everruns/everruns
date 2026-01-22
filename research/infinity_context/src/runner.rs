//! Evaluation runner - executes scenarios against strategies

use crate::metrics::{aggregate_strategy_results, check_success};
use crate::strategies::ContextStrategy;
use crate::types::{
    EvalMetrics, EvaluationResults, MessageExt, PreparedContext, Scenario, ScenarioResult, ToolCall,
};
use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Evaluation configuration
#[derive(Clone)]
pub struct EvalConfig {
    pub model: String,
    pub context_window: usize,
    pub budget_percent: f64,
    pub dry_run: bool,
}

/// Run evaluation across all scenarios and strategies
pub async fn run_evaluation(
    config: &EvalConfig,
    scenarios: &[Scenario],
    strategies: &[Arc<dyn ContextStrategy>],
) -> Result<EvaluationResults> {
    let budget_tokens = (config.context_window as f64 * config.budget_percent) as usize;

    println!(
        "\n{} Running {} scenario(s) against {} strategy(ies)\n",
        "▶".bright_blue(),
        scenarios.len(),
        strategies.len()
    );

    let mut strategy_results = Vec::new();

    for strategy in strategies {
        println!(
            "{} Strategy: {}",
            "━".repeat(60).dimmed(),
            strategy.name().bright_cyan().bold()
        );
        println!("  {}\n", strategy.description().dimmed());

        let mut results = Vec::new();

        for scenario in scenarios {
            let result =
                run_single_scenario(config, scenario, strategy.as_ref(), budget_tokens).await;

            // Print result
            let status = if result.success {
                "✓".bright_green()
            } else if result.metrics.context_exceeded {
                "⊘".bright_yellow()
            } else {
                "✗".bright_red()
            };

            println!(
                "  {} {} (tokens: {}, latency: {}ms, queries: {})",
                status,
                scenario.name,
                result.metrics.total_tokens,
                result.metrics.latency_ms,
                result.metrics.history_queries
            );

            if !result.success && !result.metrics.context_exceeded {
                if let Some(ref err) = result.error {
                    println!("    {} {}", "Error:".red(), err);
                }
            }

            results.push(result);
        }

        let aggregated = aggregate_strategy_results(strategy.name(), results);
        println!(
            "\n  {} Accuracy: {:.1}% ({}/{})\n",
            "→".bright_blue(),
            aggregated.accuracy(),
            aggregated.successful,
            aggregated.total_scenarios
        );

        strategy_results.push(aggregated);
    }

    Ok(EvaluationResults {
        timestamp: Utc::now(),
        model: config.model.clone(),
        config: HashMap::from([
            ("context_window".to_string(), config.context_window.to_string()),
            (
                "budget_percent".to_string(),
                config.budget_percent.to_string(),
            ),
        ]),
        strategy_results,
    })
}

/// Run a single scenario with a strategy
async fn run_single_scenario(
    config: &EvalConfig,
    scenario: &Scenario,
    strategy: &dyn ContextStrategy,
    budget_tokens: usize,
) -> ScenarioResult {
    let start = Instant::now();

    // Prepare context using the strategy
    let prepared = strategy.prepare_context(&scenario.messages, budget_tokens);

    // Check if we're likely to exceed context (for baseline)
    let will_exceed = prepared.estimated_tokens > config.context_window;

    if config.dry_run {
        return create_dry_run_result(scenario, strategy, &prepared, will_exceed);
    }

    // Save context counts before moving prepared
    let messages_in_context = prepared.messages.len();
    let messages_excluded = prepared.excluded_messages.len();

    // Execute LLM call(s)
    match execute_with_strategy(config, scenario, strategy, prepared).await {
        Ok((response, metrics)) => {
            let success = check_success(&response, &scenario.expected);
            ScenarioResult {
                scenario_name: scenario.name.clone(),
                strategy_name: strategy.name().to_string(),
                success,
                response,
                metrics: EvalMetrics {
                    latency_ms: start.elapsed().as_millis() as u64,
                    ..metrics
                },
                error: None,
            }
        }
        Err(e) => {
            let is_context_error = e.to_string().contains("context")
                || e.to_string().contains("too long")
                || e.to_string().contains("maximum");

            ScenarioResult {
                scenario_name: scenario.name.clone(),
                strategy_name: strategy.name().to_string(),
                success: false,
                response: String::new(),
                metrics: EvalMetrics {
                    context_exceeded: is_context_error,
                    messages_in_context,
                    messages_excluded,
                    latency_ms: start.elapsed().as_millis() as u64,
                    ..Default::default()
                },
                error: Some(e.to_string()),
            }
        }
    }
}

/// Execute LLM call(s) with tool handling for infinity strategy
async fn execute_with_strategy(
    config: &EvalConfig,
    scenario: &Scenario,
    strategy: &dyn ContextStrategy,
    mut prepared: PreparedContext,
) -> Result<(String, EvalMetrics)> {
    let client = Client::new();
    let mut metrics = EvalMetrics {
        messages_in_context: prepared.messages.len(),
        messages_excluded: prepared.excluded_messages.len(),
        ..Default::default()
    };

    // Build messages for the API
    let mut api_messages = build_api_messages(&prepared, scenario);

    // Maximum tool call iterations to prevent infinite loops
    let max_iterations = 5;
    let mut final_response = String::new();

    for iteration in 0..max_iterations {
        let response = call_llm(&client, config, &api_messages, &prepared.additional_tools).await?;

        metrics.llm_calls += 1;
        metrics.input_tokens += response.usage.input_tokens;
        metrics.output_tokens += response.usage.output_tokens;
        metrics.total_tokens = metrics.input_tokens + metrics.output_tokens;

        // Check for tool calls
        if let Some(tool_calls) = response.tool_calls {
            if !strategy.supports_tools() || tool_calls.is_empty() {
                final_response = response.content;
                break;
            }

            // Handle tool calls
            for tool_call in tool_calls {
                metrics.history_queries += 1;

                let tool_result =
                    strategy.handle_tool_call(&tool_call, &prepared.excluded_messages)?;

                // Add assistant message with tool call
                api_messages.push(ApiMessage {
                    role: "assistant".to_string(),
                    content: Some(response.content.clone()),
                    tool_calls: Some(vec![ApiToolCall {
                        id: tool_call.id.clone(),
                        r#type: "function".to_string(),
                        function: ApiFunction {
                            name: tool_call.name.clone(),
                            arguments: serde_json::to_string(&tool_call.arguments)?,
                        },
                    }]),
                    tool_call_id: None,
                    name: None,
                });

                // Add tool result
                api_messages.push(ApiMessage {
                    role: "tool".to_string(),
                    content: Some(tool_result),
                    tool_calls: None,
                    tool_call_id: Some(tool_call.id),
                    name: Some(tool_call.name),
                });
            }
        } else {
            // No tool calls - we have the final response
            final_response = response.content;
            break;
        }
    }

    Ok((final_response, metrics))
}

/// Build API messages from prepared context
fn build_api_messages(prepared: &PreparedContext, scenario: &Scenario) -> Vec<ApiMessage> {
    let mut messages = Vec::new();

    // System message with any additions
    let mut system_content = "You are a helpful assistant. Answer questions based on the conversation history.".to_string();
    for addition in &prepared.system_additions {
        system_content.push_str("\n\n");
        system_content.push_str(addition);
    }

    messages.push(ApiMessage {
        role: "system".to_string(),
        content: Some(system_content),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });

    // Conversation messages
    for msg in &prepared.messages {
        let role = match msg.role {
            crate::types::MessageRole::User => "user",
            crate::types::MessageRole::Assistant => "assistant",
            crate::types::MessageRole::ToolResult => "tool",
            crate::types::MessageRole::System => "system",
        };

        messages.push(ApiMessage {
            role: role.to_string(),
            content: Some(msg.text_content()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
    }

    // Add the task as final user message
    messages.push(ApiMessage {
        role: "user".to_string(),
        content: Some(scenario.task.clone()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });

    messages
}

/// Call the LLM API
async fn call_llm(
    client: &Client,
    config: &EvalConfig,
    messages: &[ApiMessage],
    tools: &[crate::types::ToolDefinition],
) -> Result<LlmResponse> {
    // Determine API based on model name
    let is_anthropic = config.model.starts_with("claude");

    if is_anthropic {
        call_anthropic(client, config, messages, tools).await
    } else {
        call_openai(client, config, messages, tools).await
    }
}

/// Call Anthropic API
async fn call_anthropic(
    client: &Client,
    config: &EvalConfig,
    messages: &[ApiMessage],
    tools: &[crate::types::ToolDefinition],
) -> Result<LlmResponse> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;

    // Convert messages to Anthropic format
    let system = messages
        .iter()
        .find(|m| m.role == "system")
        .and_then(|m| m.content.clone())
        .unwrap_or_default();

    let conversation_messages: Vec<_> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            json!({
                "role": m.role,
                "content": m.content
            })
        })
        .collect();

    let mut body = json!({
        "model": config.model,
        "max_tokens": 4096,
        "system": system,
        "messages": conversation_messages
    });

    // Add tools if any
    if !tools.is_empty() {
        let anthropic_tools: Vec<_> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters
                })
            })
            .collect();
        body["tools"] = json!(anthropic_tools);
    }

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = response.status();
    let text = response.text().await?;

    if !status.is_success() {
        return Err(anyhow::anyhow!("Anthropic API error {}: {}", status, text));
    }

    let response: serde_json::Value = serde_json::from_str(&text)?;

    // Extract content and tool calls
    let mut content = String::new();
    let mut tool_calls = Vec::new();

    if let Some(blocks) = response["content"].as_array() {
        for block in blocks {
            if block["type"] == "text" {
                content = block["text"].as_str().unwrap_or("").to_string();
            } else if block["type"] == "tool_use" {
                tool_calls.push(ToolCall {
                    id: block["id"].as_str().unwrap_or("").to_string(),
                    name: block["name"].as_str().unwrap_or("").to_string(),
                    arguments: block["input"].clone(),
                });
            }
        }
    }

    let usage = LlmUsage {
        input_tokens: response["usage"]["input_tokens"].as_u64().unwrap_or(0) as usize,
        output_tokens: response["usage"]["output_tokens"].as_u64().unwrap_or(0) as usize,
    };

    Ok(LlmResponse {
        content,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        usage,
    })
}

/// Call OpenAI API
async fn call_openai(
    client: &Client,
    config: &EvalConfig,
    messages: &[ApiMessage],
    tools: &[crate::types::ToolDefinition],
) -> Result<LlmResponse> {
    let api_key =
        std::env::var("OPENAI_API_KEY").map_err(|_| anyhow::anyhow!("OPENAI_API_KEY not set"))?;

    let mut body = json!({
        "model": config.model,
        "messages": messages
    });

    // Add tools if any
    if !tools.is_empty() {
        let openai_tools: Vec<_> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    }
                })
            })
            .collect();
        body["tools"] = json!(openai_tools);
    }

    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = response.status();
    let text = response.text().await?;

    if !status.is_success() {
        return Err(anyhow::anyhow!("OpenAI API error {}: {}", status, text));
    }

    let response: serde_json::Value = serde_json::from_str(&text)?;

    let choice = &response["choices"][0]["message"];
    let content = choice["content"].as_str().unwrap_or("").to_string();

    let tool_calls = choice["tool_calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .map(|c| ToolCall {
                    id: c["id"].as_str().unwrap_or("").to_string(),
                    name: c["function"]["name"].as_str().unwrap_or("").to_string(),
                    arguments: serde_json::from_str(
                        c["function"]["arguments"].as_str().unwrap_or("{}"),
                    )
                    .unwrap_or(json!({})),
                })
                .collect()
        })
        .filter(|v: &Vec<ToolCall>| !v.is_empty());

    let usage = LlmUsage {
        input_tokens: response["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as usize,
        output_tokens: response["usage"]["completion_tokens"].as_u64().unwrap_or(0) as usize,
    };

    Ok(LlmResponse {
        content,
        tool_calls,
        usage,
    })
}

/// Create a result for dry run mode
fn create_dry_run_result(
    scenario: &Scenario,
    strategy: &dyn ContextStrategy,
    prepared: &PreparedContext,
    will_exceed: bool,
) -> ScenarioResult {
    ScenarioResult {
        scenario_name: scenario.name.clone(),
        strategy_name: strategy.name().to_string(),
        success: !will_exceed, // Assume success if context fits
        response: "[DRY RUN]".to_string(),
        metrics: EvalMetrics {
            total_tokens: prepared.estimated_tokens,
            input_tokens: prepared.estimated_tokens,
            output_tokens: 0,
            llm_calls: 0,
            history_queries: 0,
            latency_ms: 0,
            context_exceeded: will_exceed,
            messages_in_context: prepared.messages.len(),
            messages_excluded: prepared.excluded_messages.len(),
        },
        error: if will_exceed {
            Some("Context would exceed limit".to_string())
        } else {
            None
        },
    }
}

// API message structures

#[derive(Debug, Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ApiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiToolCall {
    id: String,
    r#type: String,
    function: ApiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiFunction {
    name: String,
    arguments: String,
}

struct LlmResponse {
    content: String,
    tool_calls: Option<Vec<ToolCall>>,
    usage: LlmUsage,
}

struct LlmUsage {
    input_tokens: usize,
    output_tokens: usize,
}
