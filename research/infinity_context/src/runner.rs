//! Evaluation runner - executes scenarios against capabilities
//!
//! Uses core's LLM driver infrastructure for provider-agnostic LLM calls.

use crate::eval_capability::{EvalCapability, PreparedContext};
use crate::metrics::{aggregate_strategy_results, check_success};
use crate::types::{EvalMetrics, EvaluationResults, MessageExt, Scenario, ScenarioResult, ToolCall};
use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use everruns_core::llm_driver_registry::{
    DriverRegistry, LlmCallConfig, LlmMessage, LlmMessageContent, LlmMessageRole, ProviderConfig,
    ProviderType,
};
use everruns_core::tool_types::{BuiltinTool, ToolDefinition as CoreToolDefinition};
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

/// Create a driver registry with Anthropic and OpenAI drivers registered
fn create_driver_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut registry);
    everruns_openai::register_driver(&mut registry);
    registry
}

/// Get provider type from model name
fn get_provider_type(model: &str) -> ProviderType {
    if model.starts_with("claude") {
        ProviderType::Anthropic
    } else {
        ProviderType::OpenAI
    }
}

/// Run evaluation across all scenarios and capabilities
pub async fn run_evaluation(
    config: &EvalConfig,
    scenarios: &[Scenario],
    capabilities: &[Arc<dyn EvalCapability>],
) -> Result<EvaluationResults> {
    let budget_tokens = (config.context_window as f64 * config.budget_percent) as usize;

    // Create driver registry once
    let driver_registry = create_driver_registry();

    println!(
        "\n{} Running {} scenario(s) against {} capability(ies)\n",
        "▶".bright_blue(),
        scenarios.len(),
        capabilities.len()
    );

    let mut strategy_results = Vec::new();

    for capability in capabilities {
        println!(
            "{} Capability: {}",
            "━".repeat(60).dimmed(),
            capability.name().bright_cyan().bold()
        );
        println!("  {}\n", capability.description().dimmed());

        let mut results = Vec::new();

        for scenario in scenarios {
            let result = run_single_scenario(
                config,
                scenario,
                capability.as_ref(),
                budget_tokens,
                &driver_registry,
            )
            .await;

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

            if !result.success
                && !result.metrics.context_exceeded
                && let Some(err) = &result.error
            {
                println!("    {} {}", "Error:".red(), err);
            }

            results.push(result);
        }

        let aggregated = aggregate_strategy_results(capability.name(), results);
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

/// Run a single scenario with a capability
async fn run_single_scenario(
    config: &EvalConfig,
    scenario: &Scenario,
    capability: &dyn EvalCapability,
    budget_tokens: usize,
    driver_registry: &DriverRegistry,
) -> ScenarioResult {
    let start = Instant::now();

    // Prepare context using the capability
    let prepared = capability.prepare_context(&scenario.messages, budget_tokens);

    // Check if we're likely to exceed context (for baseline)
    let will_exceed = prepared.estimated_tokens > config.context_window;

    if config.dry_run {
        return create_dry_run_result(scenario, capability, &prepared, will_exceed);
    }

    // Save context counts before moving prepared
    let messages_in_context = prepared.messages.len();
    let messages_excluded = prepared.excluded.len();

    // Execute LLM call(s)
    match execute_with_capability(config, scenario, capability, prepared, driver_registry).await {
        Ok((response, metrics)) => {
            let success = check_success(&response, &scenario.expected);
            ScenarioResult {
                scenario_name: scenario.name.clone(),
                strategy_name: capability.name().to_string(),
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
                strategy_name: capability.name().to_string(),
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

/// Execute LLM call(s) with tool handling for capability
async fn execute_with_capability(
    config: &EvalConfig,
    scenario: &Scenario,
    capability: &dyn EvalCapability,
    prepared: PreparedContext,
    driver_registry: &DriverRegistry,
) -> Result<(String, EvalMetrics)> {
    let mut metrics = EvalMetrics {
        messages_in_context: prepared.messages.len(),
        messages_excluded: prepared.excluded.len(),
        ..Default::default()
    };

    // Create the LLM driver
    let provider_type = get_provider_type(&config.model);
    let api_key = get_api_key(&provider_type)?;
    let provider_config = ProviderConfig::new(provider_type).with_api_key(&api_key);
    let driver = driver_registry.create_driver(&provider_config)?;

    // Build messages for the API
    let mut llm_messages = build_llm_messages(&prepared, scenario);

    // Convert tool definitions to core format
    let tools: Vec<CoreToolDefinition> = prepared
        .tools
        .iter()
        .map(|t| {
            CoreToolDefinition::Builtin(BuiltinTool {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
                policy: Default::default(),
            })
        })
        .collect();

    // Maximum tool call iterations to prevent infinite loops
    let max_iterations = 5;
    let mut final_response = String::new();

    for _iteration in 0..max_iterations {
        // Build LLM call config
        let llm_config = LlmCallConfig {
            model: config.model.clone(),
            temperature: Some(0.0),
            max_tokens: Some(4096),
            tools: tools.clone(),
            reasoning_effort: None,
            metadata: HashMap::new(),
        };

        // Call LLM using core's driver
        let response = driver.chat_completion(llm_messages.clone(), &llm_config).await?;

        metrics.llm_calls += 1;
        if let Some(prompt) = response.metadata.prompt_tokens {
            metrics.input_tokens += prompt as usize;
        }
        if let Some(completion) = response.metadata.completion_tokens {
            metrics.output_tokens += completion as usize;
        }
        metrics.total_tokens = metrics.input_tokens + metrics.output_tokens;

        // Check for tool calls
        if let Some(tool_calls) = response.tool_calls {
            if !capability.has_tools() || tool_calls.is_empty() {
                final_response = response.text;
                break;
            }

            // Handle tool calls
            for tool_call in tool_calls {
                metrics.history_queries += 1;

                // Convert core ToolCall to local ToolCall
                let local_tool_call = ToolCall {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    arguments: tool_call.arguments.clone(),
                };

                let tool_result = capability.execute_tool(&local_tool_call, &prepared.excluded)?;

                // Add assistant message with tool call
                llm_messages.push(LlmMessage {
                    role: LlmMessageRole::Assistant,
                    content: LlmMessageContent::Text(response.text.clone()),
                    tool_calls: Some(vec![tool_call.clone()]),
                    tool_call_id: None,
                });

                // Add tool result
                llm_messages.push(LlmMessage {
                    role: LlmMessageRole::Tool,
                    content: LlmMessageContent::Text(tool_result),
                    tool_calls: None,
                    tool_call_id: Some(tool_call.id),
                });
            }
        } else {
            // No tool calls - we have the final response
            final_response = response.text;
            break;
        }
    }

    Ok((final_response, metrics))
}

/// Get API key from environment
fn get_api_key(provider_type: &ProviderType) -> Result<String> {
    let key_name = match provider_type {
        ProviderType::Anthropic => "ANTHROPIC_API_KEY",
        ProviderType::OpenAI | ProviderType::OpenAICompletions => "OPENAI_API_KEY",
        ProviderType::LlmSim => return Ok(String::new()),
    };

    std::env::var(key_name).map_err(|_| anyhow::anyhow!("{} not set", key_name))
}

/// Build LLM messages from prepared context
fn build_llm_messages(prepared: &PreparedContext, scenario: &Scenario) -> Vec<LlmMessage> {
    let mut messages = Vec::new();

    // System message with any additions
    let mut system_content = "You are a helpful assistant. Answer questions based on the conversation history.".to_string();
    if let Some(ref addition) = prepared.system_addition {
        system_content.push_str("\n\n");
        system_content.push_str(addition);
    }

    messages.push(LlmMessage {
        role: LlmMessageRole::System,
        content: LlmMessageContent::Text(system_content),
        tool_calls: None,
        tool_call_id: None,
    });

    // Conversation messages
    for msg in &prepared.messages {
        let role = match msg.role {
            crate::types::MessageRole::User => LlmMessageRole::User,
            crate::types::MessageRole::Assistant => LlmMessageRole::Assistant,
            crate::types::MessageRole::ToolResult => LlmMessageRole::Tool,
            crate::types::MessageRole::System => LlmMessageRole::System,
        };

        messages.push(LlmMessage {
            role,
            content: LlmMessageContent::Text(msg.text_content()),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    // Add the task as final user message
    messages.push(LlmMessage {
        role: LlmMessageRole::User,
        content: LlmMessageContent::Text(scenario.task.clone()),
        tool_calls: None,
        tool_call_id: None,
    });

    messages
}

/// Create a result for dry run mode
fn create_dry_run_result(
    scenario: &Scenario,
    capability: &dyn EvalCapability,
    prepared: &PreparedContext,
    will_exceed: bool,
) -> ScenarioResult {
    ScenarioResult {
        scenario_name: scenario.name.clone(),
        strategy_name: capability.name().to_string(),
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
            messages_excluded: prepared.excluded.len(),
        },
        error: if will_exceed {
            Some("Context would exceed limit".to_string())
        } else {
            None
        },
    }
}
