//! Evaluation runner - executes scenarios using InMemoryAgenticLoop
//!
//! Uses core's InMemoryAgenticLoop for realistic agentic evaluation.
//! Each capability is tested with the full Reason→Act loop.
//!
//! Metrics are derived from events emitted during execution:
//! - `llm.generation`: messages sent to LLM, token usage
//! - `tool.completed`: tool execution results (e.g., query_history calls)

use crate::capabilities::{Capability, InfinityContextCapability, MessageQuery, NaiveTrimCapability};
use crate::metrics::aggregate_strategy_results;
use crate::scorer::{JudgeConfig, Score, aggregate_scores, evaluate_all};
use crate::types::{
    ContextStrategyConfig, EvalMetrics, EvaluationResults, MessageExt, Scenario, ScenarioResult,
};
use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use everruns_core::events::{EventData, LLM_GENERATION, TOOL_COMPLETED};
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::message::Message;
use everruns_core::traits::ModelWithProvider;
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
    /// Delay in milliseconds between LLM calls within a scenario (for tool loops)
    pub inter_call_delay_ms: u64,
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
    let budget_tokens = (config.context_window as f64 * config.budget_percent) as usize;

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

            let result =
                run_single_scenario(config, scenario, capability.as_ref(), budget_tokens).await;

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
    /// Messages to include in context (sent to LLM)
    visible_messages: Vec<Message>,
    /// Estimated tokens for visible messages
    estimated_tokens: usize,
    /// Total messages in scenario history (for calculating excluded count)
    total_history_messages: usize,
}

/// Prepare context by applying capability's message filter
fn prepare_context(
    capability: &dyn Capability,
    messages: &[Message],
    budget_tokens: usize,
) -> PreparedContext {
    let total_history_messages = messages.len();
    let filter_provider = capability.message_filter_provider();

    // If no filter provider, pass all messages through (baseline)
    let Some(provider) = filter_provider else {
        let estimated_tokens: usize = messages.iter().map(|m| m.estimated_tokens()).sum();
        return PreparedContext {
            visible_messages: messages.to_vec(),
            estimated_tokens,
            total_history_messages,
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
    let visible = if let Some(limit) = query.limit {
        let limit = limit.max(0) as usize;
        if messages.len() > limit {
            // Keep the most recent messages
            let split_point = messages.len() - limit;
            messages[split_point..].to_vec()
        } else {
            messages.to_vec()
        }
    } else {
        messages.to_vec()
    };

    let estimated_tokens: usize = visible.iter().map(|m| m.estimated_tokens()).sum();

    PreparedContext {
        visible_messages: visible,
        estimated_tokens,
        total_history_messages,
    }
}

// ============================================================================
// Scenario Execution
// ============================================================================

async fn run_single_scenario(
    config: &EvalConfig,
    scenario: &Scenario,
    capability: &dyn Capability,
    budget_tokens: usize,
) -> ScenarioResult {
    let start = Instant::now();
    let prepared = prepare_context(capability, &scenario.messages, budget_tokens);
    let will_exceed = prepared.estimated_tokens > config.context_window;

    if config.dry_run {
        return create_dry_run_result(scenario, capability, &prepared, will_exceed);
    }

    // Fail immediately if context would be exceeded (simulates API rejection)
    if will_exceed {
        let messages_excluded =
            prepared.total_history_messages.saturating_sub(prepared.visible_messages.len());
        return ScenarioResult {
            scenario_name: scenario.name.clone(),
            strategy_name: capability.name().to_string(),
            score: 0.0,
            scores: vec![Score::new("context", 0.0).with_rationale("Context limit exceeded")],
            response: String::new(),
            metrics: EvalMetrics {
                context_exceeded: true,
                messages_in_context: prepared.visible_messages.len(),
                messages_excluded,
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

    match execute_with_agentic_loop(config, scenario, capability, prepared).await {
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
    prepared: PreparedContext,
) -> Result<(String, EvalMetrics)> {
    let provider_type = get_provider_type(&config.model);
    let api_key = get_api_key(&provider_type)?;

    let model = ModelWithProvider {
        model: config.model.clone(),
        provider_type,
        api_key: Some(api_key),
        base_url: None,
    };

    // Build the agentic loop
    // We add capability based on ID to get proper tool registration
    let mut builder = InMemoryAgenticLoop::builder()
        .agent_name("Eval Agent")
        .system_prompt(build_system_prompt(capability))
        .model(model)
        .driver_registry(create_driver_registry())
        .max_iterations(10);

    // Add capability if it provides tools (infinity_context has query_history)
    match capability.id() {
        "infinity_context" => {
            builder = builder.capability(InfinityContextCapability);
        }
        "naive_trim" => {
            builder = builder.capability(NaiveTrimCapability);
        }
        _ => {
            // baseline and others - no capability needed
        }
    }

    let agentic_loop = builder.build().await?;
    let session_id = agentic_loop.session_id();

    // Seed ALL messages into the retriever for query_history tool access.
    // Calculate how many excluded messages we have.
    let excluded_count =
        prepared.total_history_messages.saturating_sub(prepared.visible_messages.len());

    // Seed excluded messages first (older history) - these won't be in LLM context
    // but will be accessible via query_history tool
    if excluded_count > 0 {
        // We need to reconstruct excluded messages from scenario
        // They are the first N messages where N = total - visible
        let excluded_messages = &scenario.messages[..excluded_count];
        for msg in excluded_messages {
            agentic_loop
                .message_retriever()
                .store(session_id, msg.clone())
                .await?;
        }
    }

    // Seed visible messages (recent context - these are what the LLM sees)
    for msg in &prepared.visible_messages {
        agentic_loop
            .message_retriever()
            .store(session_id, msg.clone())
            .await?;
    }

    // Run the turn with the task as user input
    let result = agentic_loop.run_turn(scenario.task.as_str()).await?;

    // Extract metrics from events (ground truth from actual execution)
    let metrics = extract_metrics_from_events(&agentic_loop, prepared.total_history_messages).await;

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

    let messages_excluded =
        prepared.total_history_messages.saturating_sub(prepared.visible_messages.len());

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
            messages_in_context: prepared.visible_messages.len(),
            messages_excluded,
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
