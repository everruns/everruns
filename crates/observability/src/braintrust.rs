// Braintrust Event Listener
//
// This listener sends agentic loop events to Braintrust for observability.
// Braintrust provides tracing, logging, and evaluation capabilities for LLM applications.
//
// See knowledge/operations/observability.md for full specification.
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
use rand::RngExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::time::{self, Duration};
use tracing::{debug, error, info, warn};

use everruns_core::DeploymentGrade;
use everruns_core::{
    ACT_COMPLETED, ACT_STARTED, ActCompletedData, ActStartedData, Event, EventData, EventListener,
    LLM_GENERATION, REASON_COMPLETED, REASON_STARTED, REASON_THINKING_COMPLETED,
    REASON_THINKING_STARTED, ReasonCompletedData, ReasonStartedData, ReasonThinkingCompletedData,
    ReasonThinkingStartedData, SESSION_ACTIVATED, SESSION_IDLED, SESSION_STARTED, TOOL_COMPLETED,
    TOOL_STARTED, TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_STARTED, ToolStartedData,
    TurnCancelledData, TurnFailedData,
};

/// Configuration for Braintrust integration
#[derive(Debug, Clone)]
pub struct BraintrustConfig {
    /// API key for authentication
    pub api_key: String,
    /// Project ID (resolved from name if needed)
    pub project_id: String,
    /// API base URL (default: <https://api.braintrust.dev>)
    pub api_url: String,
    /// Delivery pipeline configuration
    pub delivery: BraintrustDeliveryConfig,
    /// Content/privacy controls
    pub content: BraintrustContentConfig,
    /// Deployment grade exported in root metadata
    pub deployment_grade: DeploymentGrade,
}

#[derive(Debug, Clone)]
pub struct BraintrustDeliveryConfig {
    pub queue_capacity: usize,
    pub max_batch_size: usize,
    pub flush_interval: Duration,
    pub request_timeout: Duration,
    pub max_retries: u32,
    pub base_retry_delay: Duration,
    pub max_retry_delay: Duration,
}

impl Default for BraintrustDeliveryConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            max_batch_size: 50,
            flush_interval: Duration::from_millis(500),
            request_timeout: Duration::from_secs(10),
            max_retries: 3,
            base_retry_delay: Duration::from_millis(250),
            max_retry_delay: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BraintrustThinkingMode {
    None,
    Summary,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BraintrustPayloadMode {
    Full,
    Summary,
    Redacted,
    None,
}

#[derive(Debug, Clone)]
pub struct BraintrustContentConfig {
    pub record_content: bool,
    pub record_thinking: BraintrustThinkingMode,
    pub tool_args_mode: BraintrustPayloadMode,
    pub tool_results_mode: BraintrustPayloadMode,
    pub debug_payloads: bool,
}

impl Default for BraintrustContentConfig {
    fn default() -> Self {
        Self {
            record_content: false,
            record_thinking: BraintrustThinkingMode::None,
            tool_args_mode: BraintrustPayloadMode::Redacted,
            tool_results_mode: BraintrustPayloadMode::Summary,
            debug_payloads: false,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct BraintrustSessionState {
    harness_id: Option<String>,
    agent_id: Option<String>,
    model_id: Option<String>,
    last_status: Option<String>,
    last_turn_id: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct BraintrustTurnState {
    input_message_id: Option<String>,
    turn_started_sequence: Option<i32>,
    harness_id: Option<String>,
    agent_id: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    retry_attempts: Option<u32>,
    retry_wait_ms: Option<u64>,
    compaction: Option<serde_json::Value>,
    session_status: Option<String>,
}

struct BraintrustState {
    config: BraintrustConfig,
    client: Client,
    sender: mpsc::Sender<BraintrustLogEvent>,
    sessions: Mutex<HashMap<String, BraintrustSessionState>>,
    turns: Mutex<HashMap<String, BraintrustTurnState>>,
    dropped_events: AtomicU64,
    retried_batches: AtomicU64,
    failed_batches: AtomicU64,
}

#[derive(Debug)]
enum DeliveryAttempt {
    Success,
    Retryable(String),
    Permanent(String),
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
        if matches!(env_bool_value("BRAINTRUST_ENABLED"), Some(false)) {
            return None;
        }

        let api_key = std::env::var("BRAINTRUST_API_KEY").ok()?;

        let api_url = std::env::var("BRAINTRUST_API_URL")
            .unwrap_or_else(|_| "https://api.braintrust.dev".to_string());
        let delivery = BraintrustDeliveryConfig {
            queue_capacity: env_usize("BRAINTRUST_QUEUE_CAPACITY", 1024),
            max_batch_size: env_usize("BRAINTRUST_MAX_BATCH_SIZE", 50),
            flush_interval: Duration::from_millis(env_u64("BRAINTRUST_FLUSH_INTERVAL_MS", 500)),
            request_timeout: Duration::from_millis(env_u64(
                "BRAINTRUST_REQUEST_TIMEOUT_MS",
                10_000,
            )),
            max_retries: env_u32("BRAINTRUST_MAX_RETRIES", 3),
            base_retry_delay: Duration::from_millis(env_u64("BRAINTRUST_RETRY_BASE_DELAY_MS", 250)),
            max_retry_delay: Duration::from_millis(env_u64("BRAINTRUST_RETRY_MAX_DELAY_MS", 5_000)),
        };
        let content = BraintrustContentConfig {
            record_content: env_bool("BRAINTRUST_RECORD_CONTENT", false),
            record_thinking: env_thinking_mode(
                "BRAINTRUST_RECORD_THINKING",
                BraintrustThinkingMode::None,
            ),
            tool_args_mode: env_payload_mode(
                "BRAINTRUST_TOOL_ARGS_MODE",
                BraintrustPayloadMode::Redacted,
            ),
            tool_results_mode: env_payload_mode(
                "BRAINTRUST_TOOL_RESULTS_MODE",
                BraintrustPayloadMode::Summary,
            ),
            debug_payloads: env_bool("BRAINTRUST_DEBUG_PAYLOADS", false),
        };
        let deployment_grade = DeploymentGrade::from_env();

        // Try project ID first (no API call needed)
        if let Ok(project_id) = std::env::var("BRAINTRUST_PROJECT_ID") {
            return Some(Self {
                api_key,
                project_id,
                api_url,
                delivery,
                content,
                deployment_grade,
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
                    delivery,
                    content,
                    deployment_grade,
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

fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn env_bool_value(name: &str) -> Option<bool> {
    std::env::var(name).ok().as_deref().and_then(parse_env_bool)
}

fn env_bool(name: &str, default: bool) -> bool {
    env_bool_value(name).unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_payload_mode(name: &str, default: BraintrustPayloadMode) -> BraintrustPayloadMode {
    match std::env::var(name).ok().as_deref() {
        Some("full") => BraintrustPayloadMode::Full,
        Some("summary") => BraintrustPayloadMode::Summary,
        Some("redacted") => BraintrustPayloadMode::Redacted,
        Some("none") => BraintrustPayloadMode::None,
        _ => default,
    }
}

fn env_thinking_mode(name: &str, default: BraintrustThinkingMode) -> BraintrustThinkingMode {
    match std::env::var(name).ok().as_deref() {
        Some("none") => BraintrustThinkingMode::None,
        Some("summary") => BraintrustThinkingMode::Summary,
        Some("full") => BraintrustThinkingMode::Full,
        _ => default,
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
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
struct BraintrustSpanAttributes {
    name: String,
    #[serde(rename = "type")]
    span_type: String,
}

/// Braintrust log event with parent-child support
///
/// According to Braintrust API, spans must include both `span_id` and `root_span_id`, or neither.
/// Everruns root spans self-reference with `turn_id` for both fields so child spans can link to
/// the root without inventing a second identifier.
#[derive(Debug, Clone, Serialize)]
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
    state: Arc<BraintrustState>,
}

impl BraintrustListener {
    /// Create a new Braintrust listener with the given configuration.
    ///
    /// Returns `Err` if the HTTP client cannot be constructed (e.g. a
    /// misconfigured TLS/proxy environment). Telemetry init must never panic
    /// the host process, so callers should treat this as "Braintrust disabled".
    pub fn new(config: BraintrustConfig) -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            .timeout(config.delivery.request_timeout)
            .build()?;
        let (sender, receiver) = mpsc::channel(config.delivery.queue_capacity);
        let state = Arc::new(BraintrustState {
            config,
            client,
            sender,
            sessions: Mutex::new(HashMap::new()),
            turns: Mutex::new(HashMap::new()),
            dropped_events: AtomicU64::new(0),
            retried_batches: AtomicU64::new(0),
            failed_batches: AtomicU64::new(0),
        });

        if tokio::runtime::Handle::try_current().is_ok() {
            Self::spawn_delivery_worker(Arc::clone(&state), receiver);
        } else {
            warn!(
                "Braintrust listener created without an active Tokio runtime; delivery worker not started yet"
            );
        }

        Ok(Self { state })
    }

    /// Create a new listener from environment configuration.
    ///
    /// Returns `None` if configuration is not available OR if the HTTP client
    /// fails to build. A bad TLS/proxy env var must disable Braintrust, never
    /// panic the host process during telemetry init.
    pub fn from_env() -> Option<Self> {
        let config = BraintrustConfig::from_env()?;
        match Self::new(config) {
            Ok(listener) => Some(listener),
            Err(err) => {
                warn!(
                    error = %err,
                    "Failed to build Braintrust HTTP client; Braintrust integration disabled"
                );
                None
            }
        }
    }

    fn spawn_delivery_worker(
        state: Arc<BraintrustState>,
        mut receiver: mpsc::Receiver<BraintrustLogEvent>,
    ) {
        tokio::spawn(async move {
            let mut batch = Vec::with_capacity(state.config.delivery.max_batch_size);
            let mut ticker = time::interval(state.config.delivery.flush_interval);

            loop {
                tokio::select! {
                    maybe_event = receiver.recv() => {
                        match maybe_event {
                            Some(event) => {
                                batch.push(event);
                                if batch.len() >= state.config.delivery.max_batch_size {
                                    Self::flush_batch(&state, &mut batch).await;
                                }
                            }
                            None => {
                                if !batch.is_empty() {
                                    Self::flush_batch(&state, &mut batch).await;
                                }
                                break;
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        if !batch.is_empty() {
                            Self::flush_batch(&state, &mut batch).await;
                        }
                    }
                }
            }
        });
    }

    async fn flush_batch(state: &BraintrustState, batch: &mut Vec<BraintrustLogEvent>) {
        let events = std::mem::take(batch);
        if events.is_empty() {
            return;
        }

        Self::send_events(state, events).await;
    }

    async fn send_events(state: &BraintrustState, events: Vec<BraintrustLogEvent>) {
        let url = format!(
            "{}/v1/project_logs/{}/insert",
            state.config.api_url, state.config.project_id
        );
        let request = BraintrustInsertRequest { events };

        if state.config.content.debug_payloads
            && let Ok(payload) = serde_json::to_string_pretty(&request)
        {
            debug!(
                url = %url,
                payload = %payload,
                "Sending batch to Braintrust"
            );
        }

        for attempt in 0..=state.config.delivery.max_retries {
            match Self::send_batch_attempt(state, &url, &request).await {
                DeliveryAttempt::Success => return,
                DeliveryAttempt::Retryable(reason)
                    if attempt < state.config.delivery.max_retries =>
                {
                    state.retried_batches.fetch_add(1, Ordering::Relaxed);
                    let backoff = Self::retry_delay(&state.config.delivery, attempt);
                    warn!(
                        attempt = attempt + 1,
                        max_retries = state.config.delivery.max_retries,
                        retry_in_ms = backoff.as_millis(),
                        reason = %reason,
                        "Retrying Braintrust batch"
                    );
                    time::sleep(backoff).await;
                }
                DeliveryAttempt::Retryable(reason) | DeliveryAttempt::Permanent(reason) => {
                    state.failed_batches.fetch_add(1, Ordering::Relaxed);
                    error!(reason = %reason, "Failed to send Braintrust batch");
                    return;
                }
            }
        }
    }

    async fn send_batch_attempt(
        state: &BraintrustState,
        url: &str,
        request: &BraintrustInsertRequest,
    ) -> DeliveryAttempt {
        let result = state
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", state.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await;

        match result {
            Ok(response) => {
                if response.status().is_success() {
                    debug!("Successfully sent events to Braintrust");
                    DeliveryAttempt::Success
                } else if response.status().as_u16() == 429 || response.status().is_server_error() {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    DeliveryAttempt::Retryable(format!("HTTP {} {}", status, body))
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    DeliveryAttempt::Permanent(format!("HTTP {} {}", status, body))
                }
            }
            Err(e) => {
                if e.is_timeout() || e.is_connect() || e.is_request() {
                    DeliveryAttempt::Retryable(e.to_string())
                } else {
                    DeliveryAttempt::Permanent(e.to_string())
                }
            }
        }
    }

    fn retry_delay(config: &BraintrustDeliveryConfig, attempt: u32) -> Duration {
        let exponent = 2u64.saturating_pow(attempt.min(12));
        let base_ms = config.base_retry_delay.as_millis() as u64;
        let capped_ms =
            (base_ms.saturating_mul(exponent)).min(config.max_retry_delay.as_millis() as u64);
        let jitter_ms = rand::rng().random_range(0..=capped_ms / 4);
        Duration::from_millis(capped_ms.saturating_add(jitter_ms))
    }

    fn enqueue_event(&self, bt_event: BraintrustLogEvent) {
        if let Err(error) = self.state.sender.try_send(bt_event) {
            self.state.dropped_events.fetch_add(1, Ordering::Relaxed);
            warn!(
                dropped_events = self.state.dropped_events.load(Ordering::Relaxed),
                error = %error,
                "Dropping Braintrust event because the delivery queue is full"
            );
        }
    }

    fn current_session_state(&self, session_id: &str) -> Option<BraintrustSessionState> {
        self.state
            .sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(session_id).cloned())
    }

    fn current_turn_state(&self, turn_id: &str) -> Option<BraintrustTurnState> {
        self.state
            .turns
            .lock()
            .ok()
            .and_then(|turns| turns.get(turn_id).cloned())
    }

    fn upsert_turn_state<F>(&self, turn_id: &str, update: F)
    where
        F: FnOnce(&mut BraintrustTurnState),
    {
        if let Ok(mut turns) = self.state.turns.lock() {
            let state = turns.entry(turn_id.to_string()).or_default();
            update(state);
        }
    }

    fn update_session_state<F>(&self, session_id: &str, update: F)
    where
        F: FnOnce(&mut BraintrustSessionState),
    {
        if let Ok(mut sessions) = self.state.sessions.lock() {
            let state = sessions.entry(session_id.to_string()).or_default();
            update(state);
        }
    }

    fn remove_turn_state(&self, turn_id: &str) {
        if let Ok(mut turns) = self.state.turns.lock() {
            turns.remove(turn_id);
        }
    }

    fn remove_session_state(&self, session_id: &str) {
        if let Ok(mut sessions) = self.state.sessions.lock() {
            sessions.remove(session_id);
        }
    }

    fn annotate_metadata(
        &self,
        event: &Event,
        metadata: &mut serde_json::Value,
        turn_id: Option<&str>,
    ) {
        metadata["session_id"] = serde_json::json!(event.session_id.to_string());
        metadata["deployment_grade"] =
            serde_json::json!(self.state.config.deployment_grade.to_string());

        if let Some(sequence) = event.sequence {
            metadata["session_event_sequence"] = serde_json::json!(sequence);
        }

        if let Some(turn_id) = turn_id {
            metadata["turn_id"] = serde_json::json!(turn_id);
            if let Some(turn_state) = self.current_turn_state(turn_id) {
                if let Some(input_message_id) = turn_state.input_message_id {
                    metadata["input_message_id"] = serde_json::json!(input_message_id);
                }
                if let Some(turn_started_sequence) = turn_state.turn_started_sequence {
                    metadata["turn_started_sequence"] = serde_json::json!(turn_started_sequence);
                }
                if let Some(harness_id) = turn_state.harness_id {
                    metadata["harness_id"] = serde_json::json!(harness_id);
                }
                if let Some(agent_id) = turn_state.agent_id {
                    metadata["agent_id"] = serde_json::json!(agent_id);
                }
                if let Some(model) = turn_state.model {
                    metadata["model"] = serde_json::json!(model);
                }
                if let Some(provider) = turn_state.provider {
                    metadata["provider"] = serde_json::json!(provider);
                }
                if let Some(retry_attempts) = turn_state.retry_attempts {
                    metadata["llm_retry_attempts"] = serde_json::json!(retry_attempts);
                }
                if let Some(retry_wait_ms) = turn_state.retry_wait_ms {
                    metadata["llm_retry_wait_ms"] = serde_json::json!(retry_wait_ms);
                }
                if let Some(compaction) = turn_state.compaction {
                    metadata["llm_compaction"] = compaction;
                }
                if let Some(session_status) = turn_state.session_status {
                    metadata["session_status"] = serde_json::json!(session_status);
                }
            }
        }

        if let Some(session_state) = self.current_session_state(&event.session_id.to_string()) {
            if metadata.get("harness_id").is_none()
                && let Some(harness_id) = session_state.harness_id
            {
                metadata["harness_id"] = serde_json::json!(harness_id);
            }
            if metadata.get("agent_id").is_none()
                && let Some(agent_id) = session_state.agent_id
            {
                metadata["agent_id"] = serde_json::json!(agent_id);
            }
            if metadata.get("model_id").is_none()
                && let Some(model_id) = session_state.model_id
            {
                metadata["model_id"] = serde_json::json!(model_id);
            }
            if metadata.get("session_status").is_none()
                && let Some(status) = session_state.last_status
            {
                metadata["session_status"] = serde_json::json!(status);
            }
            if metadata.get("last_session_turn_id").is_none()
                && let Some(turn_id) = session_state.last_turn_id
            {
                metadata["last_session_turn_id"] = serde_json::json!(turn_id);
            }
        }
    }

    fn summarize_text(text: &str, limit: usize) -> String {
        let summary: String = text.chars().take(limit).collect();
        if text.chars().count() > limit {
            format!("{}...", summary)
        } else {
            summary
        }
    }

    fn summarize_json_value(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => serde_json::json!({
                "type": "object",
                "keys": map.keys().collect::<Vec<_>>(),
            }),
            serde_json::Value::Array(items) => serde_json::json!({
                "type": "array",
                "item_count": items.len(),
            }),
            serde_json::Value::String(text) => serde_json::json!({
                "type": "string",
                "char_count": text.chars().count(),
            }),
            serde_json::Value::Number(_) => serde_json::json!({
                "type": "number",
            }),
            serde_json::Value::Bool(_) => serde_json::json!({
                "type": "bool",
            }),
            serde_json::Value::Null => serde_json::json!({
                "type": "null",
            }),
        }
    }

    fn summarize_content_parts(parts: &[everruns_core::ContentPart]) -> serde_json::Value {
        serde_json::json!({
            "part_count": parts.len(),
            "part_types": parts.iter().map(|part| match part {
                everruns_core::ContentPart::Text(_) => "text",
                everruns_core::ContentPart::Image(_) => "image",
                everruns_core::ContentPart::ImageFile(_) => "image_file",
                everruns_core::ContentPart::ToolCall(_) => "tool_call",
                everruns_core::ContentPart::ToolResult(_) => "tool_result",
            }).collect::<Vec<_>>(),
            "text_part_count": parts.iter().filter(|part| matches!(part, everruns_core::ContentPart::Text(_))).count(),
        })
    }

    fn summarize_messages(messages: &[everruns_core::Message]) -> serde_json::Value {
        serde_json::json!({
            "message_count": messages.len(),
            "roles": messages.iter().map(|message| message.role.to_string()).collect::<Vec<_>>(),
            "phases": messages.iter().filter_map(|message| message.phase.map(|phase| phase.to_string())).collect::<Vec<_>>(),
        })
    }

    fn serialize_tool_arguments(&self, arguments: &serde_json::Value) -> Option<serde_json::Value> {
        match self.state.config.content.tool_args_mode {
            BraintrustPayloadMode::Full => Some(arguments.clone()),
            BraintrustPayloadMode::Summary | BraintrustPayloadMode::Redacted => {
                Some(serde_json::json!({
                    "redacted": true,
                    "summary": Self::summarize_json_value(arguments),
                }))
            }
            BraintrustPayloadMode::None => None,
        }
    }

    fn include_tool_call_labels(&self) -> bool {
        !matches!(
            self.state.config.content.tool_args_mode,
            BraintrustPayloadMode::Redacted | BraintrustPayloadMode::None
        )
    }

    fn serialize_tool_call_for_llm(
        &self,
        id: &str,
        name: &str,
        arguments: &serde_json::Value,
    ) -> serde_json::Value {
        let arguments = self
            .serialize_tool_arguments(arguments)
            .unwrap_or_else(|| serde_json::json!({}));

        serde_json::json!({
            "id": id,
            "type": "function",
            "function": {
                "name": name,
                "arguments": serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string()),
            }
        })
    }

    fn serialize_tool_result(
        &self,
        result: Option<&Vec<everruns_core::ContentPart>>,
        error: Option<&String>,
    ) -> Option<serde_json::Value> {
        match self.state.config.content.tool_results_mode {
            BraintrustPayloadMode::Full => result
                .map(|result| serde_json::json!(result))
                .or_else(|| error.map(|error| serde_json::json!({ "error": error }))),
            BraintrustPayloadMode::Summary | BraintrustPayloadMode::Redacted => result
                .map(|result| {
                    serde_json::json!({
                        "redacted": true,
                        "summary": Self::summarize_content_parts(result),
                    })
                })
                .or_else(|| {
                    error.map(|error| {
                        serde_json::json!({
                            "redacted": true,
                            "error": true,
                            "summary": Self::summarize_json_value(&serde_json::json!(error)),
                        })
                    })
                }),
            BraintrustPayloadMode::None => None,
        }
    }

    fn serialize_tool_result_message_content(
        &self,
        result: Option<&serde_json::Value>,
        error: Option<&String>,
    ) -> String {
        match self.state.config.content.tool_results_mode {
            BraintrustPayloadMode::Full => {
                if let Some(error) = error {
                    format!("Error: {}", error)
                } else if let Some(result) = result {
                    serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string())
                } else {
                    "{}".to_string()
                }
            }
            BraintrustPayloadMode::Summary | BraintrustPayloadMode::Redacted => {
                let sanitized = result
                    .map(|result| {
                        serde_json::json!({
                            "redacted": true,
                            "summary": Self::summarize_json_value(result),
                        })
                    })
                    .or_else(|| {
                        error.map(|error| {
                            serde_json::json!({
                                "redacted": true,
                                "error": true,
                                "summary": Self::summarize_json_value(&serde_json::json!(error)),
                            })
                        })
                    })
                    .unwrap_or_else(|| serde_json::json!({}));

                serde_json::to_string(&sanitized).unwrap_or_else(|_| "{}".to_string())
            }
            BraintrustPayloadMode::None => String::new(),
        }
    }

    fn serialize_message_for_llm_input(
        &self,
        message: &everruns_core::Message,
    ) -> serde_json::Value {
        let mut serialized = message.to_openai_format();

        match message.role {
            everruns_core::MessageRole::Agent if !message.tool_calls().is_empty() => {
                let tool_calls = message
                    .tool_calls()
                    .into_iter()
                    .map(|tool_call| {
                        self.serialize_tool_call_for_llm(
                            &tool_call.id,
                            &tool_call.name,
                            &tool_call.arguments,
                        )
                    })
                    .collect::<Vec<_>>();
                serialized["tool_calls"] = serde_json::Value::Array(tool_calls);
            }
            everruns_core::MessageRole::ToolResult => {
                if let Some(tool_result) = message.tool_result_content() {
                    serialized["content"] =
                        serde_json::json!(self.serialize_tool_result_message_content(
                            tool_result.result.as_ref(),
                            tool_result.error.as_ref(),
                        ));
                }
            }
            _ => {}
        }

        serialized
    }

    fn llm_input_payload(
        &self,
        data: &everruns_core::LlmGenerationData,
    ) -> Option<serde_json::Value> {
        if self.state.config.content.record_content {
            let input: Vec<serde_json::Value> = data
                .messages
                .iter()
                .map(|message| self.serialize_message_for_llm_input(message))
                .collect();
            Some(serde_json::json!(input))
        } else {
            Some(Self::summarize_messages(&data.messages))
        }
    }

    fn llm_output_payload(
        &self,
        data: &everruns_core::LlmGenerationData,
    ) -> Option<serde_json::Value> {
        if self.state.config.content.record_content {
            let tool_calls: Vec<serde_json::Value> = data
                .output
                .tool_calls
                .iter()
                .map(|tool_call| {
                    self.serialize_tool_call_for_llm(
                        &tool_call.id,
                        &tool_call.name,
                        &tool_call.arguments,
                    )
                })
                .collect();
            if tool_calls.is_empty() {
                Some(serde_json::json!({ "text": data.output.text }))
            } else {
                Some(serde_json::json!({
                    "text": data.output.text,
                    "tool_calls": tool_calls,
                }))
            }
        } else {
            Some(serde_json::json!({
                "text_recorded": false,
                "text_present": data.output.text.is_some(),
                "tool_call_count": data.output.tool_calls.len(),
                "tool_names": data.output.tool_calls.iter().map(|tool_call| tool_call.name.clone()).collect::<Vec<_>>(),
            }))
        }
    }

    fn thinking_output_payload(&self, thinking: &str) -> Option<serde_json::Value> {
        match self.state.config.content.record_thinking {
            BraintrustThinkingMode::None => None,
            BraintrustThinkingMode::Summary => Some(serde_json::json!({
                "thinking_preview": Self::summarize_text(thinking, 160),
            })),
            BraintrustThinkingMode::Full => Some(serde_json::json!({
                "thinking": thinking,
            })),
        }
    }

    fn record_turn_started_state(&self, event: &Event, data: &everruns_core::TurnStartedData) {
        let turn_id = data.turn_id.to_string();
        self.upsert_turn_state(&turn_id, |turn_state| {
            turn_state.input_message_id = Some(data.input_message_id.to_string());
            turn_state.turn_started_sequence = event.sequence;
        });
    }

    fn record_reason_started_state(&self, event: &Event, data: &ReasonStartedData) {
        let session_id = event.session_id.to_string();
        let harness_id = data.harness_id.to_string();
        let agent_id = data.agent_id.map(|id| id.to_string());
        let model_id = data
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.model_id.map(|id| id.to_string()));

        self.update_session_state(&session_id, |session_state| {
            session_state.harness_id = Some(harness_id.clone());
            session_state.agent_id = agent_id.clone();
            session_state.model_id = model_id.clone();
        });

        if let Some(turn_id) = event.context.turn_id.as_ref().map(ToString::to_string) {
            self.upsert_turn_state(&turn_id, |turn_state| {
                turn_state.harness_id = Some(harness_id);
                turn_state.agent_id = agent_id;
                if let Some(model) = &data.metadata {
                    turn_state.model = Some(model.model.clone());
                }
            });
        }
    }

    fn record_llm_state(&self, event: &Event, data: &everruns_core::LlmGenerationData) {
        if let Some(turn_id) = event.context.turn_id.as_ref().map(ToString::to_string) {
            self.upsert_turn_state(&turn_id, |turn_state| {
                turn_state.model = Some(data.metadata.model.clone());
                turn_state.provider = data.metadata.provider.clone();
                turn_state.retry_attempts =
                    data.metadata.retry.as_ref().map(|retry| retry.attempts);
                turn_state.retry_wait_ms = data
                    .metadata
                    .retry
                    .as_ref()
                    .map(|retry| retry.total_wait_ms);
                turn_state.compaction = data
                    .metadata
                    .compaction
                    .as_ref()
                    .map(|compaction| serde_json::json!(compaction));
            });
        }
    }

    fn record_session_status(&self, event: &Event, turn_id: Option<&str>, status: &str) {
        let session_id = event.session_id.to_string();
        self.update_session_state(&session_id, |session_state| {
            session_state.last_status = Some(status.to_string());
            session_state.last_turn_id = turn_id.map(ToOwned::to_owned);
        });

        if let Some(turn_id) = turn_id {
            self.upsert_turn_state(turn_id, |turn_state| {
                turn_state.session_status = Some(status.to_string());
            });
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
        data: &everruns_core::TurnStartedData,
    ) -> BraintrustLogEvent {
        let turn_id = data.turn_id.to_string();
        let mut metadata = serde_json::json!({
            "input_message_id": data.input_message_id.to_string(),
            "turn_status": "started",
        });
        self.annotate_metadata(event, &mut metadata, Some(&turn_id));

        // Root span: span_id and root_span_id both reference self (turn_id)
        // This allows child spans to link to the root via root_span_id
        let turn_id_str = turn_id;

        let input = if self.state.config.content.record_content {
            data.input_content
                .as_ref()
                .map(|content| serde_json::json!(content))
                .or_else(|| {
                    Some(serde_json::json!({
                        "input_message_id": data.input_message_id.to_string(),
                    }))
                })
        } else {
            Some(serde_json::json!({
                "input_message_id": data.input_message_id.to_string(),
                "content_recorded": false,
                "has_input_content": data.input_content.is_some(),
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
        data: &everruns_core::TurnCompletedData,
    ) -> BraintrustLogEvent {
        let turn_id = data.turn_id.to_string();
        let mut metadata = serde_json::json!({
            "iterations": data.iterations,
            "turn_status": "completed",
        });
        self.annotate_metadata(event, &mut metadata, Some(&turn_id));

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
        let turn_id_str = turn_id;

        let input = if self.state.config.content.record_content {
            data.input_content
                .as_ref()
                .map(|content| serde_json::json!(content))
        } else {
            Some(serde_json::json!({
                "content_recorded": false,
                "has_input_content": data.input_content.is_some(),
            }))
        };

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
        data: &everruns_core::LlmGenerationData,
    ) -> BraintrustLogEvent {
        // Build metadata
        let mut metadata = serde_json::json!({
            "model": data.metadata.model,
            "generation_success": data.metadata.success,
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
        if let Some(request_options) = &data.metadata.request_options {
            metadata["request_options"] =
                serde_json::to_value(request_options).unwrap_or_else(|_| serde_json::json!({}));
        }
        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }
        if let Some(retry) = &data.metadata.retry {
            metadata["retry"] = serde_json::json!(retry);
        }
        if let Some(compaction) = &data.metadata.compaction {
            metadata["compaction"] = serde_json::json!(compaction);
        }
        self.annotate_metadata(
            event,
            &mut metadata,
            event
                .context
                .turn_id
                .as_ref()
                .map(|turn_id| turn_id.to_string())
                .as_deref(),
        );

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
            input: self.llm_input_payload(data),
            output: self.llm_output_payload(data),
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
        data: &everruns_core::ToolCompletedData,
    ) -> BraintrustLogEvent {
        let input = serde_json::json!({
            "tool_call_id": data.tool_call_id,
            "tool_name": data.tool_name,
            "success": data.success,
            "status": data.status,
        });
        let mut output = serde_json::json!({
            "status": data.status,
        });
        if let Some(serialized_result) =
            self.serialize_tool_result(data.result.as_ref(), data.error.as_ref())
        {
            output["result"] = serialized_result;
        }

        let mut metadata = serde_json::json!({
            "tool_name": data.tool_name,
            "tool_call_id": data.tool_call_id,
            "success": data.success,
            "status": data.status,
        });
        if self.include_tool_call_labels() {
            if let Some(display_name) = &data.display_name {
                metadata["display_name"] = serde_json::json!(display_name);
            }
            if let Some(narration) = &data.narration {
                metadata["narration"] = serde_json::json!(narration);
            }
        }

        self.annotate_metadata(
            event,
            &mut metadata,
            event
                .context
                .turn_id
                .as_ref()
                .map(|turn_id| turn_id.to_string())
                .as_deref(),
        );

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
        let turn_id = data.turn_id.to_string();
        let mut metadata = serde_json::json!({
            "turn_status": "failed",
        });
        self.annotate_metadata(event, &mut metadata, Some(&turn_id));

        if let Some(error_code) = &data.error_code {
            metadata["error_code"] = serde_json::json!(error_code);
        }

        // Root span: span_id and root_span_id both reference self (turn_id)
        let turn_id_str = turn_id;

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
        let turn_id = data.turn_id.to_string();
        let mut metadata = serde_json::json!({
            "turn_status": "cancelled",
        });
        self.annotate_metadata(event, &mut metadata, Some(&turn_id));

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
        let turn_id_str = turn_id;

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
            "agent_id": data.agent_id.map(|id| id.to_string()),
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

        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }
        self.annotate_metadata(
            event,
            &mut metadata,
            event
                .context
                .turn_id
                .as_ref()
                .map(|turn_id| turn_id.to_string())
                .as_deref(),
        );

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
            "success": data.success,
            "has_tool_calls": data.has_tool_calls,
            "tool_call_count": data.tool_call_count,
        });

        let output = serde_json::json!({
            "success": data.success,
            "has_tool_calls": data.has_tool_calls,
            "tool_call_count": data.tool_call_count,
            "text_preview": if self.state.config.content.record_content {
                data.text_preview.clone()
            } else {
                None
            },
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

        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }
        self.annotate_metadata(
            event,
            &mut metadata,
            event
                .context
                .turn_id
                .as_ref()
                .map(|turn_id| turn_id.to_string())
                .as_deref(),
        );

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
        let turn_id = data.turn_id.to_string();
        let mut metadata = serde_json::json!({
            "thinking_status": "started",
        });
        self.annotate_metadata(event, &mut metadata, Some(&turn_id));

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
        let turn_id = data.turn_id.to_string();
        let mut metadata = serde_json::json!({
            "thinking_length": data.thinking.len(),
        });
        self.annotate_metadata(event, &mut metadata, Some(&turn_id));

        let output = self.thinking_output_payload(&data.thinking);

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
            output,
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
            "tool_count": data.tool_calls.len(),
        });
        if let Some(headline) = &data.headline {
            metadata["headline"] = serde_json::json!(headline);
        }

        let input = serde_json::json!({
            "tool_calls": data.tool_calls.iter().map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "name": tc.name,
                    "display_name": tc.display_name,
                    "narration": tc.narration,
                })
            }).collect::<Vec<_>>(),
        });

        // Parent-child linking using OTel-style span fields from context
        let (span_id, root_span_id, span_parents) = Self::compute_child_span_linkage(event);

        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }
        self.annotate_metadata(
            event,
            &mut metadata,
            event
                .context
                .turn_id
                .as_ref()
                .map(|turn_id| turn_id.to_string())
                .as_deref(),
        );

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
            "completed": data.completed,
            "success_count": data.success_count,
            "error_count": data.error_count,
        });
        if let Some(headline) = &data.headline {
            metadata["headline"] = serde_json::json!(headline);
        }

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

        if let Some(exec_id) = &event.context.exec_id {
            metadata["exec_id"] = serde_json::json!(exec_id.to_string());
        }
        self.annotate_metadata(
            event,
            &mut metadata,
            event
                .context
                .turn_id
                .as_ref()
                .map(|turn_id| turn_id.to_string())
                .as_deref(),
        );

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
        let mut input = serde_json::json!({
            "tool_call_id": data.tool_call.id,
            "tool_name": data.tool_call.name,
        });
        if let Some(arguments) = self.serialize_tool_arguments(&data.tool_call.arguments) {
            input["arguments"] = arguments;
        }

        let mut metadata = serde_json::json!({
            "tool_name": data.tool_call.name,
            "tool_call_id": data.tool_call.id,
        });
        if self.include_tool_call_labels() {
            if let Some(display_name) = &data.display_name {
                metadata["display_name"] = serde_json::json!(display_name);
            }
            if let Some(narration) = &data.narration {
                metadata["narration"] = serde_json::json!(narration);
            }
        }

        self.annotate_metadata(
            event,
            &mut metadata,
            event
                .context
                .turn_id
                .as_ref()
                .map(|turn_id| turn_id.to_string())
                .as_deref(),
        );

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

    fn convert_session_lifecycle(
        &self,
        event: &Event,
        lifecycle_name: &str,
        turn_id: Option<&str>,
        payload: serde_json::Value,
    ) -> BraintrustLogEvent {
        let mut metadata = serde_json::json!({
            "session_lifecycle": lifecycle_name,
        });
        self.annotate_metadata(event, &mut metadata, turn_id);

        BraintrustLogEvent {
            id: event.id.to_string(),
            created: event.ts,
            input: None,
            output: Some(payload),
            error: None,
            metadata,
            metrics: None,
            span_attributes: BraintrustSpanAttributes {
                name: format!("session {}", lifecycle_name),
                span_type: "task".to_string(),
            },
            tags: event.tags.clone(),
            span_id: None,
            root_span_id: None,
            span_parents: None,
            is_merge: None,
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
                self.record_turn_started_state(event, data);
                self.convert_turn_started(event, data)
            }
            EventData::TurnCompleted(data) => {
                debug!(turn_id = %data.turn_id, "Processing turn.completed for Braintrust");
                self.record_session_status(event, Some(&data.turn_id.to_string()), "idle");
                self.convert_turn_completed(event, data)
            }
            EventData::TurnFailed(data) => {
                debug!(turn_id = %data.turn_id, "Processing turn.failed for Braintrust");
                self.record_session_status(event, Some(&data.turn_id.to_string()), "idle");
                self.convert_turn_failed(event, data)
            }
            EventData::TurnCancelled(data) => {
                debug!(turn_id = %data.turn_id, "Processing turn.cancelled for Braintrust");
                self.record_session_status(event, Some(&data.turn_id.to_string()), "idle");
                self.convert_turn_cancelled(event, data)
            }

            // Atom lifecycle events (reason/act phases)
            EventData::ReasonStarted(data) => {
                debug!(agent_id = ?data.agent_id, "Processing reason.started for Braintrust");
                self.record_reason_started_state(event, data);
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
                self.record_llm_state(event, data);
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

            EventData::SessionStarted(data) => {
                self.update_session_state(&event.session_id.to_string(), |session_state| {
                    session_state.harness_id = Some(data.harness_id.to_string());
                    session_state.agent_id = data.agent_id.map(|id| id.to_string());
                    session_state.model_id = data.model_id.map(|id| id.to_string());
                    session_state.last_status = Some("started".to_string());
                });
                self.convert_session_lifecycle(
                    event,
                    "started",
                    None,
                    serde_json::json!({
                        "harness_id": data.harness_id.to_string(),
                        "agent_id": data.agent_id.map(|id| id.to_string()),
                        "model_id": data.model_id.map(|id| id.to_string()),
                    }),
                )
            }
            EventData::SessionActivated(data) => {
                let turn_id = data.turn_id.to_string();
                self.record_session_status(event, Some(&turn_id), "active");
                self.convert_session_lifecycle(
                    event,
                    "activated",
                    Some(&turn_id),
                    serde_json::json!({
                        "turn_id": turn_id,
                        "input_message_id": data.input_message_id.to_string(),
                        "status": "active",
                    }),
                )
            }
            EventData::SessionIdled(data) => {
                let turn_id = data.turn_id.to_string();
                self.record_session_status(event, Some(&turn_id), "idle");
                self.convert_session_lifecycle(
                    event,
                    "idled",
                    Some(&turn_id),
                    serde_json::json!({
                        "turn_id": turn_id,
                        "iterations": data.iterations,
                        "usage": data.usage,
                        "status": "idle",
                    }),
                )
            }

            _ => return, // Ignore other event types
        };
        self.enqueue_event(bt_event);

        match &event.data {
            EventData::TurnCompleted(data) => self.remove_turn_state(&data.turn_id.to_string()),
            EventData::TurnFailed(data) => self.remove_turn_state(&data.turn_id.to_string()),
            EventData::TurnCancelled(data) => self.remove_turn_state(&data.turn_id.to_string()),
            EventData::SessionIdled(_) => self.remove_session_state(&event.session_id.to_string()),
            _ => {}
        }
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
            // Session lifecycle
            SESSION_STARTED,
            SESSION_ACTIVATED,
            SESSION_IDLED,
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
    use everruns_core::events::{
        EventContext, EventData, LlmGenerationData, LlmGenerationMetadata, LlmGenerationOutput,
        ReasonCompletedData, ReasonStartedData, TokenUsage, ToolCompletedData, ToolStartedData,
        TurnCompletedData, TurnStartedData,
    };
    use everruns_core::message::Message;
    use everruns_provider::tool_types::ToolCall;
    use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
    use serde_json::json;
    use tokio::time::sleep;
    use uuid::Uuid;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config() -> BraintrustConfig {
        BraintrustConfig {
            api_key: "test-api-key".to_string(),
            project_id: "test-project-id".to_string(),
            api_url: "https://api.braintrust.dev".to_string(),
            delivery: BraintrustDeliveryConfig {
                flush_interval: Duration::from_millis(20),
                request_timeout: Duration::from_millis(200),
                ..BraintrustDeliveryConfig::default()
            },
            content: BraintrustContentConfig::default(),
            deployment_grade: DeploymentGrade::Dev,
        }
    }

    #[test]
    fn test_listener_creation() {
        let listener = BraintrustListener::new(test_config()).unwrap();
        assert_eq!(listener.name(), "BraintrustListener");
    }

    /// Telemetry init must never panic the host process. `new` returns a
    /// `Result` so a misconfigured TLS/proxy environment disables Braintrust
    /// instead of aborting the process. With no env config, `from_env` yields
    /// `None` (disabled) rather than panicking.
    #[test]
    fn test_init_never_panics_and_disables_on_missing_config() {
        // Valid config builds successfully (Ok, not a panic).
        assert!(BraintrustListener::new(test_config()).is_ok());

        // `from_env` must not panic; with the API key absent it returns None.
        // Guard against an ambient key in the environment so the test is
        // deterministic in any CI/runtime.
        let key_present = std::env::var("BRAINTRUST_API_KEY").is_ok();
        let listener = BraintrustListener::from_env();
        if !key_present {
            assert!(
                listener.is_none(),
                "from_env should disable Braintrust when no API key is configured"
            );
        }
    }

    #[test]
    fn test_event_types() {
        let listener = BraintrustListener::new(test_config()).unwrap();
        let types = listener.event_types().unwrap();
        // 16 event types: 4 turn lifecycle + 4 atom lifecycle + 2 thinking + 1 llm + 2 tool + 3 session lifecycle
        assert_eq!(types.len(), 16);
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
        // Session lifecycle
        assert!(types.contains(&SESSION_STARTED));
        assert!(types.contains(&SESSION_ACTIVATED));
        assert!(types.contains(&SESSION_IDLED));
    }

    #[test]
    fn test_convert_turn_started() {
        let listener = BraintrustListener::new(test_config()).unwrap();

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
        let listener = BraintrustListener::new(test_config()).unwrap();

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
                    actual_cost_usd: None,
                    estimated_cost_usd: None,
                    effective_cost_usd: None,
                }),
                duration_ms: Some(100),
                time_to_first_token_ms: Some(25),
                success: true,
                error: None,
                finish_reasons: Some(vec!["stop".to_string()]),
                response_id: Some("resp_123".to_string()),
                retry: None,
                compaction: None,
                request_options: None,
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
        let listener = BraintrustListener::new(test_config()).unwrap();

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
                actual_cost_usd: None,
                estimated_cost_usd: None,
                effective_cost_usd: None,
            }),
            input_content: Some("Hello, how are you?".to_string()),
            final_message_id: None,
            final_answer_preview: None,
            time_to_first_token_ms: None,
            tool_call_count: None,
            llm_call_count: None,
            status: None,
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
        let listener = BraintrustListener::new(test_config()).unwrap();
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
            final_message_id: None,
            final_answer_preview: None,
            time_to_first_token_ms: None,
            tool_call_count: None,
            llm_call_count: None,
            status: None,
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
        let listener = BraintrustListener::new(test_config()).unwrap();
        let turn_id = TurnId::new();
        let reason_span_id = Uuid::now_v7().to_string();

        // Create event context with proper span linkage
        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(reason_span_id.clone());
        context.parent_span_id = Some(turn_id.to_string());

        let data = ReasonStartedData {
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
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
        let listener = BraintrustListener::new(test_config()).unwrap();
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
                compaction: None,
                request_options: None,
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
        use everruns_core::events::ToolCallSummary;

        let listener = BraintrustListener::new(test_config()).unwrap();
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
                    display_name: None,
                    narration: None,
                    completed_narration: None,
                },
                ToolCallSummary {
                    id: "call_2".to_string(),
                    name: "fetch".to_string(),
                    display_name: None,
                    narration: None,
                    completed_narration: None,
                },
            ],
            headline: None,
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
        let listener = BraintrustListener::new(test_config()).unwrap();
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
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
            duration_ms: Some(200),
            capability_id: None,
            capability_name: None,
            narration: None,
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
        let listener = BraintrustListener::new(test_config()).unwrap();
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
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
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
        use everruns_core::events::ToolCallSummary;

        let listener = BraintrustListener::new(test_config()).unwrap();
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
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
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
                compaction: None,
                request_options: None,
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
                display_name: None,
                narration: None,
                completed_narration: None,
            }],
            headline: None,
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
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
            duration_ms: Some(75),
            capability_id: None,
            capability_name: None,
            narration: None,
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
        let listener = BraintrustListener::new(test_config()).unwrap();
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
        let listener = BraintrustListener::new(test_config()).unwrap();
        let turn_id = TurnId::new();

        // turn.completed should have is_merge = Some(true)
        let turn_data = TurnCompletedData {
            turn_id,
            iterations: 1,
            duration_ms: Some(1000),
            usage: None,
            input_content: Some("Test".to_string()),
            final_message_id: None,
            final_answer_preview: None,
            time_to_first_token_ms: None,
            tool_call_count: None,
            llm_call_count: None,
            status: None,
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
        let listener = BraintrustListener::new(test_config()).unwrap();
        let turn_id = TurnId::new();
        let reason_span_id = Uuid::now_v7().to_string();

        let mut context = EventContext::empty();
        context.turn_id = Some(turn_id);
        context.trace_id = Some(turn_id.to_string());
        context.span_id = Some(reason_span_id.clone());
        context.parent_span_id = Some(turn_id.to_string());

        // reason.started - should NOT have _is_merge
        let started_data = ReasonStartedData {
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
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
        use everruns_core::events::ToolCallSummary;

        let listener = BraintrustListener::new(test_config()).unwrap();
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
                display_name: None,
                narration: None,
                completed_narration: None,
            }],
            headline: None,
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
            headline: None,
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
        use everruns_provider::tool_types::ToolCall;

        let listener = BraintrustListener::new(test_config()).unwrap();
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
            tool_call_fingerprint: None,
            display_name: None,
            narration: None,
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
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
            duration_ms: Some(100),
            capability_id: None,
            capability_name: None,
            narration: None,
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
        let listener = BraintrustListener::new(test_config()).unwrap();
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
        let mut config = test_config();
        config.content.record_thinking = BraintrustThinkingMode::Full;
        let listener = BraintrustListener::new(config).unwrap();
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
    fn test_turn_started_metadata_includes_session_grouping_fields() {
        let listener = BraintrustListener::new(test_config()).unwrap();
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        listener.update_session_state(&session_id.to_string(), |session_state| {
            session_state.harness_id = Some(HarnessId::new().to_string());
            session_state.agent_id = Some(AgentId::new().to_string());
            session_state.last_status = Some("active".to_string());
        });

        let event = Event::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            EventData::TurnStarted(TurnStartedData {
                turn_id,
                input_message_id,
                input_content: Some("hello braintrust".to_string()),
            }),
        )
        .with_sequence(42);

        let data = match &event.data {
            EventData::TurnStarted(data) => data.clone(),
            _ => unreachable!(),
        };
        listener.record_turn_started_state(&event, &data);
        let bt_event = listener.convert_turn_started(&event, &data);

        assert_eq!(bt_event.metadata["session_id"], session_id.to_string());
        assert_eq!(bt_event.metadata["turn_id"], turn_id.to_string());
        assert_eq!(
            bt_event.metadata["input_message_id"],
            input_message_id.to_string()
        );
        assert_eq!(bt_event.metadata["session_event_sequence"], 42);
        assert_eq!(bt_event.metadata["turn_started_sequence"], 42);
        assert_eq!(bt_event.metadata["deployment_grade"], "dev");
        assert_eq!(bt_event.metadata["session_status"], "active");
        assert!(bt_event.metadata.get("harness_id").is_some());
        assert!(bt_event.metadata.get("agent_id").is_some());
    }

    #[test]
    fn test_tool_started_redacts_arguments_when_configured() {
        let listener = BraintrustListener::new(test_config()).unwrap();
        let turn_id = TurnId::new();
        let event = Event::new(
            SessionId::new(),
            EventContext::turn(turn_id, MessageId::new()),
            EventData::ToolStarted(ToolStartedData {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "exec".to_string(),
                    arguments: json!({"secret": "value", "path": "/tmp/test"}),
                },
                tool_call_fingerprint: None,
                display_name: Some("Execute".to_string()),
                narration: Some("Running exec".to_string()),
            }),
        );

        let data = match &event.data {
            EventData::ToolStarted(data) => data.clone(),
            _ => unreachable!(),
        };
        let bt_event = listener.convert_tool_call_started(&event, &data);
        assert_eq!(
            bt_event.input.as_ref().unwrap()["arguments"]["redacted"],
            true
        );
        assert!(
            bt_event.input.as_ref().unwrap()["arguments"]["summary"]["keys"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "secret")
        );
        assert!(bt_event.metadata.get("display_name").is_none());
        assert!(bt_event.metadata.get("narration").is_none());
    }

    #[test]
    fn test_tool_started_includes_labels_when_tool_args_full() {
        let mut config = test_config();
        config.content.tool_args_mode = BraintrustPayloadMode::Full;
        let listener = BraintrustListener::new(config).unwrap();
        let turn_id = TurnId::new();
        let event = Event::new(
            SessionId::new(),
            EventContext::turn(turn_id, MessageId::new()),
            EventData::ToolStarted(ToolStartedData {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "exec".to_string(),
                    arguments: json!({"secret": "value", "path": "/tmp/test"}),
                },
                tool_call_fingerprint: None,
                display_name: Some("Execute".to_string()),
                narration: Some("Running exec".to_string()),
            }),
        );

        let data = match &event.data {
            EventData::ToolStarted(data) => data.clone(),
            _ => unreachable!(),
        };
        let bt_event = listener.convert_tool_call_started(&event, &data);
        assert_eq!(bt_event.metadata["display_name"], "Execute");
        assert_eq!(bt_event.metadata["narration"], "Running exec");
    }

    #[test]
    fn test_parse_env_bool_accepts_common_boolean_values() {
        for value in ["true", "TRUE", "1", "yes", "on"] {
            assert_eq!(parse_env_bool(value), Some(true));
        }
        for value in ["false", "FALSE", "0", "no", "off"] {
            assert_eq!(parse_env_bool(value), Some(false));
        }
        assert_eq!(parse_env_bool("maybe"), None);
    }

    #[test]
    fn test_turn_started_without_content_recording_omits_input_preview() {
        let listener = BraintrustListener::new(test_config()).unwrap();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let data = TurnStartedData {
            turn_id,
            input_message_id,
            input_content: Some("secret prompt".to_string()),
        };
        let event = Event::new(
            SessionId::new(),
            EventContext::turn(turn_id, input_message_id),
            EventData::TurnStarted(data.clone()),
        );

        let bt_event = listener.convert_turn_started(&event, &data);
        let input = bt_event.input.unwrap();
        assert_eq!(input["content_recorded"], false);
        assert_eq!(input["has_input_content"], true);
        assert!(!input.as_object().unwrap().contains_key("input_preview"));
    }

    #[test]
    fn test_llm_output_without_content_recording_omits_text_preview() {
        let listener = BraintrustListener::new(test_config()).unwrap();
        let data = LlmGenerationData {
            messages: vec![Message::user("Hello")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Sensitive completion".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    arguments: json!({"query": "rust"}),
                }],
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
                compaction: None,
                request_options: None,
            },
        };

        let output = listener.llm_output_payload(&data).unwrap();
        assert_eq!(output["text_recorded"], false);
        assert_eq!(output["text_present"], true);
        assert!(!output.as_object().unwrap().contains_key("text_preview"));
    }

    #[test]
    fn test_llm_content_recording_respects_tool_payload_modes() {
        let mut config = test_config();
        config.content.record_content = true;
        let listener = BraintrustListener::new(config).unwrap();
        let data = LlmGenerationData {
            messages: vec![
                Message::assistant_with_tools(
                    "Calling tool",
                    vec![ToolCall {
                        id: "call_1".to_string(),
                        name: "search".to_string(),
                        arguments: json!({"secret": "value", "query": "rust"}),
                    }],
                ),
                Message::tool_result(
                    "call_1",
                    Some(json!({"secret_result": "top secret", "count": 3})),
                    None,
                ),
            ],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Done".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call_2".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({"token": "shh", "path": "/tmp/out"}),
                }],
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
                compaction: None,
                request_options: None,
            },
        };

        let input = listener.llm_input_payload(&data).unwrap();
        let messages = input.as_array().unwrap();
        let assistant_args = messages[0]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert!(assistant_args.contains("\"redacted\":true"));
        assert!(!assistant_args.contains("value"));

        let tool_result_content = messages[1]["content"].as_str().unwrap();
        assert!(tool_result_content.contains("\"redacted\":true"));
        assert!(!tool_result_content.contains("top secret"));

        let output = listener.llm_output_payload(&data).unwrap();
        let output_args = output["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert!(output_args.contains("\"redacted\":true"));
        assert!(!output_args.contains("shh"));
    }

    #[test]
    fn test_tool_completed_summary_omits_text_preview() {
        let listener = BraintrustListener::new(test_config()).unwrap();
        let turn_id = TurnId::new();
        let data = ToolCompletedData {
            tool_call_id: "call_123".to_string(),
            tool_name: "search".to_string(),
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: Some(vec![everruns_core::ContentPart::text(
                "sensitive tool output",
            )]),
            error: None,
            duration_ms: Some(50),
            capability_id: None,
            capability_name: None,
            narration: None,
        };
        let event = Event::new(
            SessionId::new(),
            EventContext::turn(turn_id, MessageId::new()),
            EventData::ToolCompleted(data.clone()),
        );

        let bt_event = listener.convert_tool_call_completed(&event, &data);
        let result = bt_event.output.unwrap()["result"].clone();
        assert_eq!(result["redacted"], true);
        assert_eq!(result["summary"]["text_part_count"], 1);
        assert!(!result.to_string().contains("sensitive tool output"));
        assert!(
            !result["summary"]
                .as_object()
                .unwrap()
                .contains_key("text_preview")
        );
    }

    #[tokio::test]
    async fn test_session_idled_prunes_session_state() {
        let listener = BraintrustListener::new(test_config()).unwrap();
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();

        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::SessionStarted(everruns_core::events::SessionStartedData {
                    harness_id: HarnessId::from_seed(1),
                    agent_id: None,
                    model_id: None,
                }),
            ))
            .await;
        assert!(
            listener
                .current_session_state(&session_id.to_string())
                .is_some()
        );

        listener
            .on_event(&Event::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                EventData::SessionIdled(everruns_core::events::SessionIdledData {
                    turn_id,
                    iterations: Some(1),
                    usage: None,
                }),
            ))
            .await;

        assert!(
            listener
                .current_session_state(&session_id.to_string())
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_on_event_batches_multiple_events_into_one_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/project_logs/test-project-id/insert"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let mut config = test_config();
        config.api_url = server.uri();
        config.delivery.flush_interval = Duration::from_millis(20);
        config.delivery.max_batch_size = 10;
        let listener = BraintrustListener::new(config).unwrap();

        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let start = Event::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            EventData::TurnStarted(TurnStartedData {
                turn_id,
                input_message_id,
                input_content: Some("hello".to_string()),
            }),
        );
        let completed = Event::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            EventData::TurnCompleted(TurnCompletedData {
                turn_id,
                iterations: 1,
                duration_ms: Some(5),
                usage: None,
                input_content: Some("hello".to_string()),
                final_message_id: None,
                final_answer_preview: None,
                time_to_first_token_ms: None,
                tool_call_count: None,
                llm_call_count: None,
                status: None,
            }),
        );

        listener.on_event(&start).await;
        listener.on_event(&completed).await;
        sleep(Duration::from_millis(80)).await;

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["events"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_on_event_retries_after_429() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/project_logs/test-project-id/insert"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/project_logs/test-project-id/insert"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let mut config = test_config();
        config.api_url = server.uri();
        config.delivery.flush_interval = Duration::from_millis(10);
        config.delivery.base_retry_delay = Duration::from_millis(5);
        config.delivery.max_retry_delay = Duration::from_millis(10);
        let listener = BraintrustListener::new(config).unwrap();

        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let event = Event::new(
            SessionId::new(),
            EventContext::turn(turn_id, input_message_id),
            EventData::TurnStarted(TurnStartedData {
                turn_id,
                input_message_id,
                input_content: Some("hello".to_string()),
            }),
        );
        listener.on_event(&event).await;
        sleep(Duration::from_millis(120)).await;

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(listener.state.retried_batches.load(Ordering::Relaxed) >= 1);
        assert_eq!(listener.state.failed_batches.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_on_event_retries_after_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/project_logs/test-project-id/insert"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(50)))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/project_logs/test-project-id/insert"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let mut config = test_config();
        config.api_url = server.uri();
        config.delivery.flush_interval = Duration::from_millis(10);
        config.delivery.request_timeout = Duration::from_millis(10);
        config.delivery.base_retry_delay = Duration::from_millis(5);
        config.delivery.max_retry_delay = Duration::from_millis(10);
        let listener = BraintrustListener::new(config).unwrap();

        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let event = Event::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            EventData::TurnStarted(TurnStartedData {
                turn_id,
                input_message_id,
                input_content: Some("hello".to_string()),
            }),
        );
        listener.on_event(&event).await;
        sleep(Duration::from_millis(180)).await;

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(listener.state.retried_batches.load(Ordering::Relaxed) >= 1);
        assert_eq!(listener.state.failed_batches.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_thinking_spans_merge_correctly() {
        let listener = BraintrustListener::new(test_config()).unwrap();
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
