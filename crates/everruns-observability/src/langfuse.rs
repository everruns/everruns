// Langfuse Backend Implementation
//
// Implements observability via Langfuse's ingestion API.
// Uses batched HTTP requests rather than OpenTelemetry for simplicity
// and better control over Langfuse-specific data model.
//
// Langfuse data model:
// - Trace: Top-level container (maps to session/run)
// - Span: Generic operation span
// - Generation: LLM call with model, tokens, cost
// - Event: Point-in-time event within a trace

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::backend::{ObservabilityBackend, ObservabilityError, ObservabilityEvent};
use crate::config::LangfuseConfig;

/// Langfuse ingestion event types
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum LangfuseIngestionEvent {
    TraceCreate(TraceCreateBody),
    SpanCreate(SpanCreateBody),
    SpanUpdate(SpanUpdateBody),
    GenerationCreate(GenerationCreateBody),
    GenerationUpdate(GenerationUpdateBody),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceCreateBody {
    id: String,
    timestamp: DateTime<Utc>,
    name: Option<String>,
    user_id: Option<String>,
    session_id: Option<String>,
    release: Option<String>,
    version: Option<String>,
    metadata: Option<serde_json::Value>,
    tags: Option<Vec<String>>,
    public: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpanCreateBody {
    id: String,
    trace_id: String,
    parent_observation_id: Option<String>,
    name: String,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    metadata: Option<serde_json::Value>,
    level: Option<String>,
    status_message: Option<String>,
    input: Option<serde_json::Value>,
    output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpanUpdateBody {
    id: String,
    trace_id: String,
    end_time: Option<DateTime<Utc>>,
    metadata: Option<serde_json::Value>,
    level: Option<String>,
    status_message: Option<String>,
    output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationCreateBody {
    id: String,
    trace_id: String,
    parent_observation_id: Option<String>,
    name: String,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    model: Option<String>,
    model_parameters: Option<serde_json::Value>,
    input: Option<serde_json::Value>,
    output: Option<serde_json::Value>,
    usage: Option<UsageBody>,
    metadata: Option<serde_json::Value>,
    level: Option<String>,
    status_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationUpdateBody {
    id: String,
    trace_id: String,
    end_time: Option<DateTime<Utc>>,
    model: Option<String>,
    output: Option<serde_json::Value>,
    usage: Option<UsageBody>,
    metadata: Option<serde_json::Value>,
    level: Option<String>,
    status_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageBody {
    input: Option<i64>,
    output: Option<i64>,
    total: Option<i64>,
    unit: Option<String>,
}

/// Batch request to Langfuse ingestion API
#[derive(Debug, Clone, Serialize)]
struct IngestionBatch {
    batch: Vec<BatchItem>,
    metadata: Option<BatchMetadata>,
}

#[derive(Debug, Clone, Serialize)]
struct BatchItem {
    id: String,
    timestamp: DateTime<Utc>,
    #[serde(flatten)]
    body: LangfuseIngestionEvent,
}

#[derive(Debug, Clone, Serialize)]
struct BatchMetadata {
    sdk_name: String,
    sdk_version: String,
    public_key: String,
}

/// Response from Langfuse ingestion API
#[derive(Debug, Clone, Deserialize)]
struct IngestionResponse {
    #[allow(dead_code)]
    successes: Vec<SuccessItem>,
    errors: Vec<ErrorItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct SuccessItem {
    id: String,
    status: i32,
}

#[derive(Debug, Clone, Deserialize)]
struct ErrorItem {
    id: String,
    status: i32,
    message: Option<String>,
    error: Option<String>,
}

/// Langfuse observability backend
pub struct LangfuseBackend {
    config: LangfuseConfig,
    client: Client,
    batch: Arc<Mutex<Vec<BatchItem>>>,
}

impl LangfuseBackend {
    /// Create a new Langfuse backend from configuration
    pub fn new(config: LangfuseConfig) -> Result<Self, ObservabilityError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ObservabilityError::Config(e.to_string()))?;

        Ok(Self {
            config,
            client,
            batch: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Create from environment configuration
    pub fn from_env() -> Result<Option<Self>, ObservabilityError> {
        match LangfuseConfig::from_env() {
            Some(config) => Ok(Some(Self::new(config)?)),
            None => Ok(None),
        }
    }

    /// Add an event to the batch
    async fn add_to_batch(&self, event: LangfuseIngestionEvent) {
        let item = BatchItem {
            id: uuid::Uuid::now_v7().to_string(),
            timestamp: Utc::now(),
            body: event,
        };

        let should_flush = {
            let mut batch = self.batch.lock().await;
            batch.push(item);
            batch.len() >= self.config.max_batch_size
        };

        if should_flush {
            if let Err(e) = self.flush().await {
                warn!(error = %e, "Failed to auto-flush Langfuse batch");
            }
        }
    }

    /// Send the batch to Langfuse
    async fn send_batch(&self, items: Vec<BatchItem>) -> Result<(), ObservabilityError> {
        if items.is_empty() {
            return Ok(());
        }

        let batch = IngestionBatch {
            batch: items,
            metadata: Some(BatchMetadata {
                sdk_name: "everruns-observability".to_string(),
                sdk_version: env!("CARGO_PKG_VERSION").to_string(),
                public_key: self.config.public_key.clone(),
            }),
        };

        let url = format!(
            "{}/api/public/ingestion",
            self.config.host.trim_end_matches('/')
        );

        debug!(url = %url, batch_size = batch.batch.len(), "Sending batch to Langfuse");

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.config.auth_header())
            .header("Content-Type", "application/json")
            .json(&batch)
            .send()
            .await
            .map_err(|e| ObservabilityError::Connection(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!(status = %status, body = %body, "Langfuse ingestion failed");
            return Err(ObservabilityError::Export(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        let result: IngestionResponse = response
            .json()
            .await
            .map_err(|e| ObservabilityError::Serialization(e.to_string()))?;

        if !result.errors.is_empty() {
            for err in &result.errors {
                warn!(
                    id = %err.id,
                    status = err.status,
                    message = ?err.message,
                    error = ?err.error,
                    "Langfuse ingestion error"
                );
            }
        }

        debug!(
            successes = result.successes.len(),
            errors = result.errors.len(),
            "Langfuse batch sent"
        );

        Ok(())
    }

    /// Convert ObservabilityEvent to Langfuse events
    fn convert_event(&self, event: &ObservabilityEvent) -> Vec<LangfuseIngestionEvent> {
        match event {
            ObservabilityEvent::TraceStarted {
                trace_id,
                session_id,
                agent_id,
                metadata,
                timestamp,
            } => {
                vec![LangfuseIngestionEvent::TraceCreate(TraceCreateBody {
                    id: trace_id.clone(),
                    timestamp: *timestamp,
                    name: Some(format!("Session {}", session_id)),
                    user_id: agent_id.clone(),
                    session_id: Some(session_id.clone()),
                    release: self.config.release.clone(),
                    version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    metadata: if metadata.is_empty() {
                        None
                    } else {
                        Some(serde_json::to_value(metadata).unwrap_or_default())
                    },
                    tags: Some(vec!["everruns".to_string()]),
                    public: None,
                })]
            }

            ObservabilityEvent::TraceCompleted {
                trace_id,
                session_id: _,
                total_iterations,
                success,
                error,
                metadata,
                timestamp,
            } => {
                // Create a final span to mark completion
                let mut meta = metadata.clone();
                meta.insert(
                    "total_iterations".to_string(),
                    serde_json::json!(total_iterations),
                );
                meta.insert("success".to_string(), serde_json::json!(success));
                if let Some(err) = error {
                    meta.insert("error".to_string(), serde_json::json!(err));
                }

                vec![LangfuseIngestionEvent::SpanCreate(SpanCreateBody {
                    id: format!("{}-completion", trace_id),
                    trace_id: trace_id.clone(),
                    parent_observation_id: None,
                    name: if *success {
                        "session.completed".to_string()
                    } else {
                        "session.failed".to_string()
                    },
                    start_time: *timestamp,
                    end_time: Some(*timestamp),
                    metadata: Some(serde_json::to_value(meta).unwrap_or_default()),
                    level: Some(if *success {
                        "DEFAULT".to_string()
                    } else {
                        "ERROR".to_string()
                    }),
                    status_message: error.clone(),
                    input: None,
                    output: None,
                })]
            }

            ObservabilityEvent::GenerationStarted {
                trace_id,
                span_id,
                session_id: _,
                iteration,
                model,
                timestamp,
            } => {
                vec![LangfuseIngestionEvent::GenerationCreate(
                    GenerationCreateBody {
                        id: span_id.clone(),
                        trace_id: trace_id.clone(),
                        parent_observation_id: None,
                        name: format!("LLM Call (iteration {})", iteration),
                        start_time: *timestamp,
                        end_time: None,
                        model: model.clone(),
                        model_parameters: None,
                        input: None,
                        output: None,
                        usage: None,
                        metadata: Some(serde_json::json!({
                            "iteration": iteration
                        })),
                        level: None,
                        status_message: None,
                    },
                )]
            }

            ObservabilityEvent::GenerationCompleted {
                trace_id,
                span_id,
                session_id: _,
                iteration,
                model,
                input_tokens,
                output_tokens,
                total_tokens,
                has_tool_calls,
                duration_ms,
                timestamp,
            } => {
                let usage = if input_tokens.is_some()
                    || output_tokens.is_some()
                    || total_tokens.is_some()
                {
                    Some(UsageBody {
                        input: input_tokens.map(|t| t as i64),
                        output: output_tokens.map(|t| t as i64),
                        total: total_tokens.map(|t| t as i64),
                        unit: Some("TOKENS".to_string()),
                    })
                } else {
                    None
                };

                vec![LangfuseIngestionEvent::GenerationUpdate(
                    GenerationUpdateBody {
                        id: span_id.clone(),
                        trace_id: trace_id.clone(),
                        end_time: Some(*timestamp),
                        model: model.clone(),
                        output: None,
                        usage,
                        metadata: Some(serde_json::json!({
                            "iteration": iteration,
                            "has_tool_calls": has_tool_calls,
                            "duration_ms": duration_ms
                        })),
                        level: None,
                        status_message: None,
                    },
                )]
            }

            ObservabilityEvent::TextGenerated { .. } => {
                // Text deltas are too granular for Langfuse; skip them
                // The full output is captured in GenerationCompleted
                vec![]
            }

            ObservabilityEvent::ToolStarted {
                trace_id,
                span_id,
                parent_span_id,
                session_id: _,
                tool_call_id,
                tool_name,
                arguments,
                timestamp,
            } => {
                vec![LangfuseIngestionEvent::SpanCreate(SpanCreateBody {
                    id: span_id.clone(),
                    trace_id: trace_id.clone(),
                    parent_observation_id: Some(parent_span_id.clone()),
                    name: format!("tool:{}", tool_name),
                    start_time: *timestamp,
                    end_time: None,
                    metadata: Some(serde_json::json!({
                        "tool_call_id": tool_call_id,
                        "tool_name": tool_name
                    })),
                    level: None,
                    status_message: None,
                    input: arguments.clone(),
                    output: None,
                })]
            }

            ObservabilityEvent::ToolCompleted {
                trace_id,
                span_id,
                parent_span_id: _,
                session_id: _,
                tool_call_id,
                tool_name,
                success,
                result,
                error,
                duration_ms,
                timestamp,
            } => {
                vec![LangfuseIngestionEvent::SpanUpdate(SpanUpdateBody {
                    id: span_id.clone(),
                    trace_id: trace_id.clone(),
                    end_time: Some(*timestamp),
                    metadata: Some(serde_json::json!({
                        "tool_call_id": tool_call_id,
                        "tool_name": tool_name,
                        "success": success,
                        "duration_ms": duration_ms
                    })),
                    level: Some(if *success {
                        "DEFAULT".to_string()
                    } else {
                        "ERROR".to_string()
                    }),
                    status_message: error.clone(),
                    output: result.clone(),
                })]
            }
        }
    }
}

#[async_trait]
impl ObservabilityBackend for LangfuseBackend {
    fn name(&self) -> &'static str {
        "langfuse"
    }

    fn is_enabled(&self) -> bool {
        true
    }

    async fn record(&self, event: ObservabilityEvent) -> Result<(), ObservabilityError> {
        let langfuse_events = self.convert_event(&event);

        for lf_event in langfuse_events {
            self.add_to_batch(lf_event).await;
        }

        Ok(())
    }

    async fn flush(&self) -> Result<(), ObservabilityError> {
        let items = {
            let mut batch = self.batch.lock().await;
            std::mem::take(&mut *batch)
        };

        if items.is_empty() {
            return Ok(());
        }

        info!(batch_size = items.len(), "Flushing Langfuse batch");
        self.send_batch(items).await
    }

    async fn shutdown(&self) -> Result<(), ObservabilityError> {
        self.flush().await?;
        info!("Langfuse backend shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_config() -> LangfuseConfig {
        LangfuseConfig {
            public_key: "pk-lf-test".to_string(),
            secret_key: "sk-lf-test".to_string(),
            host: "https://cloud.langfuse.com".to_string(),
            release: Some("test-release".to_string()),
            flush_interval_ms: 5000,
            max_batch_size: 10,
        }
    }

    #[test]
    fn test_convert_trace_started() {
        let config = test_config();
        let backend = LangfuseBackend::new(config).unwrap();

        let event = ObservabilityEvent::TraceStarted {
            trace_id: "trace-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: Some("agent-1".to_string()),
            metadata: HashMap::new(),
            timestamp: Utc::now(),
        };

        let lf_events = backend.convert_event(&event);
        assert_eq!(lf_events.len(), 1);

        if let LangfuseIngestionEvent::TraceCreate(body) = &lf_events[0] {
            assert_eq!(body.id, "trace-1");
            assert_eq!(body.session_id, Some("session-1".to_string()));
            assert_eq!(body.user_id, Some("agent-1".to_string()));
        } else {
            panic!("Expected TraceCreate event");
        }
    }

    #[test]
    fn test_convert_generation_events() {
        let config = test_config();
        let backend = LangfuseBackend::new(config).unwrap();

        let start_event = ObservabilityEvent::GenerationStarted {
            trace_id: "trace-1".to_string(),
            span_id: "gen-1".to_string(),
            session_id: "session-1".to_string(),
            iteration: 1,
            model: Some("gpt-4".to_string()),
            timestamp: Utc::now(),
        };

        let start_lf_events = backend.convert_event(&start_event);
        assert_eq!(start_lf_events.len(), 1);

        let complete_event = ObservabilityEvent::GenerationCompleted {
            trace_id: "trace-1".to_string(),
            span_id: "gen-1".to_string(),
            session_id: "session-1".to_string(),
            iteration: 1,
            model: Some("gpt-4".to_string()),
            input_tokens: Some(100),
            output_tokens: Some(50),
            total_tokens: Some(150),
            has_tool_calls: false,
            duration_ms: Some(1234),
            timestamp: Utc::now(),
        };

        let complete_lf_events = backend.convert_event(&complete_event);
        assert_eq!(complete_lf_events.len(), 1);

        if let LangfuseIngestionEvent::GenerationUpdate(body) = &complete_lf_events[0] {
            assert_eq!(body.id, "gen-1");
            assert!(body.usage.is_some());
            let usage = body.usage.as_ref().unwrap();
            assert_eq!(usage.input, Some(100));
            assert_eq!(usage.output, Some(50));
        } else {
            panic!("Expected GenerationUpdate event");
        }
    }

    #[test]
    fn test_convert_tool_events() {
        let config = test_config();
        let backend = LangfuseBackend::new(config).unwrap();

        let start_event = ObservabilityEvent::ToolStarted {
            trace_id: "trace-1".to_string(),
            span_id: "tool-span-1".to_string(),
            parent_span_id: "gen-1".to_string(),
            session_id: "session-1".to_string(),
            tool_call_id: "tool-call-1".to_string(),
            tool_name: "get_weather".to_string(),
            arguments: Some(serde_json::json!({"location": "NYC"})),
            timestamp: Utc::now(),
        };

        let lf_events = backend.convert_event(&start_event);
        assert_eq!(lf_events.len(), 1);

        if let LangfuseIngestionEvent::SpanCreate(body) = &lf_events[0] {
            assert_eq!(body.name, "tool:get_weather");
            assert_eq!(body.parent_observation_id, Some("gen-1".to_string()));
        } else {
            panic!("Expected SpanCreate event");
        }
    }
}
