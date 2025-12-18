// Observable Event Emitter
//
// Wraps an existing EventEmitter to add observability without modifying
// the agent-loop. Converts LoopEvents to ObservabilityEvents and forwards
// them to configured backends.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_agent_loop::{traits::EventEmitter, LoopEvent, Result};
use tokio::sync::RwLock;
use tracing::{error, warn};
use uuid::Uuid;

use crate::backend::{ObservabilityBackend, ObservabilityEvent};

/// State tracked for active traces/spans
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used for future trace context enrichment
struct TraceState {
    trace_id: String,
    session_id: String,
    agent_id: Option<String>,
    started_at: DateTime<Utc>,
    current_iteration: usize,
    current_llm_span_id: Option<String>,
    llm_span_started: Option<Instant>,
    active_tool_spans: HashMap<String, ToolSpanState>,
}

#[derive(Debug, Clone)]
struct ToolSpanState {
    span_id: String,
    parent_span_id: String,
    tool_name: String,
    started_at: Instant,
}

/// Event emitter wrapper that adds observability
///
/// Wraps an existing EventEmitter implementation and forwards events
/// to observability backends while also delegating to the inner emitter.
pub struct ObservableEventEmitter<E: EventEmitter> {
    /// The wrapped event emitter
    inner: Arc<E>,
    /// Observability backends
    backends: Vec<Arc<dyn ObservabilityBackend>>,
    /// Active trace states (keyed by session_id)
    traces: Arc<RwLock<HashMap<String, TraceState>>>,
    /// Optional agent_id for context
    agent_id: Option<String>,
}

impl<E: EventEmitter> ObservableEventEmitter<E> {
    /// Create a new observable emitter wrapping an existing emitter
    pub fn new(inner: E, backends: Vec<Arc<dyn ObservabilityBackend>>) -> Self {
        Self {
            inner: Arc::new(inner),
            backends,
            traces: Arc::new(RwLock::new(HashMap::new())),
            agent_id: None,
        }
    }

    /// Create with an Arc-wrapped inner emitter
    pub fn with_arc(inner: Arc<E>, backends: Vec<Arc<dyn ObservabilityBackend>>) -> Self {
        Self {
            inner,
            backends,
            traces: Arc::new(RwLock::new(HashMap::new())),
            agent_id: None,
        }
    }

    /// Set the agent_id for trace context
    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    /// Get the inner emitter
    pub fn inner(&self) -> &Arc<E> {
        &self.inner
    }

    /// Convert a LoopEvent to ObservabilityEvents and record them
    async fn process_for_observability(&self, event: &LoopEvent) {
        let obs_events = self.convert_event(event).await;

        for obs_event in obs_events {
            for backend in &self.backends {
                if backend.is_enabled() {
                    if let Err(e) = backend.record(obs_event.clone()).await {
                        warn!(
                            backend = backend.name(),
                            error = %e,
                            "Failed to record observability event"
                        );
                    }
                }
            }
        }
    }

    /// Convert a LoopEvent to zero or more ObservabilityEvents
    async fn convert_event(&self, event: &LoopEvent) -> Vec<ObservabilityEvent> {
        match event {
            LoopEvent::LoopStarted {
                session_id,
                timestamp,
            } => {
                let trace_id = Uuid::now_v7().to_string();
                let state = TraceState {
                    trace_id: trace_id.clone(),
                    session_id: session_id.clone(),
                    agent_id: self.agent_id.clone(),
                    started_at: *timestamp,
                    current_iteration: 0,
                    current_llm_span_id: None,
                    llm_span_started: None,
                    active_tool_spans: HashMap::new(),
                };

                self.traces.write().await.insert(session_id.clone(), state);

                vec![ObservabilityEvent::TraceStarted {
                    trace_id,
                    session_id: session_id.clone(),
                    agent_id: self.agent_id.clone(),
                    metadata: HashMap::new(),
                    timestamp: *timestamp,
                }]
            }

            LoopEvent::LoopCompleted {
                session_id,
                total_iterations,
                timestamp,
            } => {
                let trace_id = {
                    let traces = self.traces.read().await;
                    traces.get(session_id).map(|s| s.trace_id.clone())
                };

                if let Some(trace_id) = trace_id {
                    // Remove the trace state
                    self.traces.write().await.remove(session_id);

                    vec![ObservabilityEvent::TraceCompleted {
                        trace_id,
                        session_id: session_id.clone(),
                        total_iterations: *total_iterations,
                        success: true,
                        error: None,
                        metadata: HashMap::new(),
                        timestamp: *timestamp,
                    }]
                } else {
                    vec![]
                }
            }

            LoopEvent::LoopError {
                session_id,
                error,
                timestamp,
            } => {
                let trace_info = {
                    let traces = self.traces.read().await;
                    traces
                        .get(session_id)
                        .map(|s| (s.trace_id.clone(), s.current_iteration))
                };

                if let Some((trace_id, iterations)) = trace_info {
                    self.traces.write().await.remove(session_id);

                    vec![ObservabilityEvent::TraceCompleted {
                        trace_id,
                        session_id: session_id.clone(),
                        total_iterations: iterations,
                        success: false,
                        error: Some(error.clone()),
                        metadata: HashMap::new(),
                        timestamp: *timestamp,
                    }]
                } else {
                    vec![]
                }
            }

            LoopEvent::IterationStarted {
                session_id,
                iteration,
                ..
            } => {
                let mut traces = self.traces.write().await;
                if let Some(state) = traces.get_mut(session_id) {
                    state.current_iteration = *iteration;
                }
                vec![]
            }

            LoopEvent::LlmCallStarted {
                session_id,
                iteration,
                timestamp,
            } => {
                let span_id = Uuid::now_v7().to_string();
                let trace_id = {
                    let mut traces = self.traces.write().await;
                    if let Some(state) = traces.get_mut(session_id) {
                        state.current_llm_span_id = Some(span_id.clone());
                        state.llm_span_started = Some(Instant::now());
                        Some(state.trace_id.clone())
                    } else {
                        None
                    }
                };

                if let Some(trace_id) = trace_id {
                    vec![ObservabilityEvent::GenerationStarted {
                        trace_id,
                        span_id,
                        session_id: session_id.clone(),
                        iteration: *iteration,
                        model: None, // Model info not available at start
                        timestamp: *timestamp,
                    }]
                } else {
                    vec![]
                }
            }

            LoopEvent::LlmCallCompleted {
                session_id,
                iteration,
                has_tool_calls,
                timestamp,
            } => {
                let span_info = {
                    let mut traces = self.traces.write().await;
                    if let Some(state) = traces.get_mut(session_id) {
                        let info = state.current_llm_span_id.take().map(|span_id| {
                            let duration = state
                                .llm_span_started
                                .map(|s| s.elapsed().as_millis() as u64);
                            (state.trace_id.clone(), span_id, duration)
                        });
                        state.llm_span_started = None;
                        info
                    } else {
                        None
                    }
                };

                if let Some((trace_id, span_id, duration_ms)) = span_info {
                    vec![ObservabilityEvent::GenerationCompleted {
                        trace_id,
                        span_id,
                        session_id: session_id.clone(),
                        iteration: *iteration,
                        model: None, // TODO: Get from LLM metadata
                        input_tokens: None,
                        output_tokens: None,
                        total_tokens: None,
                        has_tool_calls: *has_tool_calls,
                        duration_ms,
                        timestamp: *timestamp,
                    }]
                } else {
                    vec![]
                }
            }

            LoopEvent::TextDelta {
                session_id,
                message_id,
                delta,
                timestamp,
            } => {
                let span_info = {
                    let traces = self.traces.read().await;
                    traces.get(session_id).and_then(|state| {
                        state
                            .current_llm_span_id
                            .as_ref()
                            .map(|span_id| (state.trace_id.clone(), span_id.clone()))
                    })
                };

                if let Some((trace_id, span_id)) = span_info {
                    vec![ObservabilityEvent::TextGenerated {
                        trace_id,
                        span_id,
                        message_id: message_id.clone(),
                        delta: delta.clone(),
                        timestamp: *timestamp,
                    }]
                } else {
                    vec![]
                }
            }

            LoopEvent::ToolExecutionStarted {
                session_id,
                tool_call_id,
                tool_name,
                timestamp,
            } => {
                let span_id = Uuid::now_v7().to_string();
                let span_info = {
                    let mut traces = self.traces.write().await;
                    if let Some(state) = traces.get_mut(session_id) {
                        let parent_span_id = state
                            .current_llm_span_id
                            .clone()
                            .unwrap_or_else(|| state.trace_id.clone());

                        state.active_tool_spans.insert(
                            tool_call_id.clone(),
                            ToolSpanState {
                                span_id: span_id.clone(),
                                parent_span_id: parent_span_id.clone(),
                                tool_name: tool_name.clone(),
                                started_at: Instant::now(),
                            },
                        );

                        Some((state.trace_id.clone(), span_id, parent_span_id))
                    } else {
                        None
                    }
                };

                if let Some((trace_id, span_id, parent_span_id)) = span_info {
                    vec![ObservabilityEvent::ToolStarted {
                        trace_id,
                        span_id,
                        parent_span_id,
                        session_id: session_id.clone(),
                        tool_call_id: tool_call_id.clone(),
                        tool_name: tool_name.clone(),
                        arguments: None, // Arguments captured via AG-UI events
                        timestamp: *timestamp,
                    }]
                } else {
                    vec![]
                }
            }

            LoopEvent::ToolExecutionCompleted {
                session_id,
                tool_call_id,
                success,
                timestamp,
            } => {
                let span_info = {
                    let mut traces = self.traces.write().await;
                    if let Some(state) = traces.get_mut(session_id) {
                        state
                            .active_tool_spans
                            .remove(tool_call_id)
                            .map(|tool_state| {
                                let duration_ms =
                                    tool_state.started_at.elapsed().as_millis() as u64;
                                (
                                    state.trace_id.clone(),
                                    tool_state.span_id,
                                    tool_state.parent_span_id,
                                    tool_state.tool_name,
                                    duration_ms,
                                )
                            })
                    } else {
                        None
                    }
                };

                if let Some((trace_id, span_id, parent_span_id, tool_name, duration_ms)) = span_info
                {
                    vec![ObservabilityEvent::ToolCompleted {
                        trace_id,
                        span_id,
                        parent_span_id,
                        session_id: session_id.clone(),
                        tool_call_id: tool_call_id.clone(),
                        tool_name,
                        success: *success,
                        result: None,
                        error: if *success {
                            None
                        } else {
                            Some("Tool execution failed".to_string())
                        },
                        duration_ms: Some(duration_ms),
                        timestamp: *timestamp,
                    }]
                } else {
                    vec![]
                }
            }

            // AG-UI events are handled for SSE streaming, not observability
            LoopEvent::AgUi(_) => vec![],

            // Iteration completed doesn't need separate observability event
            LoopEvent::IterationCompleted { .. } => vec![],
        }
    }

    /// Flush all backends
    pub async fn flush(&self) {
        for backend in &self.backends {
            if let Err(e) = backend.flush().await {
                error!(backend = backend.name(), error = %e, "Failed to flush backend");
            }
        }
    }

    /// Shutdown all backends
    pub async fn shutdown(&self) {
        for backend in &self.backends {
            if let Err(e) = backend.shutdown().await {
                error!(backend = backend.name(), error = %e, "Failed to shutdown backend");
            }
        }
    }
}

#[async_trait]
impl<E: EventEmitter> EventEmitter for ObservableEventEmitter<E> {
    async fn emit(&self, event: LoopEvent) -> Result<()> {
        // First, delegate to the inner emitter
        self.inner.emit(event.clone()).await?;

        // Then process for observability (non-blocking, best-effort)
        self.process_for_observability(&event).await;

        Ok(())
    }

    async fn emit_batch(&self, events: Vec<LoopEvent>) -> Result<()> {
        // Delegate batch to inner emitter
        self.inner.emit_batch(events.clone()).await?;

        // Process each for observability
        for event in events {
            self.process_for_observability(&event).await;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::NoopBackend;
    use everruns_agent_loop::memory::InMemoryEventEmitter;

    #[tokio::test]
    async fn test_observable_emitter_delegates_to_inner() {
        let inner = InMemoryEventEmitter::new();
        let backends: Vec<Arc<dyn ObservabilityBackend>> = vec![Arc::new(NoopBackend)];
        let emitter = ObservableEventEmitter::new(inner, backends);

        let event = LoopEvent::loop_started("test-session");
        emitter.emit(event).await.unwrap();

        // Inner emitter should have received the event
        let events = emitter.inner().events().await;
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_observable_emitter_tracks_traces() {
        let inner = InMemoryEventEmitter::new();
        let backends: Vec<Arc<dyn ObservabilityBackend>> = vec![Arc::new(NoopBackend)];
        let emitter = ObservableEventEmitter::new(inner, backends);

        // Start a loop
        emitter
            .emit(LoopEvent::loop_started("session-1"))
            .await
            .unwrap();

        // Should have trace state
        {
            let traces = emitter.traces.read().await;
            assert!(traces.contains_key("session-1"));
        }

        // Complete the loop
        emitter
            .emit(LoopEvent::loop_completed("session-1", 1))
            .await
            .unwrap();

        // Trace state should be cleaned up
        {
            let traces = emitter.traces.read().await;
            assert!(!traces.contains_key("session-1"));
        }
    }
}
