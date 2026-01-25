//! Evaluation runner - executes scenarios against capabilities
//!
//! Uses core's LLM driver and Capability infrastructure directly.

use crate::capabilities::{Capability, MessageQuery};
use crate::metrics::aggregate_strategy_results;
use crate::scorer::{JudgeConfig, Score, aggregate_scores, evaluate_all};
use crate::types::{
    ContextStrategyConfig, EvalMetrics, EvaluationResults, Message, MessageExt, Scenario,
    ScenarioResult,
};
use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use everruns_core::llm_driver_registry::{
    DriverRegistry, LlmCallConfig, LlmMessage, LlmMessageContent, LlmMessageRole, ProviderConfig,
    ProviderType,
};
use everruns_core::memory::InMemoryMessageRetriever;
use everruns_core::tool_types::{BuiltinTool, ToolDefinition as CoreToolDefinition};
use everruns_core::tools::Tool;
use everruns_core::traits::ToolContext;
use everruns_core::typed_id::SessionId;
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
    /// Delay in milliseconds between scenarios (helps avoid rate limits)
    pub delay_ms: u64,
}

// ============================================================================
// Driver Setup
// ============================================================================

fn create_driver_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut registry);
    everruns_openai::register_driver(&mut registry);
    registry
}

fn get_provider_type(model: &str) -> ProviderType {
    if model.starts_with("claude") {
        ProviderType::Anthropic
    } else {
        ProviderType::OpenAI
    }
}

// ============================================================================
// Evaluation Runner
// ============================================================================

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
        let mut is_first = true;

        for scenario in scenarios {
            // Delay between scenarios (skip first)
            if !is_first && config.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(config.delay_ms)).await;
            }
            is_first = false;

            let result = run_single_scenario(
                config,
                scenario,
                capability.as_ref(),
                budget_tokens,
                &driver_registry,
            )
            .await;

            let status = if result.score >= 0.8 {
                "✓".bright_green()
            } else if result.score >= 0.5 {
                "◐".bright_yellow()
            } else if result.metrics.context_exceeded {
                "⊘".bright_yellow()
            } else {
                "✗".bright_red()
            };

            println!(
                "  {} {} (score: {:.2}, tokens: {}, latency: {}ms, queries: {})",
                status,
                scenario.name,
                result.score,
                result.metrics.total_tokens,
                result.metrics.latency_ms,
                result.metrics.history_queries
            );

            if result.score < 0.5
                && !result.metrics.context_exceeded
                && let Some(err) = &result.error
            {
                println!("    {} {}", "Error:".red(), err);
            }

            results.push(result);
        }

        let aggregated = aggregate_strategy_results(capability.name(), results);
        println!(
            "\n  {} Avg Score: {:.1}% ({}/{})\n",
            "→".bright_blue(),
            aggregated.accuracy(),
            aggregated.passed,
            aggregated.total_scenarios
        );

        strategy_results.push(aggregated);
    }

    Ok(EvaluationResults {
        timestamp: Utc::now(),
        model: config.model.clone(),
        config: HashMap::from([
            (
                "context_window".to_string(),
                config.context_window.to_string(),
            ),
            (
                "budget_percent".to_string(),
                config.budget_percent.to_string(),
            ),
        ]),
        strategy_results,
    })
}

// ============================================================================
// Context Preparation
// ============================================================================

struct PreparedContext {
    messages: Vec<Message>,
    excluded: Vec<Message>,
    system_addition: Option<String>,
    tools: Vec<Box<dyn Tool>>,
    tool_definitions: Vec<CoreToolDefinition>,
    estimated_tokens: usize,
}

fn prepare_context(
    capability: &dyn Capability,
    messages: &[Message],
    budget_tokens: usize,
) -> PreparedContext {
    let filter_provider = capability.message_filter_provider();

    // If no filter provider, pass all messages through (baseline)
    let Some(provider) = filter_provider else {
        let estimated_tokens: usize = messages.iter().map(|m| m.estimated_tokens()).sum();
        let tools = capability.tools();
        let tool_definitions = tools_to_definitions(&tools);
        return PreparedContext {
            messages: messages.to_vec(),
            excluded: vec![],
            system_addition: capability.system_prompt_addition().map(String::from),
            tools,
            tool_definitions,
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

    // Apply limit from query (keep most recent N messages)
    let (kept, excluded) = if let Some(limit) = query.limit {
        let limit = limit.max(0) as usize;
        if messages.len() > limit {
            // Keep the most recent messages
            let split_point = messages.len() - limit;
            (
                messages[split_point..].to_vec(),
                messages[..split_point].to_vec(),
            )
        } else {
            (messages.to_vec(), vec![])
        }
    } else {
        (messages.to_vec(), vec![])
    };

    let estimated_tokens: usize = kept.iter().map(|m| m.estimated_tokens()).sum();
    let tools = capability.tools();
    let tool_definitions = tools_to_definitions(&tools);

    PreparedContext {
        messages: kept,
        excluded,
        system_addition: capability.system_prompt_addition().map(String::from),
        tools,
        tool_definitions,
        estimated_tokens,
    }
}

fn tools_to_definitions(tools: &[Box<dyn Tool>]) -> Vec<CoreToolDefinition> {
    tools
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

// ============================================================================
// Scenario Execution
// ============================================================================

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

    // Fail immediately if context would be exceeded (simulates API rejection)
    if will_exceed {
        return ScenarioResult {
            scenario_name: scenario.name.clone(),
            strategy_name: capability.name().to_string(),
            score: 0.0,
            scores: vec![Score::new("context", 0.0).with_rationale("Context limit exceeded")],
            response: String::new(),
            metrics: EvalMetrics {
                context_exceeded: true,
                messages_in_context: prepared.messages.len(),
                messages_excluded: prepared.excluded.len(),
                total_tokens: prepared.estimated_tokens,
                latency_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            },
            error: Some(format!(
                "Context exceeded: {} tokens > {} limit",
                prepared.estimated_tokens, config.context_window
            )),
        };
    }

    let messages_in_context = prepared.messages.len();
    let messages_excluded = prepared.excluded.len();

    match execute_llm_calls(config, scenario, prepared, driver_registry).await {
        Ok((response, metrics)) => {
            // Score the response using all scorers
            let judge_config = create_judge_config();
            let scores = evaluate_all(
                &scenario.scorers,
                &scenario.task,
                &response,
                judge_config.as_ref(),
            )
            .await;
            let score = aggregate_scores(&scores);

            ScenarioResult {
                scenario_name: scenario.name.clone(),
                strategy_name: capability.name().to_string(),
                score,
                scores,
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
                score: 0.0,
                scores: vec![Score::new("error", 0.0).with_rationale(e.to_string())],
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

/// Create judge config from environment if API key is available
fn create_judge_config() -> Option<JudgeConfig> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .map(|api_key| JudgeConfig {
            model: "claude-3-5-haiku-20241022".to_string(),
            api_key,
        })
}

async fn execute_llm_calls(
    config: &EvalConfig,
    scenario: &Scenario,
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
    let has_tools = !prepared.tool_definitions.is_empty();
    let max_iterations = 5;
    let mut final_response = String::new();

    // Create tool context with excluded messages via InMemoryMessageRetriever
    let session_id = SessionId::new();
    let retriever = InMemoryMessageRetriever::new();
    retriever.seed(session_id, prepared.excluded).await;
    let tool_context = ToolContext::new(session_id).with_message_retriever(Arc::new(retriever));

    for _iteration in 0..max_iterations {
        let llm_config = LlmCallConfig {
            model: config.model.clone(),
            temperature: Some(0.0),
            max_tokens: Some(4096),
            tools: prepared.tool_definitions.clone(),
            reasoning_effort: None,
            metadata: HashMap::new(),
        };

        let response = driver
            .chat_completion(llm_messages.clone(), &llm_config)
            .await?;

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

                // Find and execute the tool using execute_with_context
                let tool_result = execute_tool(
                    &prepared.tools,
                    &tool_call.name,
                    &tool_call.arguments,
                    &tool_context,
                )
                .await;

                llm_messages.push(LlmMessage {
                    role: LlmMessageRole::Assistant,
                    content: LlmMessageContent::Text(response.text.clone()),
                    tool_calls: Some(vec![tool_call.clone()]),
                    tool_call_id: None,
                    thinking: None,
                    thinking_signature: None,
                });

                llm_messages.push(LlmMessage {
                    role: LlmMessageRole::Tool,
                    content: LlmMessageContent::Text(tool_result),
                    tool_calls: None,
                    tool_call_id: Some(tool_call.id),
                    thinking: None,
                    thinking_signature: None,
                });
            }
        } else {
            final_response = response.text;
            break;
        }
    }

    Ok((final_response, metrics))
}

/// Execute a tool by name using execute_with_context
async fn execute_tool(
    tools: &[Box<dyn Tool>],
    name: &str,
    arguments: &serde_json::Value,
    context: &ToolContext,
) -> String {
    use everruns_core::tools::ToolExecutionResult;

    for tool in tools {
        if tool.name() == name {
            let result = tool.execute_with_context(arguments.clone(), context).await;
            return match result {
                ToolExecutionResult::Success(value) => value.to_string(),
                ToolExecutionResult::ToolError(msg) => format!("Error: {}", msg),
                ToolExecutionResult::InternalError(err) => {
                    format!("Internal error: {}", err.message)
                }
            };
        }
    }
    format!("Unknown tool: {}", name)
}

// ============================================================================
// Helpers
// ============================================================================

fn get_api_key(provider_type: &ProviderType) -> Result<String> {
    let key_name = match provider_type {
        ProviderType::Anthropic => "ANTHROPIC_API_KEY",
        ProviderType::OpenAI | ProviderType::OpenAICompletions => "OPENAI_API_KEY",
        ProviderType::LlmSim => return Ok(String::new()),
    };
    std::env::var(key_name).map_err(|_| anyhow::anyhow!("{} not set", key_name))
}

fn build_llm_messages(prepared: &PreparedContext, scenario: &Scenario) -> Vec<LlmMessage> {
    let mut messages = Vec::new();

    let mut system_content =
        "You are a helpful assistant. Answer questions based on the conversation history."
            .to_string();
    if let Some(ref addition) = prepared.system_addition {
        system_content.push_str("\n\n");
        system_content.push_str(addition);
    }

    messages.push(LlmMessage {
        role: LlmMessageRole::System,
        content: LlmMessageContent::Text(system_content),
        tool_calls: None,
        tool_call_id: None,
        thinking: None,
        thinking_signature: None,
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
            thinking: None,
            thinking_signature: None,
        });
    }

    messages.push(LlmMessage {
        role: LlmMessageRole::User,
        content: LlmMessageContent::Text(scenario.task.clone()),
        tool_calls: None,
        tool_call_id: None,
        thinking: None,
        thinking_signature: None,
    });

    messages
}

fn create_dry_run_result(
    scenario: &Scenario,
    capability: &dyn Capability,
    prepared: &PreparedContext,
    will_exceed: bool,
) -> ScenarioResult {
    let (score, scores) = if will_exceed {
        (
            0.0,
            vec![Score::new("context", 0.0).with_rationale("Context would exceed limit")],
        )
    } else {
        (
            1.0,
            vec![Score::new("dry_run", 1.0).with_rationale("Dry run - assumed pass")],
        )
    };

    ScenarioResult {
        scenario_name: scenario.name.clone(),
        strategy_name: capability.name().to_string(),
        score,
        scores,
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

// ============================================================================
// Capability Registry
// ============================================================================

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
