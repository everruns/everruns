//! Evaluation runner - executes scenarios using InMemoryAgenticLoop
//!
//! Uses core's InMemoryAgenticLoop for realistic agentic evaluation.
//! Each capability is tested with the full Reason→Act loop.
//!
//! Metrics are derived from events emitted during execution:
//! - `llm.generation`: messages sent to LLM, token usage
//! - `tool.completed`: tool execution results (e.g., query_history calls)

use crate::capabilities::{Capability, InfinityContextCapability, NaiveTrimCapability};
use crate::metrics::aggregate_strategy_results;
use crate::scorer::{JudgeConfig, Score, aggregate_scores, evaluate_all};
use crate::types::{EvalMetrics, EvaluationResults, Scenario, ScenarioResult};
use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use everruns_core::events::{EventData, LLM_GENERATION, TOOL_COMPLETED};
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::traits::ModelWithProvider;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Evaluation configuration
#[derive(Clone)]
pub struct EvalConfig {
    pub model: String,
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

fn get_provider_type(model: &str) -> LlmProviderType {
    if model.starts_with("claude") {
        LlmProviderType::Anthropic
    } else {
        LlmProviderType::Openai
    }
}

fn get_api_key(provider_type: &LlmProviderType) -> Result<String> {
    let key_name = match provider_type {
        LlmProviderType::Anthropic => "ANTHROPIC_API_KEY",
        LlmProviderType::Openai | LlmProviderType::OpenaiCompletions => "OPENAI_API_KEY",
        LlmProviderType::LlmSim => return Ok(String::new()),
    };
    std::env::var(key_name).map_err(|_| anyhow::anyhow!("{} not set", key_name))
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
    println!(
        "\n{} Running {} scenario(s) against {} capability(ies)\n",
        "▶".bright_blue(),
        scenarios.len(),
        capabilities.len()
    );

    let mut strategy_results = Vec::new();
    let mut is_first_cap = true;

    for capability in capabilities {
        // Delay between capabilities too
        if !is_first_cap && config.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(config.delay_ms)).await;
        }
        is_first_cap = false;

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

            let result = run_single_scenario(config, scenario, capability.as_ref()).await;

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
        config: HashMap::new(),
        strategy_results,
    })
}

// ============================================================================
// Scenario Execution
// ============================================================================

async fn run_single_scenario(
    config: &EvalConfig,
    scenario: &Scenario,
    capability: &dyn Capability,
) -> ScenarioResult {
    let start = Instant::now();

    if config.dry_run {
        return create_dry_run_result(scenario, capability);
    }

    match execute_with_agentic_loop(config, scenario, capability).await {
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

/// Execute scenario using InMemoryAgenticLoop
async fn execute_with_agentic_loop(
    config: &EvalConfig,
    scenario: &Scenario,
    capability: &dyn Capability,
) -> Result<(String, EvalMetrics)> {
    let provider_type = get_provider_type(&config.model);
    let api_key = get_api_key(&provider_type)?;

    let model = ModelWithProvider {
        model: config.model.clone(),
        provider_type,
        api_key: Some(api_key),
        base_url: None,
    };

    // Build the agentic loop with the capability
    let mut builder = InMemoryAgenticLoop::builder()
        .agent_name("Eval Agent")
        .system_prompt(build_system_prompt(capability))
        .model(model)
        .driver_registry(create_driver_registry())
        .max_iterations(10);

    // Add capability for tool registration and message filtering
    match capability.id() {
        "infinity_context" => {
            builder = builder.capability(InfinityContextCapability);
        }
        "naive_trim" => {
            builder = builder.capability(NaiveTrimCapability);
        }
        _ => {
            // baseline - no capability needed
        }
    }

    let agentic_loop = builder.build().await?;
    let session_id = agentic_loop.session_id();

    // Seed all scenario messages into the retriever.
    // The capability's message filter (if any) will determine what goes to the LLM.
    // The query_history tool can access all messages via the retriever.
    for msg in &scenario.messages {
        agentic_loop
            .message_retriever()
            .store(session_id, msg.clone())
            .await?;
    }

    // Run the turn with the task as user input
    let result = agentic_loop.run_turn(scenario.task.as_str()).await?;

    // Extract metrics from events (ground truth from actual execution)
    let metrics = extract_metrics_from_events(&agentic_loop, scenario.messages.len()).await;

    Ok((result.response, metrics))
}

/// Extract evaluation metrics from events emitted during execution.
///
/// This provides ground truth metrics based on what actually happened:
/// - `llm.generation` events: messages sent to LLM, token usage, call count
/// - `tool.completed` events: query_history tool usage
async fn extract_metrics_from_events(
    agentic_loop: &InMemoryAgenticLoop,
    total_history_messages: usize,
) -> EvalMetrics {
    let mut metrics = EvalMetrics::default();

    // Extract from llm.generation events
    let llm_events = agentic_loop.events_by_type(LLM_GENERATION).await;
    metrics.llm_calls = llm_events.len();

    for event in &llm_events {
        if let EventData::LlmGeneration(data) = &event.data {
            // Token usage from the LLM call
            if let Some(usage) = &data.metadata.usage {
                metrics.input_tokens += usage.input_tokens as usize;
                metrics.output_tokens += usage.output_tokens as usize;
                metrics.total_tokens += usage.total_tokens() as usize;
            }

            // Messages in context from first LLM call (before any tool results added)
            // This represents what the capability initially sent to the LLM
            if metrics.messages_in_context == 0 {
                // Count non-system messages to match our scenario message count
                metrics.messages_in_context = data
                    .messages
                    .iter()
                    .filter(|m| !matches!(m.role, everruns_core::message::MessageRole::System))
                    .count();
            }
        }
    }

    // Calculate excluded messages: total history - what was in first LLM context
    // Subtract 1 for the task message that run_turn adds
    let context_without_task = metrics.messages_in_context.saturating_sub(1);
    metrics.messages_excluded = total_history_messages.saturating_sub(context_without_task);

    // Extract query_history tool calls from tool.completed events
    let tool_events = agentic_loop.events_by_type(TOOL_COMPLETED).await;
    metrics.history_queries = tool_events
        .iter()
        .filter(|e| {
            if let EventData::ToolCompleted(data) = &e.data {
                data.tool_name == "query_history"
            } else {
                false
            }
        })
        .count();

    metrics
}

fn build_system_prompt(capability: &dyn Capability) -> String {
    let mut prompt =
        "You are a helpful assistant. Answer questions based on the conversation history."
            .to_string();

    if let Some(addition) = capability.system_prompt_addition() {
        prompt.push_str("\n\n");
        prompt.push_str(addition);
    }

    prompt
}

fn create_dry_run_result(scenario: &Scenario, capability: &dyn Capability) -> ScenarioResult {
    ScenarioResult {
        scenario_name: scenario.name.clone(),
        strategy_name: capability.name().to_string(),
        score: 1.0,
        scores: vec![Score::new("dry_run", 1.0).with_rationale("Dry run - assumed pass")],
        response: "[DRY RUN]".to_string(),
        metrics: EvalMetrics {
            messages_in_context: scenario.messages.len(),
            ..Default::default()
        },
        error: None,
    }
}

// ============================================================================
// Capability Registry
// ============================================================================

/// Get all available capabilities for evaluation
pub fn all_capabilities() -> Vec<Arc<dyn Capability>> {
    vec![
        Arc::new(BaselineCapability),
        Arc::new(NaiveTrimCapability),
        Arc::new(InfinityContextCapability),
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
