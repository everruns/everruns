//! Evaluation runner - executes scenarios using InMemoryAgenticLoop
//!
//! Uses core's InMemoryAgenticLoop for realistic agentic evaluation.
//! Each capability is tested with the full Reason→Act loop.
//!
//! Metrics are derived from events emitted during execution:
//! - `llm.generation`: messages sent to LLM, token usage
//! - `tool.completed`: tool execution results (e.g., query_history calls)

use crate::capabilities::{InfinityContextCapability, NaiveTrimCapability};
use crate::dataset::DatasetRecord;
use crate::scorer::{JudgeConfig, Score, Scorer, aggregate_scores, evaluate_all};
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::Capability;
use everruns_core::events::{EventData, LLM_GENERATION, TOOL_COMPLETED};
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::traits::ModelWithProvider;
use serde::Serialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::time::sleep;

// ----------------------------------------------------------------------------
// Approach
// ----------------------------------------------------------------------------

/// Context management approach to evaluate
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approach {
    /// No context management - passes all messages through
    Baseline,
    /// Drop oldest messages to fit context budget
    NaiveTrim,
    /// Trim + history query tool for accessing excluded messages
    InfinityContext,
}

impl Approach {
    pub fn all() -> Vec<Approach> {
        vec![Approach::Baseline, Approach::NaiveTrim, Approach::InfinityContext]
    }

    pub fn name(&self) -> &'static str {
        match self {
            Approach::Baseline => "baseline",
            Approach::NaiveTrim => "naive_trim",
            Approach::InfinityContext => "infinity_context",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Approach::Baseline => "No context management - passes all messages through",
            Approach::NaiveTrim => "Drops oldest messages when context exceeds budget",
            Approach::InfinityContext => "Trims context + provides history query tool",
        }
    }
}

impl std::fmt::Display for Approach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl std::str::FromStr for Approach {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "baseline" => Ok(Approach::Baseline),
            "naive_trim" => Ok(Approach::NaiveTrim),
            "infinity_context" => Ok(Approach::InfinityContext),
            _ => Err(anyhow::anyhow!("Unknown approach: {}. Valid: baseline, naive_trim, infinity_context", s)),
        }
    }
}

// ----------------------------------------------------------------------------
// Result Types
// ----------------------------------------------------------------------------

/// Metrics collected during evaluation
#[derive(Debug, Clone, Default, Serialize)]
pub struct EvalMetrics {
    /// Total tokens used (input + output)
    pub total_tokens: usize,
    /// Input tokens
    pub input_tokens: usize,
    /// Output tokens
    pub output_tokens: usize,
    /// Number of LLM calls made
    pub llm_calls: usize,
    /// Number of history queries made (for infinity strategy)
    pub history_queries: usize,
    /// Time to complete in milliseconds
    pub latency_ms: u64,
    /// Whether context was exceeded
    pub context_exceeded: bool,
    /// Messages included in context
    pub messages_in_context: usize,
    /// Messages excluded from context
    pub messages_excluded: usize,
    /// Total retry attempts across all LLM calls (due to rate limits)
    pub retry_attempts: u32,
    /// Total time spent waiting for retries in milliseconds
    pub retry_wait_ms: u64,
}

/// Result of running a single record with an approach
#[derive(Debug, Clone, Serialize)]
pub struct RecordResult {
    pub record_id: String,
    pub approach_name: String,
    /// Aggregate score (0.0-1.0)
    pub score: f64,
    /// Individual scorer results
    pub scores: Vec<Score>,
    pub response: String,
    pub metrics: EvalMetrics,
    pub error: Option<String>,
}

impl RecordResult {
    /// Returns true if aggregate score >= 0.5
    pub fn passed(&self) -> bool {
        self.score >= 0.5
    }
}

/// Aggregated results across all records for an approach
#[derive(Debug, Clone, Serialize)]
pub struct ApproachResults {
    pub approach_name: String,
    pub total_records: usize,
    /// Number of records with score >= 0.5
    pub passed: usize,
    /// Number of records with score < 0.5
    pub failed: usize,
    pub context_exceeded: usize,
    /// Average score across all records (0.0-1.0)
    pub avg_score: f64,
    pub avg_tokens: f64,
    pub avg_latency_ms: f64,
    pub avg_history_queries: f64,
    /// Total retry attempts across all records
    pub total_retry_attempts: u32,
    /// Total retry wait time in milliseconds across all records
    pub total_retry_wait_ms: u64,
    pub record_results: Vec<RecordResult>,
}

impl ApproachResults {
    /// Average score as percentage (0-100)
    pub fn accuracy(&self) -> f64 {
        self.avg_score * 100.0
    }
}

/// Full evaluation results
#[derive(Debug, Clone, Serialize)]
pub struct EvaluationResults {
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub config: HashMap<String, String>,
    pub approach_results: Vec<ApproachResults>,
}

// ----------------------------------------------------------------------------
// Config & Intermediate Types
// ----------------------------------------------------------------------------

/// Evaluation configuration
#[derive(Clone)]
pub struct EvalConfig {
    pub model: String,
    pub approach: Approach,
    pub dry_run: bool,
}

/// Result of running a scenario (before scoring)
#[derive(Clone)]
pub struct RunResult {
    pub record_id: String,
    pub approach: Approach,
    pub response: String,
    pub metrics: EvalMetrics,
    pub error: Option<String>,
}

// ----------------------------------------------------------------------------
// Driver Setup
// ----------------------------------------------------------------------------

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

/// Convert expectations to LLM judge scorers
fn expectations_to_scorers(expectations: &[String]) -> Vec<Scorer> {
    expectations
        .iter()
        .enumerate()
        .map(|(i, expectation)| Scorer::llm_judge(format!("expectation_{}", i), expectation.clone()))
        .collect()
}

// ----------------------------------------------------------------------------
// Scenario Execution
// ----------------------------------------------------------------------------

/// Run a single scenario and return the result (without scoring)
pub async fn run_scenario(config: &EvalConfig, record: &DatasetRecord) -> RunResult {
    let start = Instant::now();

    if config.dry_run {
        return RunResult {
            record_id: record.id.clone(),
            approach: config.approach,
            response: "[DRY RUN]".to_string(),
            metrics: EvalMetrics {
                messages_in_context: record.input.events.len(),
                ..Default::default()
            },
            error: None,
        };
    }

    match execute_with_agentic_loop(config, record).await {
        Ok((response, metrics)) => RunResult {
            record_id: record.id.clone(),
            approach: config.approach,
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
                record_id: record.id.clone(),
                approach: config.approach,
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

// ----------------------------------------------------------------------------
// Scoring
// ----------------------------------------------------------------------------

/// Score a run result against a record's expectations
pub async fn score_result(record: &DatasetRecord, run: &RunResult) -> RecordResult {
    // If there was an error, return error score
    if let Some(err) = &run.error {
        return RecordResult {
            record_id: run.record_id.clone(),
            approach_name: run.approach.name().to_string(),
            score: 0.0,
            scores: vec![Score::new("error", 0.0).with_rationale(err.clone())],
            response: run.response.clone(),
            metrics: run.metrics.clone(),
            error: Some(err.clone()),
        };
    }

    // Score the response using all scorers
    let scorers = expectations_to_scorers(&record.expectations);
    let judge_config = create_judge_config();
    let scores = evaluate_all(&scorers, &record.task, &run.response, judge_config.as_ref()).await;
    let score = aggregate_scores(&scores);

    RecordResult {
        record_id: run.record_id.clone(),
        approach_name: run.approach.name().to_string(),
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

// ----------------------------------------------------------------------------
// Aggregation
// ----------------------------------------------------------------------------

/// Aggregate results for a single approach across all records
pub fn aggregate_approach_results(approach_name: &str, results: Vec<RecordResult>) -> ApproachResults {
    let total = results.len();
    let passed = results.iter().filter(|r| r.passed()).count();
    let context_exceeded = results.iter().filter(|r| r.metrics.context_exceeded).count();

    let avg_score = if total > 0 {
        results.iter().map(|r| r.score).sum::<f64>() / total as f64
    } else {
        0.0
    };

    let avg_tokens = if total > 0 {
        results.iter().map(|r| r.metrics.total_tokens).sum::<usize>() as f64 / total as f64
    } else {
        0.0
    };

    let avg_latency = if total > 0 {
        results.iter().map(|r| r.metrics.latency_ms).sum::<u64>() as f64 / total as f64
    } else {
        0.0
    };

    let avg_queries = if total > 0 {
        results.iter().map(|r| r.metrics.history_queries).sum::<usize>() as f64 / total as f64
    } else {
        0.0
    };

    // Sum retry metrics across all records
    let total_retry_attempts: u32 = results.iter().map(|r| r.metrics.retry_attempts).sum();
    let total_retry_wait_ms: u64 = results.iter().map(|r| r.metrics.retry_wait_ms).sum();

    ApproachResults {
        approach_name: approach_name.to_string(),
        total_records: total,
        passed,
        failed: total - passed,
        context_exceeded,
        avg_score,
        avg_tokens,
        avg_latency_ms: avg_latency,
        avg_history_queries: avg_queries,
        total_retry_attempts,
        total_retry_wait_ms,
        record_results: results,
    }
}

// ----------------------------------------------------------------------------
// Agentic Loop Execution
// ----------------------------------------------------------------------------

/// Maximum retries for rate-limited requests
const MAX_RETRIES: u32 = 5;

/// Initial backoff delay in milliseconds
const INITIAL_BACKOFF_MS: u64 = 5000;

/// Execute scenario using InMemoryAgenticLoop with rate limit retry
async fn execute_with_agentic_loop(
    config: &EvalConfig,
    record: &DatasetRecord,
) -> Result<(String, EvalMetrics)> {
    let provider_type = get_provider_type(&config.model);
    let api_key = get_api_key(&provider_type)?;

    let mut retries = 0;
    let mut backoff_ms = INITIAL_BACKOFF_MS;

    loop {
        let model = ModelWithProvider {
            model: config.model.clone(),
            provider_type: provider_type.clone(),
            api_key: Some(api_key.clone()),
            base_url: None,
        };

        // Build the agentic loop with seed events
        let mut builder = InMemoryAgenticLoop::builder()
            .agent_name("Eval Agent")
            .system_prompt(build_system_prompt(config.approach))
            .model(model)
            .driver_registry(create_driver_registry())
            .max_iterations(10)
            .seed_events(record.input.events.clone());

        // Add capability based on approach
        match config.approach {
            Approach::Baseline => {
                // No capability - pass all messages through
            }
            Approach::NaiveTrim => {
                builder = builder.capability(NaiveTrimCapability);
            }
            Approach::InfinityContext => {
                builder = builder.capability(InfinityContextCapability);
            }
        }

        let agentic_loop = builder.build().await?;

        // Run the turn with the task as user input
        let result = agentic_loop.run_turn(record.task.as_str()).await?;

        // Check if turn failed (LLM failures return Ok with success=false)
        if !result.success {
            if let Some(ref err_str) = result.error {
                let is_rate_limit = err_str.contains("429")
                    || err_str.contains("rate_limit")
                    || err_str.contains("Too Many Requests");

                if is_rate_limit && retries < MAX_RETRIES {
                    retries += 1;
                    tracing::warn!(
                        "Rate limited, retrying in {}ms (attempt {}/{})",
                        backoff_ms,
                        retries,
                        MAX_RETRIES
                    );
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms *= 2; // Exponential backoff
                    continue; // Retry the loop
                }
            }
            // Non-retryable failure - return error
            return Err(anyhow::anyhow!(
                "Turn failed: {}",
                result.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        // Success - extract metrics and return
        let metrics =
            extract_metrics_from_events(&agentic_loop, record.input.events.len()).await;
        return Ok((result.response, metrics));
    }
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

            // Retry info from rate limit handling
            if let Some(retry) = &data.metadata.retry {
                metrics.retry_attempts += retry.attempts;
                metrics.retry_wait_ms += retry.total_wait_ms;
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

fn build_system_prompt(approach: Approach) -> String {
    let base = "You are a helpful assistant. Answer questions based on the conversation history.";

    match approach {
        Approach::Baseline | Approach::NaiveTrim => base.to_string(),
        Approach::InfinityContext => {
            // Add system prompt for infinity context capability
            format!(
                "{}\n\n{}",
                base,
                InfinityContextCapability.system_prompt_addition().unwrap_or("")
            )
        }
    }
}
