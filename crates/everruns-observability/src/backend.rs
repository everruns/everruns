// Observability Backend Trait
//
// Defines the interface for observability backends (Langfuse, OpenTelemetry, etc.)
// Each backend receives high-level observability events and translates them
// to provider-specific formats.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// High-level observability events derived from LoopEvents
///
/// These represent the semantic meaning of events for observability purposes,
/// abstracting away the AG-UI protocol details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ObservabilityEvent {
    /// A trace (session/run) has started
    TraceStarted {
        trace_id: String,
        session_id: String,
        agent_id: Option<String>,
        metadata: HashMap<String, serde_json::Value>,
        timestamp: DateTime<Utc>,
    },

    /// A trace has completed
    TraceCompleted {
        trace_id: String,
        session_id: String,
        total_iterations: usize,
        success: bool,
        error: Option<String>,
        metadata: HashMap<String, serde_json::Value>,
        timestamp: DateTime<Utc>,
    },

    /// A generation (LLM call) has started
    GenerationStarted {
        trace_id: String,
        span_id: String,
        session_id: String,
        iteration: usize,
        model: Option<String>,
        timestamp: DateTime<Utc>,
    },

    /// A generation has completed
    GenerationCompleted {
        trace_id: String,
        span_id: String,
        session_id: String,
        iteration: usize,
        model: Option<String>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        total_tokens: Option<u32>,
        has_tool_calls: bool,
        duration_ms: Option<u64>,
        timestamp: DateTime<Utc>,
    },

    /// Text content was generated (streaming delta)
    TextGenerated {
        trace_id: String,
        span_id: String,
        message_id: String,
        delta: String,
        timestamp: DateTime<Utc>,
    },

    /// A tool execution span has started
    ToolStarted {
        trace_id: String,
        span_id: String,
        parent_span_id: String,
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        arguments: Option<serde_json::Value>,
        timestamp: DateTime<Utc>,
    },

    /// A tool execution has completed
    ToolCompleted {
        trace_id: String,
        span_id: String,
        parent_span_id: String,
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        success: bool,
        result: Option<serde_json::Value>,
        error: Option<String>,
        duration_ms: Option<u64>,
        timestamp: DateTime<Utc>,
    },
}

impl ObservabilityEvent {
    /// Get the trace ID for this event
    pub fn trace_id(&self) -> &str {
        match self {
            Self::TraceStarted { trace_id, .. } => trace_id,
            Self::TraceCompleted { trace_id, .. } => trace_id,
            Self::GenerationStarted { trace_id, .. } => trace_id,
            Self::GenerationCompleted { trace_id, .. } => trace_id,
            Self::TextGenerated { trace_id, .. } => trace_id,
            Self::ToolStarted { trace_id, .. } => trace_id,
            Self::ToolCompleted { trace_id, .. } => trace_id,
        }
    }

    /// Get the timestamp for this event
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::TraceStarted { timestamp, .. } => *timestamp,
            Self::TraceCompleted { timestamp, .. } => *timestamp,
            Self::GenerationStarted { timestamp, .. } => *timestamp,
            Self::GenerationCompleted { timestamp, .. } => *timestamp,
            Self::TextGenerated { timestamp, .. } => *timestamp,
            Self::ToolStarted { timestamp, .. } => *timestamp,
            Self::ToolCompleted { timestamp, .. } => *timestamp,
        }
    }
}

/// Trait for observability backends
///
/// Implementations translate ObservabilityEvents to provider-specific formats
/// and send them to the observability platform.
#[async_trait]
pub trait ObservabilityBackend: Send + Sync {
    /// Get the name of this backend (for logging)
    fn name(&self) -> &'static str;

    /// Check if the backend is enabled/configured
    fn is_enabled(&self) -> bool;

    /// Record an observability event
    async fn record(&self, event: ObservabilityEvent) -> Result<(), ObservabilityError>;

    /// Flush any pending events (called on shutdown)
    async fn flush(&self) -> Result<(), ObservabilityError>;

    /// Shutdown the backend gracefully
    async fn shutdown(&self) -> Result<(), ObservabilityError> {
        self.flush().await
    }
}

/// Errors that can occur during observability operations
#[derive(Debug, thiserror::Error)]
pub enum ObservabilityError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Export error: {0}")]
    Export(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Backend not enabled")]
    NotEnabled,
}

/// A no-op backend for when observability is disabled
pub struct NoopBackend;

#[async_trait]
impl ObservabilityBackend for NoopBackend {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn is_enabled(&self) -> bool {
        false
    }

    async fn record(&self, _event: ObservabilityEvent) -> Result<(), ObservabilityError> {
        Ok(())
    }

    async fn flush(&self) -> Result<(), ObservabilityError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_observability_event_trace_id() {
        let event = ObservabilityEvent::TraceStarted {
            trace_id: "test-trace".to_string(),
            session_id: "test-session".to_string(),
            agent_id: None,
            metadata: HashMap::new(),
            timestamp: Utc::now(),
        };

        assert_eq!(event.trace_id(), "test-trace");
    }

    #[tokio::test]
    async fn test_noop_backend() {
        let backend = NoopBackend;
        assert!(!backend.is_enabled());
        assert_eq!(backend.name(), "noop");

        let event = ObservabilityEvent::TraceStarted {
            trace_id: "test".to_string(),
            session_id: "test".to_string(),
            agent_id: None,
            metadata: HashMap::new(),
            timestamp: Utc::now(),
        };

        // Should not error
        backend.record(event).await.unwrap();
        backend.flush().await.unwrap();
    }
}
