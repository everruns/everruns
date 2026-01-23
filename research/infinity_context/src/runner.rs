//! Evaluation runner - executes scenarios against capabilities
//!
//! Uses core's LLM driver and Capability infrastructure directly.

use crate::capabilities::{BatchTransformResult, Capability, MessageFilter, MessageFilterProvider, MessageQuery};
use crate::metrics::{aggregate_strategy_results, check_success};
use crate::types::{ContextStrategyConfig, EvalMetrics, EvaluationResults, Message, MessageExt, Scenario, ScenarioResult};
use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use everruns_core::llm_driver_registry::{
    DriverRegistry, LlmCallConfig, LlmMessage, LlmMessageContent, LlmMessageRole, ProviderConfig,
    ProviderType,
};
use everruns_core::tool_types::{BuiltinTool, ToolDefinition as CoreToolDefinition};
use everruns_core::tools::Tool;
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
    capabilities: &[Arc<dyn Capability>],
) -> Result<EvaluationResults> {
    let budget_tokens = (config.context_window as f64 * config.budget_percent) as usize;
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
            ("budget_percent".to_string(), config.budget_percent.to_string()),
        ]),
        strategy_results,
    })
}

/// Prepared context after filtering
struct PreparedContext {
    messages: Vec<Message>,
    excluded: Vec<Message>,
    system_addition: Option<String>,
    tools: Vec<CoreToolDefinition>,
    estimated_tokens: usize,
}

/// Prepare context using capability's message filter
fn prepare_context(
    capability: &dyn Capability,
    messages: &[Message],
    budget_tokens: usize,
) -> PreparedContext {
    let filter_provider = capability.message_filter_provider();

    // If no filter provider, pass all messages through (baseline)
    let Some(provider) = filter_provider else {
        let estimated_tokens: usize = messages.iter().map(|m| m.estimated_tokens()).sum();
        return PreparedContext {
            messages: messages.to_vec(),
            excluded: vec![],
            system_addition: capability.system_prompt_addition().map(String::from),
            tools: capability_tools(capability),
            estimated_tokens,
        };
    };

    // Build config for the filter
    let config = serde_json::to_value(ContextStrategyConfig {
        context_budget_tokens: budget_tokens,
        ..Default::default()
    })
    .unwrap_or_default();

    // Build and apply filters
    let mut query = MessageQuery::default();
    provider.apply_filters(&mut query, &config);

    // Execute batch transform filters to get kept/excluded
    let mut current_messages = messages.to_vec();
    let mut excluded = Vec::new();

    for filter in &query.filters {
        if let MessageFilter::BatchTransform(transform) = filter {
            let result = transform(current_messages);
            current_messages = result.kept;
            excluded.extend(result.excluded);
        }
    }

    let estimated_tokens: usize = current_messages.iter().map(|m| m.estimated_tokens()).sum();

    PreparedContext {
        messages: current_messages,
        excluded,
        system_addition: capability.system_prompt_addition().map(String::from),
        tools: capability_tools(capability),
        estimated_tokens,
    }
}

/// Get tool definitions from capability
fn capability_tools(capability: &dyn Capability) -> Vec<CoreToolDefinition> {
    capability
        .tools()
        .iter()
        .map(|t| {
            CoreToolDefinition::Builtin(BuiltinTool {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
                policy: Default::default(),
            })
        })
        .collect()
}

/// Run a single scenario with a capability
async fn run_single_scenario(
    config: &EvalConfig,
    scenario: &Scenario,
    capability: &dyn Capability,
    budget_tokens: usize,
    driver_registry: &DriverRegistry,
) -> ScenarioResult {
    let start = Instant::now();
    let prepared = prepare_context(capability, &scenario.messages, budget_tokens);
    let will_exceed = prepared.estimated_tokens > config.context_window;

    if config.dry_run {
        return create_dry_run_result(scenario, capability, &prepared, will_exceed);
    }

    let messages_in_context = prepared.messages.len();
    let messages_excluded = prepared.excluded.len();

    match execute_llm_calls(config, scenario, capability, prepared, driver_registry).await {
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

/// Execute LLM calls with tool handling
async fn execute_llm_calls(
    config: &EvalConfig,
    scenario: &Scenario,
    capability: &dyn Capability,
    prepared: PreparedContext,
    driver_registry: &DriverRegistry,
) -> Result<(String, EvalMetrics)> {
    let mut metrics = EvalMetrics {
        messages_in_context: prepared.messages.len(),
        messages_excluded: prepared.excluded.len(),
        ..Default::default()
    };

    let provider_type = get_provider_type(&config.model);
    let api_key = get_api_key(&provider_type)?;
    let provider_config = ProviderConfig::new(provider_type).with_api_key(&api_key);
    let driver = driver_registry.create_driver(&provider_config)?;

    let mut llm_messages = build_llm_messages(&prepared, scenario);
    let has_tools = !prepared.tools.is_empty();
    let max_iterations = 5;
    let mut final_response = String::new();

    for _iteration in 0..max_iterations {
        let llm_config = LlmCallConfig {
            model: config.model.clone(),
            temperature: Some(0.0),
            max_tokens: Some(4096),
            tools: prepared.tools.clone(),
            reasoning_effort: None,
            metadata: HashMap::new(),
        };

        let response = driver.chat_completion(llm_messages.clone(), &llm_config).await?;

        metrics.llm_calls += 1;
        if let Some(prompt) = response.metadata.prompt_tokens {
            metrics.input_tokens += prompt as usize;
        }
        if let Some(completion) = response.metadata.completion_tokens {
            metrics.output_tokens += completion as usize;
        }
        metrics.total_tokens = metrics.input_tokens + metrics.output_tokens;

        if let Some(tool_calls) = response.tool_calls {
            if !has_tools || tool_calls.is_empty() {
                final_response = response.text;
                break;
            }

            for tool_call in tool_calls {
                metrics.history_queries += 1;

                // Handle query_history tool directly in runner
                let tool_result = if tool_call.name == "query_history" {
                    execute_query_history(&tool_call.arguments, &prepared.excluded)
                } else {
                    // For other tools, use capability's tool executor
                    execute_capability_tool(capability, &tool_call.name, &tool_call.arguments).await
                };

                llm_messages.push(LlmMessage {
                    role: LlmMessageRole::Assistant,
                    content: LlmMessageContent::Text(response.text.clone()),
                    tool_calls: Some(vec![tool_call.clone()]),
                    tool_call_id: None,
                });

                llm_messages.push(LlmMessage {
                    role: LlmMessageRole::Tool,
                    content: LlmMessageContent::Text(tool_result),
                    tool_calls: None,
                    tool_call_id: Some(tool_call.id),
                });
            }
        } else {
            final_response = response.text;
            break;
        }
    }

    Ok((final_response, metrics))
}

/// Execute query_history tool with excluded messages
fn execute_query_history(arguments: &serde_json::Value, excluded: &[Message]) -> String {
    let query = arguments.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = arguments.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    if query.is_empty() {
        // Return most recent excluded messages
        let recent: Vec<_> = excluded.iter().rev().take(limit).collect();
        let mut output = format!("Most recent {} excluded messages:\n\n", recent.len());
        for msg in recent {
            let content = msg.text_content();
            let truncated = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };
            output.push_str(&format!("--- {:?} ---\n{}\n\n", msg.role, truncated));
        }
        return output;
    }

    // Search by keyword
    let query_lower = query.to_lowercase();
    let mut results: Vec<(usize, &Message, f64)> = Vec::new();

    for (idx, msg) in excluded.iter().enumerate() {
        let content = msg.text_content().to_lowercase();
        if content.contains(&query_lower) {
            let score = 1.0 + (idx as f64 / excluded.len().max(1) as f64) * 0.3;
            results.push((idx, msg, score));
        }
    }

    results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);

    if results.is_empty() {
        return "No matching messages found.".to_string();
    }

    let mut output = format!("Found {} relevant message(s):\n\n", results.len());
    for (idx, msg, score) in results {
        let content = msg.text_content();
        let truncated = if content.len() > 500 {
            format!("{}...", &content[..500])
        } else {
            content
        };
        output.push_str(&format!(
            "--- Message {} ({:?}, score: {:.2}) ---\n{}\n\n",
            idx + 1,
            msg.role,
            score,
            truncated
        ));
    }

    output
}

/// Execute a capability tool by name
async fn execute_capability_tool(
    capability: &dyn Capability,
    name: &str,
    arguments: &serde_json::Value,
) -> String {
    for tool in capability.tools() {
        if tool.name() == name {
            let result = tool.execute(arguments.clone()).await;
            return result.output.to_string();
        }
    }
    format!("Unknown tool: {}", name)
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
    capability: &dyn Capability,
    prepared: &PreparedContext,
    will_exceed: bool,
) -> ScenarioResult {
    ScenarioResult {
        scenario_name: scenario.name.clone(),
        strategy_name: capability.name().to_string(),
        success: !will_exceed,
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

/// Get all available capabilities for evaluation
pub fn all_capabilities() -> Vec<Arc<dyn Capability>> {
    vec![
        Arc::new(BaselineCapability),
        Arc::new(crate::capabilities::NaiveTrimCapability),
        Arc::new(crate::capabilities::InfinityContextCapability),
    ]
}

/// Baseline capability - no filtering, passes all messages through
struct BaselineCapability;

impl Capability for BaselineCapability {
    fn id(&self) -> &str {
        "baseline"
    }

    fn name(&self) -> &str {
        "baseline"
    }

    fn description(&self) -> &str {
        "No context management - passes all messages through"
    }
}
