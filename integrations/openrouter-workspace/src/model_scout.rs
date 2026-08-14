// Model Scout Capability — OpenRouter model benchmark blueprint
//
// Contributes the `openrouter_model_scout` AgentBlueprint. The blueprint
// provides a private tool suite that lets an orchestrating agent:
//   1. Fetch the OpenRouter model catalog.
//   2. Run small probe tasks against candidate model/provider routes.
//   3. Rank candidates on success rate, latency, and cost.
//   4. Produce a concrete model-router update proposal.
//
// Applying the proposal is always an explicit operator step; the blueprint
// never mutates router configuration automatically.
//
// Design note: probe calls go directly to OpenRouter via the
// `provider_credential_store` (openrouter API key) so each candidate is
// evaluated in isolation. The ranking logic is deterministic and fully unit-
// tested without network access.
//
// Security: TM-LLM — probe prompts are fixed strings provided by this code
// or supplied by the operator in the blueprint config; they are sent only to
// OpenRouter endpoints. API key comes from `provider_credential_store` and is
// never logged. TM-DOS — probe count is bounded by `max_candidates`
// (default 10) and spend by `max_spend_usd` (default $0.10).

use super::{
    AgentBlueprint, BlueprintModel, Capability, CapabilityLocalization, CapabilityStatus, RiskLevel,
};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use everruns_core::tool_context::ToolContext;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MODEL_SCOUT_CAPABILITY_ID: &str = "model_scout";
const DEFAULT_PROBE_TIMEOUT_MS: u64 = 10_000;
const MIN_PROBE_TIMEOUT_MS: u64 = 1_000;
const MAX_PROBE_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_MAX_SPEND_USD: f64 = 0.10;
const MAX_PROBE_SPEND_USD: f64 = 10.0;
const MAX_PROBE_TASKS: usize = 50;

/// Capability that contributes the OpenRouter model scout blueprint.
pub struct ModelScoutCapability;

impl Capability for ModelScoutCapability {
    fn id(&self) -> &str {
        MODEL_SCOUT_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Model Scout"
    }

    fn description(&self) -> &str {
        "OpenRouter model benchmarking blueprint — evaluates model/provider routes with probe tasks and recommends model-router updates."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("telescope")
    }

    fn category(&self) -> Option<&str> {
        Some("AI")
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
        vec![AgentBlueprint {
            id: "openrouter_model_scout",
            name: "OpenRouter Model Scout",
            description: "Benchmarks OpenRouter models/providers using small probe tasks and recommends model-router updates. Use when you need to evaluate which OpenRouter models best suit a task profile for cost, latency, or quality.",
            // Haiku is cheap and fast enough for orchestration; heavy work is done
            // by probe calls directly against candidate models.
            model: BlueprintModel::Default("claude-haiku-4-5-20251001".to_string()),
            system_prompt: SCOUT_SYSTEM_PROMPT,
            tools: vec![
                Box::new(ListOpenRouterCatalogTool),
                Box::new(ProbeModelTool),
                Box::new(RankModelsTool),
                Box::new(ProposeRouterUpdateTool),
            ],
            max_turns: Some(30),
            config_schema: Some(json!({
                "type": "object",
                "properties": {
                    "max_candidates": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10,
                        "description": "Maximum number of models to probe."
                    },
                    "max_spend_usd": {
                        "type": "number",
                        "minimum": 0.0,
                        "maximum": 10.0,
                        "default": 0.10,
                        "description": "Maximum total spend in USD across all probes."
                    },
                    "probe_timeout_ms": {
                        "type": "integer",
                        "minimum": 1000,
                        "maximum": 60000,
                        "default": 10000,
                        "description": "Per-probe HTTP timeout in milliseconds."
                    },
                    "probe_tasks": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/ProbeTask" },
                        "description": "Custom probe tasks. If empty, built-in probes are used."
                    },
                    "target_route_key": {
                        "type": "string",
                        "default": "base",
                        "description": "Router route key to update in the proposal."
                    }
                },
                "$defs": {
                    "ProbeTask": {
                        "type": "object",
                        "required": ["id", "prompt"],
                        "properties": {
                            "id": { "type": "string" },
                            "prompt": { "type": "string" },
                            "checks": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Check IDs: not_empty, max_latency_5s, max_latency_10s"
                            }
                        }
                    }
                }
            })),
        }]
    }
}

const SCOUT_SYSTEM_PROMPT: &str = "\
You are the OpenRouter Model Scout. Your job is to benchmark OpenRouter \
models/providers against a set of probe tasks and recommend model-router updates.

Workflow:
1. Call list_openrouter_catalog to get available models with metadata. Apply any \
   filters from your config (capability requirements, cost ceilings).
2. Select up to max_candidates models to probe (respect the cost budget).
3. For each candidate, call probe_model with each probe task. Track cumulative \
   estimated cost; stop probing if you approach max_spend_usd.
4. Call rank_models with all collected probe results.
5. Call propose_router_update with the rankings to generate a model-router proposal.
6. Present a clear summary: top-3 ranked models, key trade-offs, and the router \
   update proposal. Explain why the top model was chosen.

Guard rails:
- Never probe more than max_candidates models.
- Stop probing if cumulative estimated cost reaches max_spend_usd.
- If a probe fails or times out, record the error and continue with remaining candidates.
- The proposal is advisory — do NOT apply it automatically.";

// ============================================================================
// Shared types
// ============================================================================

/// A single probe task definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeTask {
    pub id: String,
    pub prompt: String,
    /// Check IDs to evaluate on the response: `not_empty`, `max_latency_5s`, `max_latency_10s`.
    #[serde(default)]
    pub checks: Vec<String>,
}

/// Result of probing one model with one task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub model_id: String,
    pub task_id: String,
    pub success: bool,
    pub latency_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Estimated cost in USD (may be None when usage data is unavailable).
    pub cost_usd: Option<f64>,
    pub error: Option<String>,
    pub passed_checks: Vec<String>,
    pub failed_checks: Vec<String>,
}

/// Aggregated score for one model across all probe tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRanking {
    pub model_id: String,
    pub display_name: Option<String>,
    pub success_rate: f64,
    pub avg_latency_ms: f64,
    pub total_cost_usd: f64,
    pub probe_count: usize,
    /// Composite score in [0, 1]: higher is better.
    pub score: f64,
}

/// Update proposal for a model router route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterUpdateProposal {
    pub route_key: String,
    /// Ordered list — first model is the primary, remainder are fallbacks.
    pub proposed_candidates: Vec<String>,
    pub rationale: String,
}

// ============================================================================
// Ranking logic (network-free, fully unit-tested)
// ============================================================================

const MAX_LATENCY_BASELINE_MS: f64 = 15_000.0;

/// Compute a composite score for a model.
///
/// Weights: success_rate 50 %, latency 30 %, cost efficiency 20 %.
/// `max_cost_usd` is the highest per-probe cost observed across all candidates,
/// used to normalise within the cohort.
pub fn compute_score(ranking: &ModelRanking, max_cost_per_probe_usd: f64) -> f64 {
    let success = ranking.success_rate;
    let latency_norm = (ranking.avg_latency_ms / MAX_LATENCY_BASELINE_MS).min(1.0);
    let latency_score = 1.0 - latency_norm;
    let cost_per_probe = if ranking.probe_count > 0 {
        ranking.total_cost_usd / ranking.probe_count as f64
    } else {
        0.0
    };
    let cost_norm = if max_cost_per_probe_usd > 0.0 {
        (cost_per_probe / max_cost_per_probe_usd).min(1.0)
    } else {
        0.0
    };
    let cost_score = 1.0 - cost_norm;
    success * 0.50 + latency_score * 0.30 + cost_score * 0.20
}

/// Rank a list of probe results, returning candidates sorted by composite score
/// (highest first). Only includes models with at least one probe result.
pub fn rank_results(results: &[ProbeResult]) -> Vec<ModelRanking> {
    use std::collections::HashMap;

    // Aggregate per model
    let mut by_model: HashMap<&str, Vec<&ProbeResult>> = HashMap::new();
    for r in results {
        by_model.entry(&r.model_id).or_default().push(r);
    }

    let mut rankings: Vec<ModelRanking> = by_model
        .into_iter()
        .map(|(model_id, probes)| {
            let probe_count = probes.len();
            let success_count = probes.iter().filter(|p| p.success).count();
            let success_rate = success_count as f64 / probe_count as f64;
            let avg_latency_ms =
                probes.iter().map(|p| p.latency_ms as f64).sum::<f64>() / probe_count as f64;
            let total_cost_usd = probes.iter().filter_map(|p| p.cost_usd).sum::<f64>();
            ModelRanking {
                model_id: model_id.to_string(),
                display_name: None,
                success_rate,
                avg_latency_ms,
                total_cost_usd,
                probe_count,
                score: 0.0, // filled below
            }
        })
        .collect();

    let max_cost = rankings
        .iter()
        .map(|r| {
            if r.probe_count > 0 {
                r.total_cost_usd / r.probe_count as f64
            } else {
                0.0
            }
        })
        .fold(0.0f64, f64::max);

    for r in &mut rankings {
        r.score = compute_score(r, max_cost);
    }

    rankings.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rankings
}

// ============================================================================
// Default probe tasks
// ============================================================================

fn default_probe_tasks() -> Vec<ProbeTask> {
    vec![
        ProbeTask {
            id: "basic_response".to_string(),
            prompt: "Reply with exactly: OK".to_string(),
            checks: vec!["not_empty".to_string()],
        },
        ProbeTask {
            id: "tool_hint".to_string(),
            prompt: "You have a tool called `get_weather(city: string)`. If someone asks for London weather, what tool call would you make? Reply only with the JSON object: {\"name\": \"...\", \"arguments\": {...}}".to_string(),
            checks: vec!["not_empty".to_string(), "max_latency_10s".to_string()],
        },
        ProbeTask {
            id: "json_output".to_string(),
            prompt: "Return a JSON object with keys `status` (string \"ok\") and `value` (integer 42). Reply only with the JSON.".to_string(),
            checks: vec!["not_empty".to_string(), "max_latency_10s".to_string()],
        },
    ]
}

// ============================================================================
// Tool: ListOpenRouterCatalogTool
// ============================================================================

struct ListOpenRouterCatalogTool;

#[async_trait]
impl Tool for ListOpenRouterCatalogTool {
    fn name(&self) -> &str {
        "list_openrouter_catalog"
    }

    fn description(&self) -> &str {
        "Fetch the OpenRouter model catalog. Returns model IDs, display names, pricing, and supported capabilities. Use this to select candidates for probing."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "default": 50,
                    "description": "Maximum number of models to return."
                },
                "require_tools": {
                    "type": "boolean",
                    "default": false,
                    "description": "If true, only return models that advertise tool/function-calling support."
                },
                "require_json": {
                    "type": "boolean",
                    "default": false,
                    "description": "If true, only return models that advertise JSON/response_format support."
                },
                "max_prompt_price_per_million": {
                    "type": "number",
                    "description": "Filter: exclude models with prompt price above this USD/M-token threshold."
                }
            }
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "list_openrouter_catalog requires provider credentials — use execute_with_context",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let max_results = arguments
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;
        let require_tools = arguments
            .get("require_tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let require_json = arguments
            .get("require_json")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_price = arguments
            .get("max_prompt_price_per_million")
            .and_then(|v| v.as_f64());

        let api_key = match resolve_openrouter_key(context).await {
            Ok(k) => k,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let client = reqwest::Client::new();
        let response = match client
            .get("https://openrouter.ai/api/v1/models")
            .bearer_auth(&api_key)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to fetch OpenRouter catalog: {e}"
                ));
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return ToolExecutionResult::tool_error(format!(
                "OpenRouter /models returned HTTP {status}: {body}"
            ));
        }

        let body: Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to parse OpenRouter catalog response: {e}"
                ));
            }
        };

        let models = match body.get("data").and_then(|d| d.as_array()) {
            Some(arr) => arr,
            None => {
                return ToolExecutionResult::tool_error(
                    "OpenRouter catalog response missing 'data' array".to_string(),
                );
            }
        };

        // Sort raw models by name first so filter+truncation is deterministic
        // regardless of the order OpenRouter returns them.
        let mut sorted_models: Vec<&Value> = models.iter().collect();
        sorted_models.sort_by(|a, b| {
            let na = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let nb = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
            na.cmp(nb)
        });

        let entries: Vec<Value> = sorted_models
            .iter()
            .filter(|m| {
                let params: Vec<String> = m
                    .get("supported_parameters")
                    .and_then(|p| p.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();

                if require_tools && !params.iter().any(|p| p == "tools") {
                    return false;
                }
                if require_json && !params.iter().any(|p| p == "response_format") {
                    return false;
                }
                if let Some(max_p) = max_price {
                    // Models with missing or unparseable pricing are excluded when a
                    // price ceiling is active to avoid silently letting expensive or
                    // unknown-cost models through.
                    let prompt_price: Option<f64> = m
                        .get("pricing")
                        .and_then(|pr| pr.get("prompt"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok());
                    match prompt_price {
                        // OpenRouter reports price per token; convert to per-million
                        Some(p) if p * 1_000_000.0 <= max_p => {}
                        _ => return false,
                    }
                }
                true
            })
            .take(max_results)
            .map(|m| {
                json!({
                    "id": m.get("id").cloned().unwrap_or(Value::Null),
                    "name": m.get("name").cloned().unwrap_or(Value::Null),
                    "context_length": m.get("context_length").cloned().unwrap_or(Value::Null),
                    "prompt_price_per_token": m.get("pricing").and_then(|p| p.get("prompt")).cloned().unwrap_or(Value::Null),
                    "completion_price_per_token": m.get("pricing").and_then(|p| p.get("completion")).cloned().unwrap_or(Value::Null),
                    "supported_parameters": m.get("supported_parameters").cloned().unwrap_or(json!([])),
                })
            })
            .collect();

        ToolExecutionResult::success(json!({
            "total_returned": entries.len(),
            "models": entries,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: ProbeModelTool
// ============================================================================

struct ProbeModelTool;

#[async_trait]
impl Tool for ProbeModelTool {
    fn name(&self) -> &str {
        "probe_model"
    }

    fn description(&self) -> &str {
        "Run one or more probe tasks against an OpenRouter model and return latency, success, token usage, and cost signals. Use built-in probes or supply custom tasks."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["model_id"],
            "properties": {
                "model_id": {
                    "type": "string",
                    "description": "OpenRouter model ID (e.g. 'openai/gpt-4o-mini')."
                },
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["id", "prompt"],
                        "properties": {
                            "id": { "type": "string" },
                            "prompt": { "type": "string" },
                            "checks": {
                                "type": "array",
                                "items": { "type": "string" }
                            }
                        }
                    },
                    "description": "Probe tasks to run. If omitted, built-in probes are used."
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1000,
                    "maximum": 60000,
                    "default": 10000,
                    "description": "HTTP timeout per probe in milliseconds."
                },
                "max_spend_usd": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 10.0,
                    "default": 0.10,
                    "description": "Maximum observed spend in USD before stopping further probes."
                }
            }
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "probe_model requires provider credentials — use execute_with_context",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let model_id = match arguments.get("model_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolExecutionResult::tool_error("model_id is required"),
        };

        let timeout_ms = bounded_probe_timeout_ms(&arguments);
        let max_spend_usd = bounded_probe_max_spend_usd(&arguments);

        let tasks: Vec<ProbeTask> = match arguments.get("tasks") {
            Some(Value::Array(arr)) if !arr.is_empty() => {
                // Cap the JSON array *before* cloning/deserializing so an
                // adversarial multi-million-element `tasks` payload can't force
                // unbounded allocation/CPU ahead of the limit (TM-DOS).
                let capped: Vec<Value> = arr.iter().take(MAX_PROBE_TASKS).cloned().collect();
                match serde_json::from_value(Value::Array(capped)) {
                    Ok(t) => limit_probe_tasks(t),
                    Err(e) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid probe tasks: {e}"
                        ));
                    }
                }
            }
            _ => default_probe_tasks(),
        };

        let api_key = match resolve_openrouter_key(context).await {
            Ok(k) => k,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to build HTTP client: {e}"
                ));
            }
        };

        let mut results: Vec<ProbeResult> = Vec::new();
        let mut observed_spend_usd = 0.0;

        for task in &tasks {
            // Check the budget before each probe so a zero budget runs zero
            // (paid) probes rather than always paying for the first one.
            if observed_spend_usd >= max_spend_usd {
                break;
            }
            let result = run_probe(&client, &api_key, &model_id, task).await;
            if let Some(cost_usd) = result.cost_usd {
                observed_spend_usd += cost_usd;
            }
            results.push(result);
        }

        let result_values: Vec<Value> = results
            .iter()
            .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
            .collect();

        ToolExecutionResult::success(json!({
            "model_id": model_id,
            "results": result_values,
            "observed_spend_usd": observed_spend_usd,
            "max_spend_usd": max_spend_usd,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

fn bounded_probe_timeout_ms(arguments: &Value) -> u64 {
    arguments
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_PROBE_TIMEOUT_MS)
        .clamp(MIN_PROBE_TIMEOUT_MS, MAX_PROBE_TIMEOUT_MS)
}

fn bounded_probe_max_spend_usd(arguments: &Value) -> f64 {
    let spend = arguments
        .get("max_spend_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(DEFAULT_MAX_SPEND_USD);

    if spend.is_finite() {
        spend.clamp(0.0, MAX_PROBE_SPEND_USD)
    } else {
        DEFAULT_MAX_SPEND_USD
    }
}

fn limit_probe_tasks(tasks: Vec<ProbeTask>) -> Vec<ProbeTask> {
    tasks.into_iter().take(MAX_PROBE_TASKS).collect()
}

/// Run one probe task against a model and return the result.
async fn run_probe(
    client: &reqwest::Client,
    api_key: &str,
    model_id: &str,
    task: &ProbeTask,
) -> ProbeResult {
    let start = std::time::Instant::now();

    let payload = json!({
        "model": model_id,
        "messages": [{"role": "user", "content": task.prompt}],
        "max_tokens": 256,
        // Ask OpenRouter to return generation accounting so `usage.cost` is
        // populated; without this the spend guardrail never observes any cost
        // and the per-call budget cap is a no-op.
        "usage": { "include": true },
    });

    let response = match client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return ProbeResult {
                model_id: model_id.to_string(),
                task_id: task.id.clone(),
                success: false,
                latency_ms: start.elapsed().as_millis() as u64,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
                error: Some(format!("Request failed: {e}")),
                passed_checks: vec![],
                failed_checks: task.checks.clone(),
            };
        }
    };

    let latency_ms = start.elapsed().as_millis() as u64;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return ProbeResult {
            model_id: model_id.to_string(),
            task_id: task.id.clone(),
            success: false,
            latency_ms,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            error: Some(format!("HTTP {status}: {body}")),
            passed_checks: vec![],
            failed_checks: task.checks.clone(),
        };
    }

    let body: Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            return ProbeResult {
                model_id: model_id.to_string(),
                task_id: task.id.clone(),
                success: false,
                latency_ms,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
                error: Some(format!("Failed to parse response: {e}")),
                passed_checks: vec![],
                failed_checks: task.checks.clone(),
            };
        }
    };

    let content = body
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|ch| ch.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    let input_tokens = body
        .get("usage")
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64());
    let output_tokens = body
        .get("usage")
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_u64());
    let cost_usd = body
        .get("usage")
        .and_then(|u| u.get("cost"))
        .and_then(|v| v.as_f64());

    // Evaluate checks
    let mut passed = vec![];
    let mut failed = vec![];
    for check in &task.checks {
        let ok = match check.as_str() {
            "not_empty" => !content.trim().is_empty(),
            "max_latency_5s" => latency_ms <= 5_000,
            "max_latency_10s" => latency_ms <= 10_000,
            // Unknown check IDs fail so typos in task configs surface immediately
            // rather than silently inflating success rates.
            _ => false,
        };
        if ok {
            passed.push(check.clone());
        } else {
            failed.push(check.clone());
        }
    }

    let success = failed.is_empty() && !content.trim().is_empty();

    ProbeResult {
        model_id: model_id.to_string(),
        task_id: task.id.clone(),
        success,
        latency_ms,
        input_tokens,
        output_tokens,
        cost_usd,
        error: None,
        passed_checks: passed,
        failed_checks: failed,
    }
}

// ============================================================================
// Tool: RankModelsTool
// ============================================================================

struct RankModelsTool;

#[async_trait]
impl Tool for RankModelsTool {
    fn name(&self) -> &str {
        "rank_models"
    }

    fn description(&self) -> &str {
        "Aggregate probe results and rank models by composite score (success rate, latency, cost). Returns candidates sorted highest-score first."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["results"],
            "properties": {
                "results": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "ProbeResult objects returned by probe_model."
                }
            }
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let raw_results = match arguments.get("results").and_then(|v| v.as_array()) {
            Some(arr) => arr.clone(),
            None => return ToolExecutionResult::tool_error("results array is required"),
        };

        let probe_results: Vec<ProbeResult> =
            match serde_json::from_value(Value::Array(raw_results)) {
                Ok(r) => r,
                Err(e) => return ToolExecutionResult::tool_error(format!("Invalid results: {e}")),
            };

        if probe_results.is_empty() {
            return ToolExecutionResult::success(json!({ "rankings": [] }));
        }

        let rankings = rank_results(&probe_results);
        let out: Vec<Value> = rankings
            .iter()
            .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
            .collect();

        ToolExecutionResult::success(json!({ "rankings": out }))
    }
}

// ============================================================================
// Tool: ProposeRouterUpdateTool
// ============================================================================

struct ProposeRouterUpdateTool;

#[async_trait]
impl Tool for ProposeRouterUpdateTool {
    fn name(&self) -> &str {
        "propose_router_update"
    }

    fn description(&self) -> &str {
        "Generate a model-router update proposal from ranked results. The proposal lists an ordered candidate set (primary + fallbacks) for the target route key. Applying it is always an explicit operator step."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["rankings"],
            "properties": {
                "rankings": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "ModelRanking objects from rank_models."
                },
                "route_key": {
                    "type": "string",
                    "default": "base",
                    "description": "Router route key to update."
                },
                "top_n": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10,
                    "default": 3,
                    "description": "Number of candidates to include (primary + N-1 fallbacks)."
                }
            }
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let raw_rankings = match arguments.get("rankings").and_then(|v| v.as_array()) {
            Some(arr) => arr.clone(),
            None => return ToolExecutionResult::tool_error("rankings array is required"),
        };

        let rankings: Vec<ModelRanking> = match serde_json::from_value(Value::Array(raw_rankings)) {
            Ok(r) => r,
            Err(e) => return ToolExecutionResult::tool_error(format!("Invalid rankings: {e}")),
        };

        if rankings.is_empty() {
            return ToolExecutionResult::tool_error(
                "No ranked models provided — run rank_models first",
            );
        }

        let route_key = arguments
            .get("route_key")
            .and_then(|v| v.as_str())
            .unwrap_or("base")
            .to_string();

        let top_n = arguments.get("top_n").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

        // Sort defensively — callers may provide unsorted input.
        let mut sorted = rankings;
        sorted.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let top_models: Vec<String> = sorted
            .iter()
            .take(top_n)
            .map(|r| r.model_id.clone())
            .collect();

        let best = &sorted[0];
        let rationale = format!(
            "Top model '{}' scored {:.3} (success_rate={:.0}%, avg_latency={:.0}ms, total_cost_usd={:.4}). \
             {} candidates probed total.",
            best.model_id,
            best.score,
            best.success_rate * 100.0,
            best.avg_latency_ms,
            best.total_cost_usd,
            sorted.len(),
        );

        let proposal = RouterUpdateProposal {
            route_key,
            proposed_candidates: top_models,
            rationale,
        };

        ToolExecutionResult::success(serde_json::to_value(&proposal).unwrap_or(Value::Null))
    }
}

// ============================================================================
// Credential helper
// ============================================================================

async fn resolve_openrouter_key(context: &ToolContext) -> Result<String, String> {
    let store = context
        .provider_credential_store
        .as_ref()
        .ok_or_else(|| "No provider credential store available".to_string())?;

    let creds = store
        .get_default_provider_credentials("openrouter")
        .await
        .map_err(|e| format!("Failed to resolve OpenRouter credentials: {e}"))?
        .ok_or_else(|| {
            "No OpenRouter provider configured — add an OpenRouter provider to your org".to_string()
        })?;

    Ok(creds.api_key)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_probe(
        model_id: &str,
        task_id: &str,
        success: bool,
        latency_ms: u64,
        cost: Option<f64>,
    ) -> ProbeResult {
        ProbeResult {
            model_id: model_id.to_string(),
            task_id: task_id.to_string(),
            success,
            latency_ms,
            input_tokens: Some(10),
            output_tokens: Some(20),
            cost_usd: cost,
            error: if success {
                None
            } else {
                Some("error".to_string())
            },
            passed_checks: if success {
                vec!["not_empty".to_string()]
            } else {
                vec![]
            },
            failed_checks: if success {
                vec![]
            } else {
                vec!["not_empty".to_string()]
            },
        }
    }

    #[test]
    fn rank_results_orders_by_score() {
        let results = vec![
            // slow_model: 1/3 success, 12s avg, cheap
            make_probe("slow_model", "t1", false, 12_000, Some(0.0001)),
            make_probe("slow_model", "t2", false, 11_000, Some(0.0001)),
            make_probe("slow_model", "t3", true, 13_000, Some(0.0001)),
            // fast_model: 3/3 success, 500ms avg, more expensive
            make_probe("fast_model", "t1", true, 500, Some(0.001)),
            make_probe("fast_model", "t2", true, 400, Some(0.001)),
            make_probe("fast_model", "t3", true, 600, Some(0.001)),
        ];

        let rankings = rank_results(&results);
        assert_eq!(rankings.len(), 2);
        // fast_model should win despite higher cost, because success rate dominates
        assert_eq!(rankings[0].model_id, "fast_model");
        assert_eq!(rankings[1].model_id, "slow_model");
        assert!(rankings[0].score > rankings[1].score);
    }

    #[test]
    fn rank_results_empty_input_returns_empty() {
        assert!(rank_results(&[]).is_empty());
    }

    #[test]
    fn rank_results_single_model() {
        let results = vec![
            make_probe("only_model", "t1", true, 1_000, Some(0.001)),
            make_probe("only_model", "t2", true, 2_000, Some(0.001)),
        ];
        let rankings = rank_results(&results);
        assert_eq!(rankings.len(), 1);
        assert_eq!(rankings[0].model_id, "only_model");
        assert_eq!(rankings[0].success_rate, 1.0);
        assert_eq!(rankings[0].probe_count, 2);
    }

    #[test]
    fn compute_score_perfect_is_high() {
        let r = ModelRanking {
            model_id: "m".to_string(),
            display_name: None,
            success_rate: 1.0,
            avg_latency_ms: 100.0,
            total_cost_usd: 0.0001,
            probe_count: 3,
            score: 0.0,
        };
        let s = compute_score(&r, 0.01);
        // success=1.0*0.5 + latency=(1-100/15000)*0.3 + cost=1.0*0.2
        assert!(s > 0.9, "perfect model should score above 0.9, got {s}");
    }

    #[test]
    fn compute_score_zero_success_is_low() {
        let r = ModelRanking {
            model_id: "m".to_string(),
            display_name: None,
            success_rate: 0.0,
            avg_latency_ms: 15_000.0,
            total_cost_usd: 1.0,
            probe_count: 5,
            score: 0.0,
        };
        let s = compute_score(&r, 0.2);
        // 0.0*0.5 + 0.0*0.3 + 0.0*0.2 = 0.0
        assert_eq!(
            s, 0.0,
            "zero success + max latency + max cost should score 0"
        );
    }

    #[test]
    fn compute_score_cost_zero_max_cost_zero() {
        // When max_cost_per_probe is 0, cost_score should be 1.0 (no penalty)
        let r = ModelRanking {
            model_id: "m".to_string(),
            display_name: None,
            success_rate: 1.0,
            avg_latency_ms: 0.0,
            total_cost_usd: 0.0,
            probe_count: 1,
            score: 0.0,
        };
        let s = compute_score(&r, 0.0);
        assert!((s - 1.0).abs() < 1e-9, "score should be 1.0, got {s}");
    }

    #[test]
    fn rank_results_prefers_lower_latency_among_equal_success() {
        let results = vec![
            make_probe("model_a", "t1", true, 5_000, Some(0.001)),
            make_probe("model_b", "t1", true, 1_000, Some(0.001)),
        ];
        let rankings = rank_results(&results);
        assert_eq!(
            rankings[0].model_id, "model_b",
            "lower latency should rank higher when success rates are equal"
        );
    }

    #[test]
    fn default_probe_tasks_non_empty() {
        let tasks = default_probe_tasks();
        assert!(!tasks.is_empty());
        for t in &tasks {
            assert!(!t.id.is_empty());
            assert!(!t.prompt.is_empty());
        }
    }

    #[test]
    fn model_scout_capability_is_high_risk() {
        assert_eq!(ModelScoutCapability.risk_level(), RiskLevel::High);
    }

    #[test]
    fn probe_timeout_is_clamped_to_schema_bounds() {
        assert_eq!(
            bounded_probe_timeout_ms(&json!({})),
            DEFAULT_PROBE_TIMEOUT_MS
        );
        assert_eq!(
            bounded_probe_timeout_ms(&json!({ "timeout_ms": 1 })),
            MIN_PROBE_TIMEOUT_MS
        );
        assert_eq!(
            bounded_probe_timeout_ms(&json!({ "timeout_ms": 3_600_000 })),
            MAX_PROBE_TIMEOUT_MS
        );
    }

    #[test]
    fn probe_spend_is_clamped_to_schema_bounds() {
        assert_eq!(
            bounded_probe_max_spend_usd(&json!({})),
            DEFAULT_MAX_SPEND_USD
        );
        assert_eq!(
            bounded_probe_max_spend_usd(&json!({ "max_spend_usd": -1.0 })),
            0.0
        );
        assert_eq!(
            bounded_probe_max_spend_usd(&json!({ "max_spend_usd": 1_000.0 })),
            MAX_PROBE_SPEND_USD
        );
    }

    #[test]
    fn custom_probe_tasks_are_capped() {
        let tasks: Vec<ProbeTask> = (0..75)
            .map(|i| ProbeTask {
                id: format!("task_{i}"),
                prompt: "Reply OK".to_string(),
                checks: vec![],
            })
            .collect();

        let limited = limit_probe_tasks(tasks);
        assert_eq!(limited.len(), MAX_PROBE_TASKS);
        assert_eq!(limited[0].id, "task_0");
        assert_eq!(limited[MAX_PROBE_TASKS - 1].id, "task_49");
    }

    #[tokio::test]
    async fn rank_models_tool_returns_sorted_rankings() {
        let tool = RankModelsTool;
        let results = json!([
            {
                "model_id": "cheap",
                "task_id": "t1",
                "success": true,
                "latency_ms": 800,
                "input_tokens": 10,
                "output_tokens": 10,
                "cost_usd": 0.0001,
                "error": null,
                "passed_checks": ["not_empty"],
                "failed_checks": []
            },
            {
                "model_id": "expensive",
                "task_id": "t1",
                "success": true,
                "latency_ms": 800,
                "input_tokens": 10,
                "output_tokens": 10,
                "cost_usd": 0.1,
                "error": null,
                "passed_checks": ["not_empty"],
                "failed_checks": []
            }
        ]);

        let out = tool.execute(json!({ "results": results })).await;
        assert!(out.is_success());
        let tool_result = out.into_tool_result("id", "rank_models");
        let content = tool_result.result.unwrap();
        let rankings = content["rankings"].as_array().unwrap();
        assert_eq!(rankings.len(), 2);
        // cheap should score higher due to lower cost
        assert_eq!(rankings[0]["model_id"].as_str().unwrap(), "cheap");
    }

    #[tokio::test]
    async fn propose_router_update_tool_produces_proposal() {
        let tool = ProposeRouterUpdateTool;
        let rankings = json!([
            {
                "model_id": "best_model",
                "display_name": null,
                "success_rate": 1.0,
                "avg_latency_ms": 500.0,
                "total_cost_usd": 0.001,
                "probe_count": 3,
                "score": 0.95
            },
            {
                "model_id": "fallback_model",
                "display_name": null,
                "success_rate": 0.66,
                "avg_latency_ms": 2000.0,
                "total_cost_usd": 0.0005,
                "probe_count": 3,
                "score": 0.60
            }
        ]);

        let out = tool
            .execute(json!({
                "rankings": rankings,
                "route_key": "base",
                "top_n": 2
            }))
            .await;

        assert!(out.is_success());
        let tool_result = out.into_tool_result("id", "propose_router_update");
        let content = tool_result.result.unwrap();
        assert_eq!(content["route_key"].as_str().unwrap(), "base");
        let candidates = content["proposed_candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].as_str().unwrap(), "best_model");
        assert_eq!(candidates[1].as_str().unwrap(), "fallback_model");
    }

    #[tokio::test]
    async fn propose_router_update_empty_rankings_returns_error() {
        let tool = ProposeRouterUpdateTool;
        let out = tool.execute(json!({ "rankings": [] })).await;
        assert!(out.is_error(), "empty rankings should return error");
    }
}
