//! Parallel machine-payment capability.
//!
//! Decision: expose only Parallel-specific tools. Payment mechanics stay inside
//! `PaymentAuthority`, so the model cannot initiate arbitrary paid HTTP calls.
//! Decision: this lives in the Parallel integration crate, not core. Core owns the
//! payment trust boundary (`PaymentAuthority`, payment DTOs, `ToolContext`); the
//! vendor-specific paid adapter is a plugin gated by the `machine_payments`
//! internal feature flag.

use async_trait::async_trait;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel,
};
use everruns_core::payment::{MachinePaymentRequest, PaymentMethod, PaymentRail};
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_narration::{
    generic_phrase, labeled_phrase, safe_arg_str, truncate, url_display,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_provider::tool_types::ToolHints;
use serde::Deserialize;
use serde_json::{Value, json};

const PARALLEL_BASE_URL: &str = "https://parallelmpp.dev";

pub struct ParallelPaymentsCapability;

#[async_trait]
impl Capability for ParallelPaymentsCapability {
    fn id(&self) -> &str {
        "parallel"
    }

    fn name(&self) -> &str {
        "Parallel (Machine Payments)"
    }

    fn description(&self) -> &str {
        "Paid Parallel search, extract, and async task tools backed by Everruns machine payments."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("wallet")
    }

    fn category(&self) -> Option<&str> {
        Some("Machine Payments")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "When web research is needed and Parallel tools are available, prefer the Parallel tools for structured paid search/extract/task work. Use `parallel_task_status` to poll async task run IDs until complete.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ParallelSearchTool),
            Box::new(ParallelExtractTool),
            Box::new(ParallelTaskTool),
            Box::new(ParallelTaskStatusTool),
        ]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["machine_payments"]
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Parallel (машинні платежі)",
            "Платні інструменти Parallel для пошуку, вилучення даних та асинхронних завдань \
             на основі машинних платежів Everruns.",
        )]
    }
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default = "default_search_mode")]
    mode: String,
}

fn default_search_mode() -> String {
    "one-shot".to_string()
}

pub struct ParallelSearchTool;

#[async_trait]
impl Tool for ParallelSearchTool {
    fn narrate(
        &self,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let query = safe_arg_str(&tool_call.arguments, &["query", "q", "objective"])
            .map(|value| truncate(value, 48));
        Some(labeled_phrase(
            "Searching Parallel",
            "Searched Parallel",
            "Could not search Parallel",
            query,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "parallel_search"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Parallel Search")
    }

    fn description(&self) -> &str {
        "Search the web through Parallel's paid API. Costs up to $0.01 per call."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query." },
                "mode": {
                    "type": "string",
                    "enum": ["one-shot", "fast"],
                    "description": "Use one-shot for comprehensive results or fast for lower latency.",
                    "default": "one-shot"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        missing_payment_authority()
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let args: SearchArgs = match serde_json::from_value(arguments) {
            Ok(args) => args,
            Err(error) => return invalid_args(error),
        };
        if !matches!(args.mode.as_str(), "one-shot" | "fast") {
            return ToolExecutionResult::tool_error("mode must be one of: one-shot, fast");
        }
        execute_paid_parallel(
            context,
            "search",
            "/api/search",
            json!({ "query": args.query, "mode": args.mode }),
            0.01,
        )
        .await
    }
}

#[derive(Debug, Deserialize)]
struct ExtractArgs {
    urls: Vec<String>,
    objective: String,
}

pub struct ParallelExtractTool;

#[async_trait]
impl Tool for ParallelExtractTool {
    fn narrate(
        &self,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // The schema arg `urls` is an array; fall back to its first element
        // (also accept a scalar `url` alias). Rendered via url_display so any
        // embedded credentials/query strings are stripped.
        let url = safe_arg_str(&tool_call.arguments, &["url"])
            .map(str::to_string)
            .or_else(|| {
                tool_call
                    .arguments
                    .get("urls")
                    .and_then(Value::as_array)
                    .and_then(|items| items.first())
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .map(|value| url_display(&value))
            .filter(|value| !value.is_empty());
        Some(labeled_phrase(
            "Extracting URL",
            "Extracted URL",
            "Could not extract URL",
            url,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "parallel_extract"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Parallel Extract")
    }

    fn description(&self) -> &str {
        "Extract structured facts from URLs through Parallel's paid API. Costs up to $0.01 per URL, minimum $0.01."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "description": "URLs to extract from."
                },
                "objective": { "type": "string", "description": "What facts to extract." }
            },
            "required": ["urls", "objective"],
            "additionalProperties": false
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        missing_payment_authority()
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let args: ExtractArgs = match serde_json::from_value(arguments) {
            Ok(args) => args,
            Err(error) => return invalid_args(error),
        };
        if args.urls.is_empty() {
            return ToolExecutionResult::tool_error("urls must contain at least one URL");
        }
        let max_amount_usd = (args.urls.len() as f64 * 0.01).max(0.01);
        execute_paid_parallel(
            context,
            "extract",
            "/api/extract",
            json!({ "urls": args.urls, "objective": args.objective }),
            max_amount_usd,
        )
        .await
    }
}

#[derive(Debug, Deserialize)]
struct TaskArgs {
    input: String,
    #[serde(default = "default_processor")]
    processor: String,
}

fn default_processor() -> String {
    "ultra".to_string()
}

pub struct ParallelTaskTool;

#[async_trait]
impl Tool for ParallelTaskTool {
    fn narrate(
        &self,
        _tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(generic_phrase(
            "Running Parallel task",
            "Ran Parallel task",
            "Failed to run Parallel task",
            None,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "parallel_task"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Parallel Task")
    }

    fn description(&self) -> &str {
        "Start a deep async Parallel task. Costs up to $0.10 for pro or $0.30 for ultra; poll with parallel_task_status."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "input": { "type": "string", "description": "Task input." },
                "processor": {
                    "type": "string",
                    "enum": ["pro", "ultra"],
                    "default": "ultra"
                }
            },
            "required": ["input"],
            "additionalProperties": false
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        missing_payment_authority()
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let args: TaskArgs = match serde_json::from_value(arguments) {
            Ok(args) => args,
            Err(error) => return invalid_args(error),
        };
        let max_amount_usd = match args.processor.as_str() {
            "pro" => 0.10,
            "ultra" => 0.30,
            _ => return ToolExecutionResult::tool_error("processor must be one of: pro, ultra"),
        };
        execute_paid_parallel(
            context,
            "task",
            "/api/task",
            json!({ "input": args.input, "processor": args.processor }),
            max_amount_usd,
        )
        .await
    }
}

#[derive(Debug, Deserialize)]
struct TaskStatusArgs {
    run_id: String,
}

pub struct ParallelTaskStatusTool;

#[async_trait]
impl Tool for ParallelTaskStatusTool {
    fn narrate(
        &self,
        _tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // Bare: the run id is not user-friendly.
        Some(generic_phrase(
            "Checking task status",
            "Checked task status",
            "Failed to check task status",
            None,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "parallel_task_status"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Parallel Task Status")
    }

    fn description(&self) -> &str {
        "Poll a Parallel task run. This endpoint is free and does not require payment."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "Run ID returned by parallel_task." }
            },
            "required": ["run_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let args: TaskStatusArgs = match serde_json::from_value(arguments) {
            Ok(args) => args,
            Err(error) => return invalid_args(error),
        };
        let run_id = args.run_id.trim();
        if run_id.is_empty() || run_id.contains('/') {
            return ToolExecutionResult::tool_error("run_id is invalid");
        }
        let url = format!("{PARALLEL_BASE_URL}/api/task/{run_id}");
        match reqwest::Client::new().get(url).send().await {
            Ok(response) => match response.json::<Value>().await {
                Ok(value) => ToolExecutionResult::success(value),
                Err(error) => ToolExecutionResult::tool_error(format!(
                    "Parallel task status response was not valid JSON: {error}"
                )),
            },
            Err(error) => ToolExecutionResult::tool_error(format!(
                "Failed to poll Parallel task status: {error}"
            )),
        }
    }
}

async fn execute_paid_parallel(
    context: &ToolContext,
    operation: &str,
    path: &str,
    body: Value,
    max_amount_usd: f64,
) -> ToolExecutionResult {
    let Some(authority) = context.payment_authority.as_ref() else {
        return missing_payment_authority();
    };

    let request = MachinePaymentRequest {
        capability: "parallel".to_string(),
        operation: operation.to_string(),
        method: PaymentMethod::Post,
        url: format!("{PARALLEL_BASE_URL}{path}"),
        body: Some(body),
        max_amount_usd,
        rail_preference: vec![PaymentRail::X402Base],
        metadata: json!({
            "provider": "parallel",
            "host": "parallelmpp.dev",
            "path": path,
        }),
    };

    match authority
        .execute_machine_payment(context.session_id, request)
        .await
    {
        Ok(response) => ToolExecutionResult::success(json!({
            "result": response.response,
            "payment": {
                "attempt_id": response.attempt_id.map(|id| id.to_string()),
                "amount_usd": response.amount_usd,
                "rail": response.rail.map(|rail| rail.to_string()),
                "receipt": response.receipt,
            }
        })),
        Err(error) => ToolExecutionResult::tool_error(format!("Machine payment failed: {error}")),
    }
}

fn missing_payment_authority() -> ToolExecutionResult {
    ToolExecutionResult::tool_error(
        "Machine payments are not configured for this session. Configure a payment wallet and policy before using Parallel paid tools.",
    )
}

fn invalid_args(error: serde_json::Error) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(format!("Invalid arguments: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata() {
        let cap = ParallelPaymentsCapability;
        assert_eq!(cap.id(), "parallel");
        assert_eq!(cap.category(), Some("Machine Payments"));
        assert_eq!(cap.risk_level(), RiskLevel::High);
        assert_eq!(cap.tools().len(), 4);
        assert_eq!(cap.features(), vec!["machine_payments"]);
    }

    #[tokio::test]
    async fn paid_tools_fail_closed_without_authority() {
        // No payment authority wired -> the paid tools must refuse, not spend.
        let result = ParallelSearchTool
            .execute(json!({ "query": "hello" }))
            .await;
        assert!(result.is_error());
    }

    // ========================================================================
    // Tool narration
    // ========================================================================

    use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
    use everruns_provider::tool_types::ToolCall;

    fn narrate(tool: &dyn Tool, arguments: Value, phase: ToolNarrationPhase) -> Option<String> {
        let call = ToolCall {
            id: "call-1".to_string(),
            name: tool.name().to_string(),
            arguments,
        };
        tool.narrate(&call, phase, None, ToolNarrationContext::default())
    }

    #[test]
    fn narrate_search_all_phases_and_truncation() {
        let tool = ParallelSearchTool;
        assert_eq!(
            narrate(
                &tool,
                json!({"query": "rust async"}),
                ToolNarrationPhase::Started
            )
            .as_deref(),
            Some("Searching Parallel: rust async")
        );
        assert_eq!(
            narrate(
                &tool,
                json!({"query": "rust async"}),
                ToolNarrationPhase::Completed
            )
            .as_deref(),
            Some("Searched Parallel: rust async")
        );
        assert_eq!(
            narrate(
                &tool,
                json!({"query": "rust async"}),
                ToolNarrationPhase::Failed
            )
            .as_deref(),
            Some("Could not search Parallel: rust async")
        );
        // Long query is truncated to 48 chars + ellipsis.
        let long = "a".repeat(80);
        let narration =
            narrate(&tool, json!({ "query": long }), ToolNarrationPhase::Started).unwrap();
        assert!(narration.starts_with("Searching Parallel: "));
        assert!(narration.ends_with("..."));
        // No query arg -> bare verb fallback.
        assert_eq!(
            narrate(&tool, json!({}), ToolNarrationPhase::Started).as_deref(),
            Some("Searching Parallel")
        );
    }

    #[test]
    fn narrate_extract_uses_first_url_and_strips_scheme() {
        let tool = ParallelExtractTool;
        assert_eq!(
            narrate(
                &tool,
                json!({"urls": ["https://example.com/a?x=1", "https://example.com/b"], "objective": "facts"}),
                ToolNarrationPhase::Started
            )
            .as_deref(),
            Some("Extracting URL: example.com/a")
        );
        // No urls -> bare verb fallback.
        assert_eq!(
            narrate(&tool, json!({}), ToolNarrationPhase::Failed).as_deref(),
            Some("Could not extract URL")
        );
    }

    #[test]
    fn narrate_task_status_is_bare() {
        let tool = ParallelTaskStatusTool;
        assert_eq!(
            narrate(
                &tool,
                json!({"run_id": "abc123"}),
                ToolNarrationPhase::Started
            )
            .as_deref(),
            Some("Checking task status")
        );
        assert_eq!(
            narrate(
                &tool,
                json!({"run_id": "abc123"}),
                ToolNarrationPhase::Completed
            )
            .as_deref(),
            Some("Checked task status")
        );
    }

    #[test]
    fn narrate_task_omits_private_input() {
        let tool = ParallelTaskTool;
        assert_eq!(
            narrate(
                &tool,
                json!({"input": "Private analysis with customer token SECRET42"}),
                ToolNarrationPhase::Started
            )
            .as_deref(),
            Some("Running Parallel task")
        );
    }
}
