// Braintrust Event Listener
//
// This listener sends agentic loop events to Braintrust for observability.
// Braintrust provides tracing, logging, and evaluation capabilities for LLM applications.
//
// See specs/braintrust-integration.md for full specification.
//
// API Documentation:
// - Braintrust Docs: https://www.braintrust.dev/docs
// - API Reference: https://www.braintrust.dev/docs/api-reference/introduction
// - Insert Logs: https://www.braintrust.dev/docs/api-reference/logs/insert-project-logs-events
// - List Projects: https://www.braintrust.dev/docs/reference/api/Projects
//
// Configuration via environment variables:
// - BRAINTRUST_API_KEY: API key for authentication (required to enable)
// - BRAINTRUST_PROJECT_NAME: Project name (default: "My Project", resolved to ID via API)
// - BRAINTRUST_PROJECT_ID: Project UUID (alternative, skips name resolution)
// - BRAINTRUST_API_URL: API base URL (default: https://api.braintrust.dev)
//
// Event types traced:
// - turn.started/turn.completed/turn.failed/turn.cancelled - Root span for agentic turn (type: "task")
// - reason.started/reason.completed - LLM reasoning phase within turn (type: "task")
// - act.started/act.completed - Tool execution phase within turn (type: "task")
// - llm.generation - LLM API calls (type: "llm")
// - tool.started/tool.completed - Tool executions (type: "tool")
//
// Parent-child relationships use OTel-style trace_id/span_id/parent_span_id fields.
// trace_id groups all spans in a turn, span_id identifies each span, parent_span_id links to parent.
// Atom-level events (reason, act) provide finer-grained tracing of the agentic loop.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::{
    ACT_COMPLETED, ACT_STARTED, ActCompletedData, ActStartedData, Event, EventData, EventListener,
    LLM_GENERATION, REASON_COMPLETED, REASON_STARTED, REASON_THINKING_COMPLETED,
    REASON_THINKING_STARTED, ReasonCompletedData, ReasonStartedData, ReasonThinkingCompletedData,
    ReasonThinkingStartedData, TOOL_COMPLETED, TOOL_STARTED, TURN_CANCELLED, TURN_COMPLETED,
    TURN_FAILED, TURN_STARTED, ToolStartedData, TurnCancelledData, TurnFailedData,
};

/// Configuration for Braintrust integration
#[derive(Debug, Clone)]
pub struct BraintrustConfig {
    /// API key for authentication
    pub api_key: String,
    /// Project ID (resolved from name if needed)
    pub project_id: String,
    /// API base URL (default: https://api.braintrust.dev)
    pub api_url: String,
}

/// Response from Braintrust list projects API
#[derive(Debug, Deserialize)]
struct ProjectListResponse {
    objects: Vec<Project>,
}

/// Braintrust project
#[derive(Debug, Deserialize)]
struct Project {
    id: String,
    name: String,
}

impl BraintrustConfig {
    /// Load configuration from environment variables
    /// Returns None if BRAINTRUST_API_KEY is not set
    ///
    /// Supports two ways to specify the project:
    /// 1. BRAINTRUST_PROJECT_NAME - Human-readable name (resolved to ID via API)
    /// 2. BRAINTRUST_PROJECT_ID - Direct UUID (no API call needed)
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("BRAINTRUST_API_KEY").ok()?;

        let api_url = std::env::var("BRAINTRUST_API_URL")
            .unwrap_or_else(|_| "https://api.braintrust.dev".to_string());

        // Try project ID first (no API call needed)
        if let Ok(project_id) = std::env::var("BRAINTRUST_PROJECT_ID") {
            return Some(Self {
                api_key,
                project_id,
                api_url,
            });
        }

        // Try project name (requires API call to resolve)
        // Default to "My Project" if not specified (matches Braintrust onboarding default)
        let project_name =
            std::env::var("BRAINTRUST_PROJECT_NAME").unwrap_or_else(|_| "My Project".to_string());

        match resolve_project_id(&api_url, &api_key, &project_name) {
            Ok(project_id) => {
                info!(
                    project_name = %project_name,
                    project_id = %project_id,
                    "Resolved Braintrust project name to ID"
                );
                Some(Self {
                    api_key,
                    project_id,
                    api_url,
                })
            }
            Err(e) => {
                error!(
                    project_name = %project_name,
                    error = %e,
                    "Failed to resolve Braintrust project name"
                );
                None
            }
        }
    }
}

/// Resolve a project name to its ID via the Braintrust API
fn resolve_project_id(api_url: &str, api_key: &str, project_name: &str) -> Result<String, String> {
    // Use block_in_place to run blocking HTTP in async context
    // This is safe during startup before the server starts accepting requests
    tokio::task::block_in_place(|| {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        let url = format!("{}/v1/project?project_name={}", api_url, project_name);

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .map_err(|e| format!("API request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "API returned error: {} {}",
                response.status(),
                response.text().unwrap_or_default()
            ));
        }

        let data: ProjectListResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        // Find exact match by name
        data.objects
            .into_iter()
            .find(|p| p.name == project_name)
            .map(|p| p.id)
            .ok_or_else(|| format!("Project '{}' not found", project_name))
    })
}

/// Braintrust span metrics (token counts and timing)
#[derive(Debug, Serialize)]
struct BraintrustMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_to_first_token: Option<f64>,
    /// Prompt caching: tokens read from cache (Claude, OpenAI)
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read_tokens: Option<u32>,
    /// Prompt caching: tokens written to cache (Claude)
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_creation_tokens: Option<u32>,
}

/// Braintrust span attributes
#[derive(Debug, Serialize)]
struct BraintrustSpanAttributes {
    name: String,
    #[serde(rename = "type")]
    span_type: String,
}

/// Braintrust log event with parent-child support
///
/// According to Braintrust API: "Must include both 'span_id' and 'root_span_id' or neither"
/// - Root spans (turn.started, etc.): Neither span_id nor root_span_id
/// - Child spans (llm, tool, etc.): Both span_id AND root_span_id
#[derive(Debug, Serialize)]
struct BraintrustLogEvent {
    id: String,
    created: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    metadata: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    metrics: Option<BraintrustMetrics>,
    span_attributes: BraintrustSpanAttributes,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    /// Span ID for this event (required for child spans)
    #[serde(skip_serializing_if = "Option::is_none")]
    span_id: Option<String>,
    /// Root span ID for parent-child relationships (required if span_id is set)
    #[serde(skip_serializing_if = "Option::is_none")]
    root_span_id: Option<String>,
    /// Parent span IDs
    #[serde(skip_serializing_if = "Option::is_none")]
    span_parents: Option<Vec<String>>,
    /// When true, merge with existing event instead of replacing
    /// Required for started/completed event pairs to combine properly
    #[serde(rename = "_is_merge", skip_serializing_if = "Option::is_none")]
    is_merge: Option<bool>,
}

/// Request body for Braintrust insert endpoint
#[derive(Debug, Serialize)]
struct BraintrustInsertRequest {
    events: Vec<BraintrustLogEvent>,
}

/// Event listener that sends agentic loop events to Braintrust
pub struct BraintrustListener {
    config: BraintrustConfig,
    client: Client,
}

impl BraintrustListener {
    /// Create a new Braintrust listener with the given configuration
    pub fn new(config: BraintrustConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self { config, client }
    }

    /// Create a new listener from environment configuration
    /// Returns None if configuration is not available
    pub fn from_env() -> Option<Self> {
        BraintrustConfig::from_env().map(Self::new)
    }

    /// Send events to Braintrust API
    async fn send_events(&self, events: Vec<BraintrustLogEvent>) {
        let url = format!(
            "{}/v1/project_logs/{}/insert",
            self.config.api_url, self.config.project_id
        );

        let request = BraintrustInsertRequest { events };

        // Log the request payload for debugging
        if let Ok(payload) = serde_json::to_string_pretty(&request) {
            debug!(
                url = %url,
                payload = %payload,
                "Sending events to Braintrust"
            );
        }

        let result = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await;

        match result {
            Ok(response) => {
                if response.status().is_success() {
                    debug!("Successfully sent events to Braintrust");
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    error!(
                        status = %status,
                        body = %body,
                        "Failed to send events to Braintrust"
                    );
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to send events to Braintrust");
            }
        }
    }

    /// Compute span linkage for child events using OTel-style context
    ///
    /// Returns (span_id, root_span_id, span_parents) tuple:
    /// - span_id: This span's unique ID from context.span_id, or event.id as fallback
    /// - root_span_id: Root span from context.trace_id, or turn_id as fallback
    /// - span_parents: Direct parent from context.parent_span_id
    fn compute_child_span_linkage(
        event: &Event,
    ) -> (Option<String>, Option<String>, Option<Vec<String>>) {
        // Use OTel-style fields from context if available
        let span_id = event.context.span_id.clone();
        let trace_id = event.context.trace_id.clone();
        let parent_span_id = event.context.parent_span_id.clone();
        let turn_id = event.context.turn_id.as_ref();

        // Determine root_span_id (prefer trace_id, fallback to turn_id)
        let root_span_id = trace_id.or_else(|| turn_id.map(|t| t.to_string()));

        // Determine span_id (prefer context.span_id, fallback to event.id)
        let final_span_id = span_id.or_else(|| root_span_id.as_ref().map(|_| event.id.to_string()));

        // Determine span_parents (prefer context.parent_span_id, fallback to turn_id)
        let span_parents = match (parent_span_id, turn_id) {
            (Some(pid), _) => Some(vec![pid]),
            (None, Some(tid)) => Some(vec![tid.to_string()]),
            _ => None,
        };

        match root_span_id {
            Some(rsid) => (final_span_id, Some(rsid), span_parents),
            None => (None, None, None),
        }
    }

    /// Convert a turn.started event to Braintrust format (root span)
    fn convert_turn_started(
        &self,
        event: &Event,
        data: &crate::TurnStartedData,
    ) -> BraintrustLogEvent {
        let metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "turn_id": data.turn_id.to_string(),
            "input_message_id": data.input_message_id.to_string(),
        });

        // Root span: span_id and root_span_id both reference self (turn_id)
        // This allows child spans to link to the root via root_span_id
        let turn_id_str = data.turn_id.to_string();

        // Use input_content if available, otherwise fall back to message ID
        let input = if let Some(content) = &data.input_content {
            Some(serde_json::json!(content))
        } else {
            Some(serde_json::json!({
                "input_message_id": data.input_message_id.to_string(),
            }))
        };

        // Set start time in metrics for proper timeline ordering
        let start_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
        let metrics = Some(BraintrustMetrics {
            start: Some(start_time),
            end: None, // Will be set by completed event
            prompt_tokens: None,
            completion_tokens: None,
            tokens: None,
            time_to_first_token: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        });

        BraintrustLogEvent {
            id: turn_id_str.clone(), // Use turn_id as span ID for parent linking
            created: event.ts,
            input,
            output: None,
            error: None,
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: "agent turn".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id: Some(turn_id_str.clone()), // Root span self-references
            root_span_id: Some(turn_id_str.clone()), // Root span self-references
            span_parents: None,                 // Root has no parents
            is_merge: None,                     // First event creates the span
        }
    }

    /// Convert a turn.completed event to Braintrust format (updates root span)
    fn convert_turn_completed(
        &self,
        event: &Event,
        data: &crate::TurnCompletedData,
    ) -> BraintrustLogEvent {
        let mut metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "turn_id": data.turn_id.to_string(),
            "iterations": data.iterations,
        });

        // Build metrics if we have usage/duration (with prompt caching support)
        let metrics = if data.usage.is_some() || data.duration_ms.is_some() {
            let end_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
            let start_time = data.duration_ms.map(|d| end_time - (d as f64 / 1000.0));

            Some(BraintrustMetrics {
                start: start_time,
                end: Some(end_time),
                prompt_tokens: data.usage.as_ref().map(|u| u.input_tokens),
                completion_tokens: data.usage.as_ref().map(|u| u.output_tokens),
                tokens: data.usage.as_ref().map(|u| u.total_tokens()),
                time_to_first_token: None,
                cache_read_tokens: data.usage.as_ref().and_then(|u| u.cache_read_tokens),
                cache_creation_tokens: data.usage.as_ref().and_then(|u| u.cache_creation_tokens),
            })
        } else {
            None
        };

        if let Some(duration_ms) = data.duration_ms {
            metadata["duration_ms"] = serde_json::json!(duration_ms);
        }

        // Root span: span_id and root_span_id both reference self (turn_id)
        let turn_id_str = data.turn_id.to_string();

        // Use input_content if available (to preserve it during span merge)
        let input = data
            .input_content
            .as_ref()
            .map(|content| serde_json::json!(content));

        BraintrustLogEvent {
            id: turn_id_str.clone(), // Same ID as started to update the span
            created: event.ts,
            input,
            output: Some(serde_json::json!({
                "iterations": data.iterations,
                "status": "completed",
            })),
            error: None,
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: "agent turn".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id: Some(turn_id_str.clone()), // Root span self-references
            root_span_id: Some(turn_id_str.clone()), // Root span self-references
            span_parents: None,
            is_merge: Some(true), // Merge with turn.started event
        }
    }

    /// Convert an LLM generation event to Braintrust format (child span)
    fn convert_llm_generation(
        &self,
        event: &Event,
        data: &crate::LlmGenerationData,
    ) -> BraintrustLogEvent {
        // Convert messages to Braintrust-compatible (OpenAI) format
        // Our internal format uses "agent" and "tool_result" roles, but Braintrust
        // expects "assistant" and "tool" roles
        let input: serde_json::Value = data.messages.iter().map(|m| m.to_openai_format()).collect();

        // Convert output tool_calls to OpenAI format
        let tool_calls: Vec<serde_json::Value> = data
            .output
            .tool_calls
            .iter()
            .map(|tc| tc.to_openai_format())
            .collect();

        let output = if tool_calls.is_empty() {
            serde_json::json!({
                "text": data.output.text,
            })
        } else {
            serde_json::json!({
                "text": data.output.text,
                "tool_calls": tool_calls,
            })
        };

        // Build metadata
        let mut metadata = serde_json::json!({
            "model": data.metadata.model,
            "session_id": event.session_id.to_string(),
        });

        if let Some(provider) = &data.metadata.provider {
            metadata["provider"] = serde_json::json!(provider);
        }
        if let Some(response_id) = &data.metadata.response_id {
            metadata["response_id"] = serde_json::json!(response_id);
        }
        if let Some(finish_reasons) = &data.metadata.finish_reasons {
            metadata["finish_reasons"] = serde_json::json!(finish_reasons);
        }
        if let Some(turn_id) = &event.context.turn_id {
            metadata["turn_id"] = serde_json::json!(turn_id.to_string());
        }
        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }

        // Build metrics with prompt caching support
        let metrics = data.metadata.usage.as_ref().map(|usage| {
            let end_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
            let start_time = data
                .metadata
                .duration_ms
                .map(|d| end_time - (d as f64 / 1000.0));

            BraintrustMetrics {
                start: start_time,
                end: Some(end_time),
                prompt_tokens: Some(usage.input_tokens),
                completion_tokens: Some(usage.output_tokens),
                tokens: Some(usage.total_tokens()),
                time_to_first_token: data
                    .metadata
                    .time_to_first_token_ms
                    .map(|t| t as f64 / 1000.0),
                cache_read_tokens: usage.cache_read_tokens,
                cache_creation_tokens: usage.cache_creation_tokens,
            }
        });

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        BraintrustLogEvent {
            id: event.id.to_string(),
            created: event.ts,
            input: Some(input),
            output: Some(output),
            error: data.metadata.error.clone(),
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: format!("chat {}", data.metadata.model),
                span_type: "llm".to_string(),
            },
            tags: event.tags.clone(),
            span_id,
            root_span_id,
            span_parents,
            is_merge: None, // Single event, no merge needed
        }
    }

    /// Convert a tool.completed event to Braintrust format (child span)
    /// Uses span_id as the log ID so started/completed events merge into one span
    fn convert_tool_call_completed(
        &self,
        event: &Event,
        data: &crate::ToolCompletedData,
    ) -> BraintrustLogEvent {
        let input = serde_json::json!({
            "tool_call_id": data.tool_call_id,
            "tool_name": data.tool_name,
        });

        let output = if data.success {
            serde_json::json!({
                "status": "success",
                "result": data.result,
            })
        } else {
            serde_json::json!({
                "status": data.status,
                "error": data.error,
            })
        };

        let mut metadata = serde_json::json!({
            "tool_name": data.tool_name,
            "tool_call_id": data.tool_call_id,
            "session_id": event.session_id.to_string(),
            "success": data.success,
            "status": data.status,
        });

        if let Some(turn_id) = &event.context.turn_id {
            metadata["turn_id"] = serde_json::json!(turn_id.to_string());
        }

        // Build metrics if we have duration for timeline display
        let metrics = data.duration_ms.map(|duration_ms| {
            let end_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
            let start_time = end_time - (duration_ms as f64 / 1000.0);

            BraintrustMetrics {
                start: Some(start_time),
                end: Some(end_time),
                prompt_tokens: None,
                completion_tokens: None,
                tokens: None,
                time_to_first_token: None,
                cache_read_tokens: None,
                cache_creation_tokens: None,
            }
        });

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        // Use span_id as log ID so started/completed merge into one span
        let log_id = span_id.clone().unwrap_or_else(|| event.id.to_string());

        BraintrustLogEvent {
            id: log_id,
            created: event.ts,
            input: Some(input),
            output: Some(output),
            error: data.error.clone(),
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: format!("tool {}", data.tool_name),
                span_type: "tool".to_string(),
            },
            tags: event.tags.clone(),
            span_id,
            root_span_id,
            span_parents,
            is_merge: Some(true), // Merge with tool_call.started event
        }
    }

    /// Convert a turn.failed event to Braintrust format (updates root span with error)
    fn convert_turn_failed(&self, event: &Event, data: &TurnFailedData) -> BraintrustLogEvent {
        let mut metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "turn_id": data.turn_id.to_string(),
        });

        if let Some(error_code) = &data.error_code {
            metadata["error_code"] = serde_json::json!(error_code);
        }

        // Root span: span_id and root_span_id both reference self (turn_id)
        let turn_id_str = data.turn_id.to_string();

        BraintrustLogEvent {
            id: turn_id_str.clone(),
            created: event.ts,
            input: None,
            output: Some(serde_json::json!({
                "status": "failed",
            })),
            error: Some(data.error.clone()),
            metadata,
            metrics: None,
            span_attributes: BraintrustSpanAttributes {
                name: "agent turn".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id: Some(turn_id_str.clone()), // Root span self-references
            root_span_id: Some(turn_id_str.clone()), // Root span self-references
            span_parents: None,
            is_merge: Some(true), // Merge with turn.started event
        }
    }

    /// Convert a turn.cancelled event to Braintrust format (updates root span)
    fn convert_turn_cancelled(
        &self,
        event: &Event,
        data: &TurnCancelledData,
    ) -> BraintrustLogEvent {
        let mut metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "turn_id": data.turn_id.to_string(),
        });

        // Build metrics if we have usage
        let metrics = data.usage.as_ref().map(|usage| BraintrustMetrics {
            start: None,
            end: Some(event.ts.timestamp_micros() as f64 / 1_000_000.0),
            prompt_tokens: Some(usage.input_tokens),
            completion_tokens: Some(usage.output_tokens),
            tokens: Some(usage.total_tokens()),
            time_to_first_token: None,
            cache_read_tokens: usage.cache_read_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
        });

        if let Some(reason) = &data.reason {
            metadata["cancellation_reason"] = serde_json::json!(reason);
        }

        // Root span: span_id and root_span_id both reference self (turn_id)
        let turn_id_str = data.turn_id.to_string();

        BraintrustLogEvent {
            id: turn_id_str.clone(),
            created: event.ts,
            input: None,
            output: Some(serde_json::json!({
                "status": "cancelled",
            })),
            error: None,
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: "agent turn".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id: Some(turn_id_str.clone()), // Root span self-references
            root_span_id: Some(turn_id_str.clone()), // Root span self-references
            span_parents: None,
            is_merge: Some(true), // Merge with turn.started event
        }
    }

    /// Convert a reason.started event to Braintrust format (child task span)
    /// Uses span_id as the log ID so started/completed events merge into one span
    fn convert_reason_started(
        &self,
        event: &Event,
        data: &ReasonStartedData,
    ) -> BraintrustLogEvent {
        let mut metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "agent_id": data.agent_id.to_string(),
        });

        if let Some(model_meta) = &data.metadata {
            metadata["model"] = serde_json::json!(model_meta.model);
            if let Some(model_id) = &model_meta.model_id {
                metadata["model_id"] = serde_json::json!(model_id.to_string());
            }
            if let Some(provider_id) = &model_meta.provider_id {
                metadata["provider_id"] = serde_json::json!(provider_id.to_string());
            }
        }

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        if let Some(turn_id) = &event.context.turn_id {
            metadata["turn_id"] = serde_json::json!(turn_id.to_string());
        }
        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }

        // Use span_id as log ID so started/completed merge into one span
        let log_id = span_id.clone().unwrap_or_else(|| event.id.to_string());

        // Set start time in metrics for proper timeline ordering
        let start_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
        let metrics = Some(BraintrustMetrics {
            start: Some(start_time),
            end: None, // Will be set by completed event
            prompt_tokens: None,
            completion_tokens: None,
            tokens: None,
            time_to_first_token: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        });

        BraintrustLogEvent {
            id: log_id,
            created: event.ts,
            input: None,
            output: None,
            error: None,
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: "reason".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id,
            root_span_id,
            span_parents,
            is_merge: None, // First event creates the span
        }
    }

    /// Convert a reason.completed event to Braintrust format (child task span)
    /// Uses span_id as the log ID so started/completed events merge into one span
    fn convert_reason_completed(
        &self,
        event: &Event,
        data: &ReasonCompletedData,
    ) -> BraintrustLogEvent {
        let mut metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "success": data.success,
            "has_tool_calls": data.has_tool_calls,
            "tool_call_count": data.tool_call_count,
        });

        let output = serde_json::json!({
            "success": data.success,
            "has_tool_calls": data.has_tool_calls,
            "tool_call_count": data.tool_call_count,
            "text_preview": data.text_preview,
        });

        // Build metrics if we have duration/usage for timeline display
        let metrics = if data.duration_ms.is_some() || data.usage.is_some() {
            let end_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
            let start_time = data.duration_ms.map(|d| end_time - (d as f64 / 1000.0));

            Some(BraintrustMetrics {
                start: start_time,
                end: Some(end_time),
                prompt_tokens: data.usage.as_ref().map(|u| u.input_tokens),
                completion_tokens: data.usage.as_ref().map(|u| u.output_tokens),
                tokens: data.usage.as_ref().map(|u| u.total_tokens()),
                time_to_first_token: None,
                cache_read_tokens: data.usage.as_ref().and_then(|u| u.cache_read_tokens),
                cache_creation_tokens: data.usage.as_ref().and_then(|u| u.cache_creation_tokens),
            })
        } else {
            None
        };

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        if let Some(turn_id) = &event.context.turn_id {
            metadata["turn_id"] = serde_json::json!(turn_id.to_string());
        }
        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }

        // Use span_id as log ID so started/completed merge into one span
        let log_id = span_id.clone().unwrap_or_else(|| event.id.to_string());

        BraintrustLogEvent {
            id: log_id,
            created: event.ts,
            input: None,
            output: Some(output),
            error: data.error.clone(),
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: "reason".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id,
            root_span_id,
            span_parents,
            is_merge: Some(true), // Merge with reason.started event
        }
    }

    /// Convert a reason.thinking.started event to Braintrust format (child task span)
    /// Used for extended thinking models like Claude with thinking enabled.
    /// Uses span_id as the log ID so started/completed events merge into one span.
    fn convert_reason_thinking_started(
        &self,
        event: &Event,
        data: &ReasonThinkingStartedData,
    ) -> BraintrustLogEvent {
        let mut metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "turn_id": data.turn_id.to_string(),
        });

        if let Some(model) = &data.model {
            metadata["model"] = serde_json::json!(model);
        }

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }

        // Use span_id as log ID so started/completed merge into one span
        let log_id = span_id.clone().unwrap_or_else(|| event.id.to_string());

        // Set start time in metrics for proper timeline ordering
        let start_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
        let metrics = Some(BraintrustMetrics {
            start: Some(start_time),
            end: None, // Will be set by completed event
            prompt_tokens: None,
            completion_tokens: None,
            tokens: None,
            time_to_first_token: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        });

        BraintrustLogEvent {
            id: log_id,
            created: event.ts,
            input: None,
            output: None,
            error: None,
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: "thinking".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id,
            root_span_id,
            span_parents,
            is_merge: None, // First event creates the span
        }
    }

    /// Convert a reason.thinking.completed event to Braintrust format (child task span)
    /// Contains the complete thinking content from extended thinking models.
    /// Uses span_id as the log ID so started/completed events merge into one span.
    fn convert_reason_thinking_completed(
        &self,
        event: &Event,
        data: &ReasonThinkingCompletedData,
    ) -> BraintrustLogEvent {
        let metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "turn_id": data.turn_id.to_string(),
            "thinking_length": data.thinking.len(),
        });

        // Output contains the full thinking content
        let output = serde_json::json!({
            "thinking": data.thinking,
        });

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        // Use span_id as log ID so started/completed merge into one span
        let log_id = span_id.clone().unwrap_or_else(|| event.id.to_string());

        // Set end time in metrics
        let end_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
        let metrics = Some(BraintrustMetrics {
            start: None, // Was set by started event
            end: Some(end_time),
            prompt_tokens: None,
            completion_tokens: None,
            tokens: None,
            time_to_first_token: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        });

        BraintrustLogEvent {
            id: log_id,
            created: event.ts,
            input: None,
            output: Some(output),
            error: None,
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: "thinking".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id,
            root_span_id,
            span_parents,
            is_merge: Some(true), // Merge with reason.thinking.started event
        }
    }

    /// Convert an act.started event to Braintrust format (child task span)
    /// Uses span_id as the log ID so started/completed events merge into one span
    fn convert_act_started(&self, event: &Event, data: &ActStartedData) -> BraintrustLogEvent {
        let mut metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "tool_count": data.tool_calls.len(),
        });

        let input = serde_json::json!({
            "tool_calls": data.tool_calls.iter().map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "name": tc.name,
                })
            }).collect::<Vec<_>>(),
        });

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        if let Some(turn_id) = &event.context.turn_id {
            metadata["turn_id"] = serde_json::json!(turn_id.to_string());
        }
        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }

        // Use span_id as log ID so started/completed merge into one span
        let log_id = span_id.clone().unwrap_or_else(|| event.id.to_string());

        // Set start time in metrics for proper timeline ordering
        let start_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
        let metrics = Some(BraintrustMetrics {
            start: Some(start_time),
            end: None, // Will be set by completed event
            prompt_tokens: None,
            completion_tokens: None,
            tokens: None,
            time_to_first_token: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        });

        BraintrustLogEvent {
            id: log_id,
            created: event.ts,
            input: Some(input),
            output: None,
            error: None,
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: "act".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id,
            root_span_id,
            span_parents,
            is_merge: None, // First event creates the span
        }
    }

    /// Convert an act.completed event to Braintrust format (child task span)
    /// Uses span_id as the log ID so started/completed events merge into one span
    fn convert_act_completed(&self, event: &Event, data: &ActCompletedData) -> BraintrustLogEvent {
        let mut metadata = serde_json::json!({
            "session_id": event.session_id.to_string(),
            "completed": data.completed,
            "success_count": data.success_count,
            "error_count": data.error_count,
        });

        let output = serde_json::json!({
            "completed": data.completed,
            "success_count": data.success_count,
            "error_count": data.error_count,
        });

        // Build metrics if we have duration for timeline display
        let metrics = data.duration_ms.map(|duration_ms| {
            let end_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
            let start_time = end_time - (duration_ms as f64 / 1000.0);

            BraintrustMetrics {
                start: Some(start_time),
                end: Some(end_time),
                prompt_tokens: None,
                completion_tokens: None,
                tokens: None,
                time_to_first_token: None,
                cache_read_tokens: None,
                cache_creation_tokens: None,
            }
        });

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        if let Some(turn_id) = &event.context.turn_id {
            metadata["turn_id"] = serde_json::json!(turn_id.to_string());
        }
        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }

        // Use span_id as log ID so started/completed merge into one span
        let log_id = span_id.clone().unwrap_or_else(|| event.id.to_string());

        BraintrustLogEvent {
            id: log_id,
            created: event.ts,
            input: None,
            output: Some(output),
            error: None,
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: "act".to_string(),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id,
            root_span_id,
            span_parents,
            is_merge: Some(true), // Merge with act.started event
        }
    }

    /// Convert a tool.started event to Braintrust format (child tool span)
    /// Uses span_id as the log ID so started/completed events merge into one span
    fn convert_tool_call_started(
        &self,
        event: &Event,
        data: &ToolStartedData,
    ) -> BraintrustLogEvent {
        let input = serde_json::json!({
            "tool_call_id": data.tool_call.id,
            "tool_name": data.tool_call.name,
            "arguments": data.tool_call.arguments,
        });

        let mut metadata = serde_json::json!({
            "tool_name": data.tool_call.name,
            "tool_call_id": data.tool_call.id,
            "session_id": event.session_id.to_string(),
        });

        if let Some(turn_id) = &event.context.turn_id {
            metadata["turn_id"] = serde_json::json!(turn_id.to_string());
        }

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        // Use span_id as log ID so started/completed merge into one span
        let log_id = span_id.clone().unwrap_or_else(|| event.id.to_string());

        // Set start time in metrics for proper timeline ordering
        let start_time = event.ts.timestamp_micros() as f64 / 1_000_000.0;
        let metrics = Some(BraintrustMetrics {
            start: Some(start_time),
            end: None, // Will be set by completed event
            prompt_tokens: None,
            completion_tokens: None,
            tokens: None,
            time_to_first_token: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        });

        BraintrustLogEvent {
            id: log_id,
            created: event.ts,
            input: Some(input),
            output: None,
            error: None,
            metadata,
            metrics,
            span_attributes: BraintrustSpanAttributes {
                name: format!("tool {}", data.tool_call.name),
                span_type: "tool".to_string(),
            },
            tags: event.tags.clone(),
            span_id,
            root_span_id,
            span_parents,
            is_merge: None, // First event creates the span
        }
    }
}

#[async_trait]
impl EventListener for BraintrustListener {
    async fn on_event(&self, event: &Event) {
        let bt_event = match &event.data {
            // Turn lifecycle events (root task spans)
            EventData::TurnStarted(data) => {
                debug!(turn_id = %data.turn_id, "Processing turn.started for Braintrust");
                self.convert_turn_started(event, data)
            }
            EventData::TurnCompleted(data) => {
                debug!(turn_id = %data.turn_id, "Processing turn.completed for Braintrust");
                self.convert_turn_completed(event, data)
            }
            EventData::TurnFailed(data) => {
                debug!(turn_id = %data.turn_id, "Processing turn.failed for Braintrust");
                self.convert_turn_failed(event, data)
            }
            EventData::TurnCancelled(data) => {
                debug!(turn_id = %data.turn_id, "Processing turn.cancelled for Braintrust");
                self.convert_turn_cancelled(event, data)
            }

            // Atom lifecycle events (reason/act phases)
            EventData::ReasonStarted(data) => {
                debug!(agent_id = %data.agent_id, "Processing reason.started for Braintrust");
                self.convert_reason_started(event, data)
            }
            EventData::ReasonCompleted(data) => {
                debug!(
                    success = data.success,
                    "Processing reason.completed for Braintrust"
                );
                self.convert_reason_completed(event, data)
            }

            // Extended thinking events (e.g., Claude extended thinking mode)
            EventData::ReasonThinkingStarted(data) => {
                debug!(turn_id = %data.turn_id, "Processing reason.thinking.started for Braintrust");
                self.convert_reason_thinking_started(event, data)
            }
            EventData::ReasonThinkingCompleted(data) => {
                debug!(turn_id = %data.turn_id, "Processing reason.thinking.completed for Braintrust");
                self.convert_reason_thinking_completed(event, data)
            }

            EventData::ActStarted(data) => {
                debug!(
                    tool_count = data.tool_calls.len(),
                    "Processing act.started for Braintrust"
                );
                self.convert_act_started(event, data)
            }
            EventData::ActCompleted(data) => {
                debug!(
                    success_count = data.success_count,
                    error_count = data.error_count,
                    "Processing act.completed for Braintrust"
                );
                self.convert_act_completed(event, data)
            }

            // LLM generation events
            EventData::LlmGeneration(data) => {
                debug!(
                    event_id = %event.id,
                    model = %data.metadata.model,
                    "Processing llm.generation for Braintrust"
                );
                self.convert_llm_generation(event, data)
            }

            // Tool events
            EventData::ToolStarted(data) => {
                debug!(
                    tool_name = %data.tool_call.name,
                    tool_call_id = %data.tool_call.id,
                    "Processing tool.started for Braintrust"
                );
                self.convert_tool_call_started(event, data)
            }
            EventData::ToolCompleted(data) => {
                debug!(
                    tool_name = %data.tool_name,
                    tool_call_id = %data.tool_call_id,
                    "Processing tool.completed for Braintrust"
                );
                self.convert_tool_call_completed(event, data)
            }

            _ => return, // Ignore other event types
        };

        // Clone self for async task
        let client = self.client.clone();
        let config = self.config.clone();

        // Spawn async task to send event (don't block event processing)
        tokio::spawn(async move {
            let listener = BraintrustListener { config, client };
            listener.send_events(vec![bt_event]).await;
        });
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            // Turn lifecycle
            TURN_STARTED,
            TURN_COMPLETED,
            TURN_FAILED,
            TURN_CANCELLED,
            // Atom lifecycle (reason/act phases)
            REASON_STARTED,
            REASON_COMPLETED,
            ACT_STARTED,
            ACT_COMPLETED,
            // Extended thinking (e.g., Claude extended thinking mode)
            REASON_THINKING_STARTED,
            REASON_THINKING_COMPLETED,
            // LLM generation
            LLM_GENERATION,
            // Tool execution
            TOOL_STARTED,
            TOOL_COMPLETED,
        ])
    }

    fn name(&self) -> &'static str {
        "BraintrustListener"
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{
        EventContext, LlmGenerationData, LlmGenerationMetadata, LlmGenerationOutput,
        ReasonCompletedData, TokenUsage, ToolCompletedData, TurnCompletedData, TurnStartedData,
    };
    use crate::message::Message;
    use crate::typed_id::{AgentId, MessageId, SessionId, TurnId};
    use uuid::Uuid;

    fn test_config() -> BraintrustConfig {
        BraintrustConfig {
            api_key: "test-api-key".to_string(),
            project_id: "test-project-id".to_string(),
            api_url: "https://api.braintrust.dev".to_string(),
        }
    }

    #[test]
    fn test_listener_creation() {
        let listener = BraintrustListener::new(test_config());
        assert_eq!(listener.name(), "BraintrustListener");
    }

    #[test]
    fn test_event_types() {
        let listener = BraintrustListener::new(test_config());
        let types = listener.event_types().unwrap();
        // 13 event types: 4 turn lifecycle + 4 atom lifecycle + 2 thinking + 1 llm + 2 tool
        assert_eq!(types.len(), 13);
        // Turn lifecycle
        assert!(types.contains(&TURN_STARTED));
        assert!(types.contains(&TURN_COMPLETED));
        assert!(types.contains(&TURN_FAILED));
        assert!(types.contains(&TURN_CANCELLED));
        // Atom lifecycle
        assert!(types.contains(&REASON_STARTED));
        assert!(types.contains(&REASON_COMPLETED));
        assert!(types.contains(&ACT_STARTED));
        assert!(types.contains(&ACT_COMPLETED));
        // Extended thinking
        assert!(types.contains(&REASON_THINKING_STARTED));
        assert!(types.contains(&REASON_THINKING_COMPLETED));
        // LLM
        assert!(types.contains(&LLM_GENERATION));
        // Tool
        assert!(types.contains(&TOOL_STARTED));
        assert!(types.contains(&TOOL_COMPLETED));
    }

    #[test]
    fn test_convert_turn_started() {
        let listener = BraintrustListener::new(test_config());

        let turn_id = TurnId::new();
        let message_id = MessageId::new();

        let data = TurnStartedData {
            turn_id,
            input_message_id: message_id,
            input_content: Some("Hello, how are you?".to_string()),
        };

        let event = Event::new(
            SessionId::new(),
            EventContext::empty(),
            EventData::TurnStarted(data.clone()),
        );

        let bt_event = listener.convert_turn_started(&event, &data);

        assert_eq!(bt_event.id, turn_id.to_string());
        assert_eq!(bt_event.span_attributes.span_type, "task");
        assert_eq!(bt_event.span_attributes.name, "agent turn");
        // Root span self-references for proper trace correlation
        assert_eq!(bt_event.span_id, Some(turn_id.to_string()));
        assert_eq!(bt_event.root_span_id, Some(turn_id.to_string()));
        assert!(bt_event.span_parents.is_none()); // Root has no parents
    }

    #[test]
    fn test_convert_llm_generation_with_parent() {
        let listener = BraintrustListener::new(test_config());

        let turn_id = TurnId::new();

        let data = LlmGenerationData {
            messages: vec![Message::user("Hello"), Message::assistant("Hi there!")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Hi there!".to_string()),
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4".to_string(),
                provider: Some("openai".to_string()),
                usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                }),
                duration_ms: Some(100),
                time_to_first_token_ms: Some(25),
                success: true,
                error: None,
                finish_reasons: Some(vec!["stop".to_string()]),
                response_id: Some("resp_123".to_string()),
                retry: None,
            },
        };

        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);

        let event = Event::new(
            SessionId::new(),
            context,
            EventData::LlmGeneration(data.clone()),
        );

        let bt_event = listener.convert_llm_generation(&event, &data);

        assert_eq!(bt_event.span_attributes.name, "chat gpt-4");
        assert_eq!(bt_event.span_attributes.span_type, "llm");
        assert_eq!(bt_event.root_span_id, Some(turn_id.to_string()));
        assert_eq!(bt_event.span_parents, Some(vec![turn_id.to_string()]));
    }

    #[test]
    fn test_convert_turn_completed_with_usage() {
        let listener = BraintrustListener::new(test_config());

        let turn_id = TurnId::new();

        let data = TurnCompletedData {
            turn_id,
            iterations: 3,
            duration_ms: Some(5000),
            usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: None,
                cache_creation_tokens: None,
            }),
            input_content: Some("Hello, how are you?".to_string()),
        };

        let event = Event::new(
            SessionId::new(),
            EventContext::empty(),
            EventData::TurnCompleted(data.clone()),
        );

        let bt_event = listener.convert_turn_completed(&event, &data);

        assert_eq!(bt_event.id, turn_id.to_string());
        assert!(bt_event.metrics.is_some());
        let metrics = bt_event.metrics.unwrap();
        assert_eq!(metrics.prompt_tokens, Some(100));
        assert_eq!(metrics.completion_tokens, Some(50));
        assert_eq!(metrics.tokens, Some(150));
    }

    // =============================================================================
    // Event Hierarchy Tests
    // =============================================================================
    //
    // These tests verify the span ID relationships required for proper trace hierarchy:
    //
    // agent turn (root)
    // ├── reason (parent: turn)
    // │   └── llm.generation (parent: reason)
    // ├── act (parent: turn)
    // │   └── tool.call (parent: act)
    // └── ...

    #[test]
    fn test_turn_events_are_self_referencing_root_spans() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();

        // Test turn.started
        let started_data = TurnStartedData {
            turn_id,
            input_message_id,
            input_content: Some("Test input content".to_string()),
        };
        let started_event = Event::new(
            SessionId::new(),
            EventContext::empty(),
            EventData::TurnStarted(started_data.clone()),
        );
        let bt_started = listener.convert_turn_started(&started_event, &started_data);

        // Root span: span_id = root_span_id = turn_id, no parents
        assert_eq!(
            bt_started.id,
            turn_id.to_string(),
            "turn.started id should be turn_id"
        );
        assert_eq!(
            bt_started.span_id,
            Some(turn_id.to_string()),
            "turn.started span_id should be turn_id"
        );
        assert_eq!(
            bt_started.root_span_id,
            Some(turn_id.to_string()),
            "turn.started root_span_id should be turn_id"
        );
        assert!(
            bt_started.span_parents.is_none(),
            "turn.started should have no parents (root span)"
        );

        // Test turn.completed uses same IDs (for merging)
        let completed_data = TurnCompletedData {
            turn_id,
            iterations: 1,
            duration_ms: Some(1000),
            usage: None,
            input_content: Some("Test input content".to_string()),
        };
        let completed_event = Event::new(
            SessionId::new(),
            EventContext::empty(),
            EventData::TurnCompleted(completed_data.clone()),
        );
        let bt_completed = listener.convert_turn_completed(&completed_event, &completed_data);

        // Same IDs as started for proper merging
        assert_eq!(
            bt_completed.id,
            turn_id.to_string(),
            "turn.completed id should match turn.started"
        );
        assert_eq!(
            bt_completed.span_id,
            Some(turn_id.to_string()),
            "turn.completed span_id should match turn.started"
        );
        assert_eq!(bt_completed.root_span_id, Some(turn_id.to_string()));
    }

    #[test]
    fn test_reason_events_have_turn_as_parent() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let reason_span_id = Uuid::now_v7().to_string();

        // Create event context with proper span linkage
        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(reason_span_id.clone());
        context.parent_span_id = Some(turn_id.to_string());

        let data = ReasonStartedData {
            agent_id: AgentId::new(),
            metadata: None,
        };
        let event = Event::new(
            SessionId::new(),
            context,
            EventData::ReasonStarted(data.clone()),
        );

        let bt_event = listener.convert_reason_started(&event, &data);

        // Reason should be child of turn
        assert_eq!(
            bt_event.span_id,
            Some(reason_span_id.clone()),
            "reason span_id should be the reason's span"
        );
        assert_eq!(
            bt_event.root_span_id,
            Some(turn_id.to_string()),
            "reason root_span_id should be turn_id"
        );
        assert_eq!(
            bt_event.span_parents,
            Some(vec![turn_id.to_string()]),
            "reason parent should be turn"
        );
    }

    #[test]
    fn test_llm_generation_with_span_context_has_reason_as_parent() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let reason_span_id = Uuid::now_v7().to_string();
        let llm_span_id = Uuid::now_v7().to_string();

        // LLM generation event with span context (parent is reason)
        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(llm_span_id.clone());
        context.parent_span_id = Some(reason_span_id.clone());

        let data = LlmGenerationData {
            messages: vec![Message::user("Hello")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Hi!".to_string()),
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4".to_string(),
                provider: None,
                usage: None,
                duration_ms: None,
                time_to_first_token_ms: None,
                success: true,
                error: None,
                finish_reasons: None,
                response_id: None,
                retry: None,
            },
        };
        let event = Event::new(
            SessionId::new(),
            context,
            EventData::LlmGeneration(data.clone()),
        );

        let bt_event = listener.convert_llm_generation(&event, &data);

        // LLM should be child of reason
        assert_eq!(
            bt_event.span_id,
            Some(llm_span_id),
            "llm span_id should be its own span"
        );
        assert_eq!(
            bt_event.root_span_id,
            Some(turn_id.to_string()),
            "llm root_span_id should be turn_id"
        );
        assert_eq!(
            bt_event.span_parents,
            Some(vec![reason_span_id]),
            "llm parent should be reason span"
        );
    }

    #[test]
    fn test_act_events_have_turn_as_parent() {
        use crate::events::ToolCallSummary;

        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let act_span_id = Uuid::now_v7().to_string();

        // Create event context with proper span linkage
        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(act_span_id.clone());
        context.parent_span_id = Some(turn_id.to_string());

        let data = ActStartedData {
            tool_calls: vec![
                ToolCallSummary {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                },
                ToolCallSummary {
                    id: "call_2".to_string(),
                    name: "fetch".to_string(),
                },
            ],
        };
        let event = Event::new(
            SessionId::new(),
            context,
            EventData::ActStarted(data.clone()),
        );

        let bt_event = listener.convert_act_started(&event, &data);

        // Act should be child of turn
        assert_eq!(
            bt_event.span_id,
            Some(act_span_id.clone()),
            "act span_id should be the act's span"
        );
        assert_eq!(
            bt_event.root_span_id,
            Some(turn_id.to_string()),
            "act root_span_id should be turn_id"
        );
        assert_eq!(
            bt_event.span_parents,
            Some(vec![turn_id.to_string()]),
            "act parent should be turn"
        );
    }

    #[test]
    fn test_tool_call_events_have_act_as_parent() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let act_span_id = Uuid::now_v7().to_string();
        let tool_span_id = Uuid::now_v7().to_string();

        // Tool call event with span context (parent is act)
        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(tool_span_id.clone());
        context.parent_span_id = Some(act_span_id.clone());

        let data = ToolCompletedData {
            tool_call_id: "call_123".to_string(),
            tool_name: "search".to_string(),
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
            duration_ms: Some(200),
        };
        let event = Event::new(
            SessionId::new(),
            context,
            EventData::ToolCompleted(data.clone()),
        );

        let bt_event = listener.convert_tool_call_completed(&event, &data);

        // Tool should be child of act
        assert_eq!(
            bt_event.span_id,
            Some(tool_span_id),
            "tool span_id should be its own span"
        );
        assert_eq!(
            bt_event.root_span_id,
            Some(turn_id.to_string()),
            "tool root_span_id should be turn_id"
        );
        assert_eq!(
            bt_event.span_parents,
            Some(vec![act_span_id]),
            "tool parent should be act span"
        );
    }

    #[test]
    fn test_started_completed_pairs_share_span_id() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let shared_span_id = Uuid::now_v7().to_string();

        // Both started and completed should use the same span_id
        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(shared_span_id.clone());
        context.parent_span_id = Some(turn_id.to_string());

        // reason.started
        let started_data = ReasonStartedData {
            agent_id: AgentId::new(),
            metadata: None,
        };
        let started_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ReasonStarted(started_data.clone()),
        );
        let bt_started = listener.convert_reason_started(&started_event, &started_data);

        // reason.completed with same span context
        let completed_data = ReasonCompletedData {
            success: true,
            text_preview: Some("Hello".to_string()),
            has_tool_calls: false,
            tool_call_count: 0,
            error: None,
            duration_ms: Some(1500),
            usage: None,
        };
        let completed_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ReasonCompleted(completed_data.clone()),
        );
        let bt_completed = listener.convert_reason_completed(&completed_event, &completed_data);

        // Both should have same span_id for Braintrust to merge them
        assert_eq!(
            bt_started.span_id, bt_completed.span_id,
            "started and completed should share span_id"
        );
        assert_eq!(
            bt_started.id, bt_completed.id,
            "started and completed should share log id"
        );
    }

    #[test]
    fn test_all_events_in_trace_share_root_span_id() {
        use crate::events::ToolCallSummary;

        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let reason_span_id = Uuid::now_v7().to_string();
        let act_span_id = Uuid::now_v7().to_string();
        let llm_span_id = Uuid::now_v7().to_string();
        let tool_span_id = Uuid::now_v7().to_string();
        let input_message_id = MessageId::new();

        // All events should have the same root_span_id = turn_id
        let expected_root = turn_id.to_string();

        // turn.started
        let turn_data = TurnStartedData {
            turn_id,
            input_message_id,
            input_content: Some("Test message".to_string()),
        };
        let turn_event = Event::new(
            SessionId::new(),
            EventContext::empty(),
            EventData::TurnStarted(turn_data.clone()),
        );
        let bt_turn = listener.convert_turn_started(&turn_event, &turn_data);
        assert_eq!(
            bt_turn.root_span_id,
            Some(expected_root.clone()),
            "turn should have root_span_id = turn_id"
        );

        // reason event
        let mut reason_ctx = EventContext::empty();
        reason_ctx.turn_id = Some(turn_id);
        reason_ctx.trace_id = Some(turn_id.to_string());
        reason_ctx.span_id = Some(reason_span_id.clone());
        reason_ctx.parent_span_id = Some(turn_id.to_string());
        let reason_data = ReasonStartedData {
            agent_id: AgentId::new(),
            metadata: None,
        };
        let reason_event = Event::new(
            SessionId::new(),
            reason_ctx,
            EventData::ReasonStarted(reason_data.clone()),
        );
        let bt_reason = listener.convert_reason_started(&reason_event, &reason_data);
        assert_eq!(
            bt_reason.root_span_id,
            Some(expected_root.clone()),
            "reason should have root_span_id = turn_id"
        );

        // llm event
        let mut llm_ctx = EventContext::empty();
        llm_ctx.turn_id = Some(turn_id);
        llm_ctx.trace_id = Some(turn_id.to_string());
        llm_ctx.span_id = Some(llm_span_id);
        llm_ctx.parent_span_id = Some(reason_span_id.clone());
        let llm_data = LlmGenerationData {
            messages: vec![],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("hi".to_string()),
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4".to_string(),
                provider: None,
                usage: None,
                duration_ms: None,
                time_to_first_token_ms: None,
                success: true,
                error: None,
                finish_reasons: None,
                response_id: None,
                retry: None,
            },
        };
        let llm_event = Event::new(
            SessionId::new(),
            llm_ctx,
            EventData::LlmGeneration(llm_data.clone()),
        );
        let bt_llm = listener.convert_llm_generation(&llm_event, &llm_data);
        assert_eq!(
            bt_llm.root_span_id,
            Some(expected_root.clone()),
            "llm should have root_span_id = turn_id"
        );

        // act event
        let mut act_ctx = EventContext::empty();
        act_ctx.turn_id = Some(turn_id);
        act_ctx.trace_id = Some(turn_id.to_string());
        act_ctx.span_id = Some(act_span_id.clone());
        act_ctx.parent_span_id = Some(turn_id.to_string());
        let act_data = ActStartedData {
            tool_calls: vec![ToolCallSummary {
                id: "call_1".to_string(),
                name: "search".to_string(),
            }],
        };
        let act_event = Event::new(
            SessionId::new(),
            act_ctx,
            EventData::ActStarted(act_data.clone()),
        );
        let bt_act = listener.convert_act_started(&act_event, &act_data);
        assert_eq!(
            bt_act.root_span_id,
            Some(expected_root.clone()),
            "act should have root_span_id = turn_id"
        );

        // tool event
        let mut tool_ctx = EventContext::empty();
        tool_ctx.turn_id = Some(turn_id);
        tool_ctx.trace_id = Some(turn_id.to_string());
        tool_ctx.span_id = Some(tool_span_id);
        tool_ctx.parent_span_id = Some(act_span_id);
        let tool_data = ToolCompletedData {
            tool_call_id: "call_1".to_string(),
            tool_name: "search".to_string(),
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
            duration_ms: Some(75),
        };
        let tool_event = Event::new(
            SessionId::new(),
            tool_ctx,
            EventData::ToolCompleted(tool_data.clone()),
        );
        let bt_tool = listener.convert_tool_call_completed(&tool_event, &tool_data);
        assert_eq!(
            bt_tool.root_span_id,
            Some(expected_root.clone()),
            "tool should have root_span_id = turn_id"
        );
    }

    // =============================================================================
    // _is_merge Serialization Tests
    // =============================================================================
    //
    // These tests verify that the _is_merge field is correctly serialized:
    // - Started events: is_merge = None (creates new span)
    // - Completed events: is_merge = Some(true) (merges with existing span)

    #[test]
    fn test_is_merge_serialization_started_events() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();

        // turn.started should have is_merge = None
        let turn_data = TurnStartedData {
            turn_id,
            input_message_id,
            input_content: Some("Test".to_string()),
        };
        let event = Event::new(
            SessionId::new(),
            EventContext::empty(),
            EventData::TurnStarted(turn_data.clone()),
        );
        let bt_event = listener.convert_turn_started(&event, &turn_data);

        // is_merge should be None for started events
        assert!(
            bt_event.is_merge.is_none(),
            "turn.started should have is_merge = None"
        );

        // Serialize to JSON and verify _is_merge is not present
        let json = serde_json::to_string(&bt_event).unwrap();
        assert!(
            !json.contains("_is_merge"),
            "turn.started JSON should not contain _is_merge: {}",
            json
        );
    }

    #[test]
    fn test_is_merge_serialization_completed_events() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();

        // turn.completed should have is_merge = Some(true)
        let turn_data = TurnCompletedData {
            turn_id,
            iterations: 1,
            duration_ms: Some(1000),
            usage: None,
            input_content: Some("Test".to_string()),
        };
        let event = Event::new(
            SessionId::new(),
            EventContext::empty(),
            EventData::TurnCompleted(turn_data.clone()),
        );
        let bt_event = listener.convert_turn_completed(&event, &turn_data);

        // is_merge should be Some(true) for completed events
        assert_eq!(
            bt_event.is_merge,
            Some(true),
            "turn.completed should have is_merge = Some(true)"
        );

        // Serialize to JSON and verify _is_merge is present and true
        let json = serde_json::to_string(&bt_event).unwrap();
        assert!(
            json.contains("\"_is_merge\":true"),
            "turn.completed JSON should contain _is_merge:true: {}",
            json
        );
    }

    #[test]
    fn test_is_merge_serialization_reason_events() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let reason_span_id = Uuid::now_v7().to_string();

        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(reason_span_id.clone());
        context.parent_span_id = Some(turn_id.to_string());

        // reason.started - should NOT have _is_merge
        let started_data = ReasonStartedData {
            agent_id: AgentId::new(),
            metadata: None,
        };
        let started_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ReasonStarted(started_data.clone()),
        );
        let bt_started = listener.convert_reason_started(&started_event, &started_data);
        let started_json = serde_json::to_string(&bt_started).unwrap();
        assert!(
            !started_json.contains("_is_merge"),
            "reason.started should not have _is_merge: {}",
            started_json
        );

        // reason.completed - should have _is_merge: true
        let completed_data = ReasonCompletedData {
            success: true,
            text_preview: None,
            has_tool_calls: false,
            tool_call_count: 0,
            error: None,
            duration_ms: Some(500),
            usage: None,
        };
        let completed_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ReasonCompleted(completed_data.clone()),
        );
        let bt_completed = listener.convert_reason_completed(&completed_event, &completed_data);
        let completed_json = serde_json::to_string(&bt_completed).unwrap();
        assert!(
            completed_json.contains("\"_is_merge\":true"),
            "reason.completed should have _is_merge:true: {}",
            completed_json
        );
    }

    #[test]
    fn test_is_merge_serialization_act_events() {
        use crate::events::ToolCallSummary;

        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let act_span_id = Uuid::now_v7().to_string();

        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(act_span_id.clone());
        context.parent_span_id = Some(turn_id.to_string());

        // act.started - should NOT have _is_merge
        let started_data = ActStartedData {
            tool_calls: vec![ToolCallSummary {
                id: "call_1".to_string(),
                name: "search".to_string(),
            }],
        };
        let started_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ActStarted(started_data.clone()),
        );
        let bt_started = listener.convert_act_started(&started_event, &started_data);
        let started_json = serde_json::to_string(&bt_started).unwrap();
        assert!(
            !started_json.contains("_is_merge"),
            "act.started should not have _is_merge: {}",
            started_json
        );

        // act.completed - should have _is_merge: true
        let completed_data = ActCompletedData {
            completed: true,
            success_count: 1,
            error_count: 0,
            duration_ms: Some(200),
        };
        let completed_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ActCompleted(completed_data.clone()),
        );
        let bt_completed = listener.convert_act_completed(&completed_event, &completed_data);
        let completed_json = serde_json::to_string(&bt_completed).unwrap();
        assert!(
            completed_json.contains("\"_is_merge\":true"),
            "act.completed should have _is_merge:true: {}",
            completed_json
        );
    }

    #[test]
    fn test_is_merge_serialization_tool_events() {
        use crate::tool_types::ToolCall;

        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let tool_span_id = Uuid::now_v7().to_string();
        let act_span_id = Uuid::now_v7().to_string();

        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(tool_span_id.clone());
        context.parent_span_id = Some(act_span_id.clone());

        // tool.started - should NOT have _is_merge
        let started_data = ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({"query": "test"}),
            },
        };
        let started_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ToolStarted(started_data.clone()),
        );
        let bt_started = listener.convert_tool_call_started(&started_event, &started_data);
        let started_json = serde_json::to_string(&bt_started).unwrap();
        assert!(
            !started_json.contains("_is_merge"),
            "tool.started should not have _is_merge: {}",
            started_json
        );

        // tool.completed - should have _is_merge: true
        let completed_data = ToolCompletedData {
            tool_call_id: "call_1".to_string(),
            tool_name: "search".to_string(),
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
            duration_ms: Some(100),
        };
        let completed_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ToolCompleted(completed_data.clone()),
        );
        let bt_completed = listener.convert_tool_call_completed(&completed_event, &completed_data);
        let completed_json = serde_json::to_string(&bt_completed).unwrap();
        assert!(
            completed_json.contains("\"_is_merge\":true"),
            "tool.completed should have _is_merge:true: {}",
            completed_json
        );
    }

    #[test]
    fn test_convert_reason_thinking_started() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let span_id = Uuid::new_v4().to_string();

        let data = ReasonThinkingStartedData {
            turn_id,
            model: Some("claude-sonnet-4-20250514".to_string()),
        };

        // Context with parent span
        let context = EventContext {
            turn_id: Some(turn_id),
            input_message_id: None,
            span_id: Some(span_id.clone()),
            parent_span_id: Some(turn_id.to_string()),
            exec_id: None,
            trace_id: Some(turn_id.to_string()),
        };

        let event = Event::new(
            SessionId::new(),
            context,
            EventData::ReasonThinkingStarted(data.clone()),
        );

        let bt_event = listener.convert_reason_thinking_started(&event, &data);

        assert_eq!(bt_event.span_attributes.name, "thinking");
        assert_eq!(bt_event.span_attributes.span_type, "task");
        assert_eq!(bt_event.span_id, Some(span_id.clone()));
        assert_eq!(bt_event.root_span_id, Some(turn_id.to_string()));
        assert!(bt_event.is_merge.is_none()); // First event creates the span
        assert!(bt_event.metrics.as_ref().unwrap().start.is_some());
        assert!(bt_event.metrics.as_ref().unwrap().end.is_none());

        // Verify metadata
        let metadata = bt_event.metadata;
        assert_eq!(metadata["model"], "claude-sonnet-4-20250514");
        assert_eq!(metadata["turn_id"], turn_id.to_string());
    }

    #[test]
    fn test_convert_reason_thinking_completed() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let span_id = Uuid::new_v4().to_string();

        let thinking_content = "Let me think about this problem step by step...".to_string();
        let data = ReasonThinkingCompletedData {
            turn_id,
            thinking: thinking_content.clone(),
        };

        // Context with parent span
        let context = EventContext {
            turn_id: Some(turn_id),
            input_message_id: None,
            span_id: Some(span_id.clone()),
            parent_span_id: Some(turn_id.to_string()),
            exec_id: None,
            trace_id: Some(turn_id.to_string()),
        };

        let event = Event::new(
            SessionId::new(),
            context,
            EventData::ReasonThinkingCompleted(data.clone()),
        );

        let bt_event = listener.convert_reason_thinking_completed(&event, &data);

        assert_eq!(bt_event.span_attributes.name, "thinking");
        assert_eq!(bt_event.span_attributes.span_type, "task");
        assert_eq!(bt_event.span_id, Some(span_id.clone()));
        assert_eq!(bt_event.root_span_id, Some(turn_id.to_string()));
        assert_eq!(bt_event.is_merge, Some(true)); // Merges with started event

        // Verify output contains thinking content
        let output = bt_event.output.unwrap();
        assert_eq!(output["thinking"], thinking_content);

        // Verify metadata
        let metadata = bt_event.metadata;
        assert_eq!(metadata["thinking_length"], thinking_content.len());
        assert_eq!(metadata["turn_id"], turn_id.to_string());

        // Verify metrics have end time
        assert!(bt_event.metrics.as_ref().unwrap().end.is_some());
    }

    #[test]
    fn test_thinking_spans_merge_correctly() {
        let listener = BraintrustListener::new(test_config());
        let turn_id = TurnId::new();
        let span_id = Uuid::new_v4().to_string();

        let context = EventContext {
            turn_id: Some(turn_id),
            input_message_id: None,
            span_id: Some(span_id.clone()),
            parent_span_id: Some(turn_id.to_string()),
            exec_id: None,
            trace_id: Some(turn_id.to_string()),
        };

        // thinking.started
        let started_data = ReasonThinkingStartedData {
            turn_id,
            model: Some("claude-sonnet-4-20250514".to_string()),
        };
        let started_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ReasonThinkingStarted(started_data.clone()),
        );
        let bt_started = listener.convert_reason_thinking_started(&started_event, &started_data);

        // thinking.completed
        let completed_data = ReasonThinkingCompletedData {
            turn_id,
            thinking: "Complete thinking content".to_string(),
        };
        let completed_event = Event::new(
            SessionId::new(),
            context.clone(),
            EventData::ReasonThinkingCompleted(completed_data.clone()),
        );
        let bt_completed =
            listener.convert_reason_thinking_completed(&completed_event, &completed_data);

        // Both should use the same span_id for merging
        assert_eq!(bt_started.id, bt_completed.id);
        assert_eq!(bt_started.span_id, bt_completed.span_id);

        // started should not have _is_merge, completed should have _is_merge: true
        let started_json = serde_json::to_string(&bt_started).unwrap();
        assert!(
            !started_json.contains("\"_is_merge\""),
            "thinking.started should not have _is_merge: {}",
            started_json
        );

        let completed_json = serde_json::to_string(&bt_completed).unwrap();
        assert!(
            completed_json.contains("\"_is_merge\":true"),
            "thinking.completed should have _is_merge:true: {}",
            completed_json
        );
    }
}
