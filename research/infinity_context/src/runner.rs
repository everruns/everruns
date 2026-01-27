//! Evaluation runner - executes scenarios using InMemoryAgenticLoop
//!
//! Uses core's InMemoryAgenticLoop for realistic agentic evaluation.
//! Each capability is tested with the full Reason→Act loop.
//!
//! Metrics are derived from events emitted during execution:
//! - `llm.generation`: messages sent to LLM, token usage
//! - `tool.completed`: tool execution results (e.g., query_history calls)

use crate::capabilities::{Capability, InfinityContextCapability, NaiveTrimCapability};
use crate::scorer::{JudgeConfig, Score, aggregate_scores, evaluate_all};
use crate::types::{EvalMetrics, Scenario, ScenarioResult};
use anyhow::Result;
use everruns_core::events::{EventData, LLM_GENERATION, TOOL_COMPLETED};
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::traits::ModelWithProvider;
use std::sync::Arc;
use std::time::Instant;

/// Evaluation configuration
#[derive(Clone)]
pub struct EvalConfig {
    pub model: String,
    pub capability: Arc<dyn Capability>,
    pub dry_run: bool,
}

/// Result of running a scenario (before scoring)
#[derive(Clone)]
pub struct RunResult {
    pub scenario_name: String,
    pub capability_name: String,
    pub response: String,
    pub metrics: EvalMetrics,
    pub error: Option<String>,
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
// Scenario Execution
// ============================================================================

/// Run a single scenario and return the result (without scoring)
pub async fn run_scenario(config: &EvalConfig, scenario: &Scenario) -> RunResult {
    let start = Instant::now();

    if config.dry_run {
        return RunResult {
            scenario_name: scenario.name.clone(),
            capability_name: config.capability.name().to_string(),
            response: "[DRY RUN]".to_string(),
            metrics: EvalMetrics {
                messages_in_context: scenario.messages.len(),
                ..Default::default()
            },
            error: None,
        };
    }

    match execute_with_agentic_loop(config, scenario).await {
        Ok((response, metrics)) => RunResult {
            scenario_name: scenario.name.clone(),
            capability_name: config.capability.name().to_string(),
            response,
            metrics: EvalMetrics {
                latency_ms: start.elapsed().as_millis() as u64,
                ..metrics
            },
            error: None,
        },
        Err(e) => {
            let is_context_error = e.to_string().contains("context")
                || e.to_string().contains("too long")
                || e.to_string().contains("maximum");

            RunResult {
                scenario_name: scenario.name.clone(),
                capability_name: config.capability.name().to_string(),
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

// ============================================================================
// Scoring
// ============================================================================

/// Score a run result against a scenario's expectations
pub async fn score_result(scenario: &Scenario, run: &RunResult) -> ScenarioResult {
    // If there was an error, return error score
    if let Some(err) = &run.error {
        return ScenarioResult {
            scenario_name: run.scenario_name.clone(),
            strategy_name: run.capability_name.clone(),
            score: 0.0,
            scores: vec![Score::new("error", 0.0).with_rationale(err.clone())],
            response: run.response.clone(),
            metrics: run.metrics.clone(),
            error: Some(err.clone()),
        };
    }

    // Score the response using all scorers
    let judge_config = create_judge_config();
    let scores = evaluate_all(
        &scenario.scorers,
        &scenario.task,
        &run.response,
        judge_config.as_ref(),
    )
    .await;
    let score = aggregate_scores(&scores);

    ScenarioResult {
        scenario_name: run.scenario_name.clone(),
        strategy_name: run.capability_name.clone(),
        score,
        scores,
        response: run.response.clone(),
        metrics: run.metrics.clone(),
        error: None,
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

// ============================================================================
// Agentic Loop Execution
// ============================================================================

/// Execute scenario using InMemoryAgenticLoop
async fn execute_with_agentic_loop(
    config: &EvalConfig,
    scenario: &Scenario,
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
        .system_prompt(build_system_prompt(config.capability.as_ref()))
        .model(model)
        .driver_registry(create_driver_registry())
        .max_iterations(10);

    // Add capability for tool registration and message filtering
    match config.capability.id() {
        "infinity_context" => {
            builder = builder.capability(InfinityContextCapability);
        }
        "naive_trim" => {
            builder = builder.capability(NaiveTrimCapability);
        }
        id => {
            anyhow::bail!("Unknown capability: {}", id);
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

// ============================================================================
// Capability Registry
// ============================================================================

/// Get all available capabilities for evaluation
pub fn all_capabilities() -> Vec<Arc<dyn Capability>> {
    vec![
        Arc::new(NaiveTrimCapability),
        Arc::new(InfinityContextCapability),
    ]
}
