// OpenTelemetry Event Listener
//
// This listener generates OTel spans from events following the gen-ai semantic conventions.
// See: https://opentelemetry.io/docs/specs/semconv/gen-ai/
//
// IMPORTANT: This listener is STATELESS. It creates point-in-time spans for each event
// and uses the EventContext span fields (trace_id, span_id, parent_span_id) for
// hierarchical correlation. This design allows the listener to run in any process/container
// without requiring shared state.
//
// Span hierarchy is achieved through:
// - trace_id: Groups all spans in a turn (typically the turn_id)
// - span_id: Unique identifier for each span
// - parent_span_id: Links child spans to parents
//
// Phoenix and other observability backends use these IDs to reconstruct the hierarchy:
// - AGENT span (turn) is the root span
// - LLM spans (generations) are children of the turn
// - TOOL spans (executions) are children of the turn

use async_trait::async_trait;

use crate::event_listeners::EventListener;
use crate::events::{
    Event, EventData, LLM_GENERATION, LlmGenerationData, TOOL_CALL_COMPLETED, TOOL_CALL_STARTED,
    TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_STARTED, ToolCallCompletedData,
    ToolCallStartedData, TurnCancelledData, TurnCompletedData, TurnFailedData, TurnStartedData,
};
use crate::telemetry::{gen_ai, openinference};

// ============================================================================
// OtelEventListener
// ============================================================================

/// Stateless OpenTelemetry event listener that generates gen-ai spans.
///
/// ## Stateless Design
///
/// Unlike traditional span-tracking approaches, this listener is completely stateless.
/// It creates point-in-time spans for each event and relies on the EventContext
/// span fields for hierarchical correlation:
///
/// - `trace_id`: Groups all spans in a turn (typically the turn_id string)
/// - `span_id`: Unique identifier for this span
/// - `parent_span_id`: Links to the parent span for hierarchy
///
/// This design allows the listener to:
/// - Run in any process or container
/// - Handle events arriving out of order
/// - Survive process restarts
/// - Scale horizontally
///
/// ## Span Types
///
/// The listener creates spans for these event types:
///
/// - `turn.started/completed/failed/cancelled` → AGENT span
/// - `llm.generation` → LLM span
/// - `tool.call_started/completed` → TOOL span
///
/// ## Usage
///
/// ```ignore
/// use everruns_core::observation::OtelEventListener;
/// use everruns_core::EventListener;
///
/// let listener = OtelEventListener::new();
/// event_service.add_listener(Arc::new(listener));
/// ```
#[derive(Default)]
pub struct OtelEventListener;

impl OtelEventListener {
    /// Create a new stateless OTel event listener
    pub fn new() -> Self {
        Self
    }

    /// Handle turn.started event - create AGENT span
    fn handle_turn_started(&self, event: &Event, data: &TurnStartedData) {
        let span_name = format!("agent_turn {}", data.turn_id);

        // Extract span context from EventContext
        let trace_id = event
            .context
            .trace_id
            .as_deref()
            .unwrap_or("unknown");
        let span_id = event.context.span_id.as_deref().unwrap_or("");

        let span = tracing::info_span!(
            "agent_turn",
            "otel.name" = %span_name,
            "otel.kind" = "internal",

            // === OpenTelemetry gen-ai semantic conventions ===
            "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
            "gen_ai.conversation.id" = %event.session_id,

            // === OpenInference semantic conventions (Arize Phoenix) ===
            "openinference.span.kind" = openinference::span_kind::AGENT,
            "session.id" = %event.session_id,

            // === Span context (for cross-correlation) ===
            "everruns.trace_id" = %trace_id,
            "everruns.span_id" = %span_id,

            // === Turn context ===
            "turn.id" = %data.turn_id,
            "turn.input_message_id" = %data.input_message_id,
            "turn.status" = "started",
        );

        let _guard = span.enter();
        tracing::debug!(
            turn_id = %data.turn_id,
            session_id = %event.session_id,
            "Turn started"
        );
    }

    /// Handle turn.completed event - create AGENT span with metrics
    fn handle_turn_completed(&self, event: &Event, data: &TurnCompletedData) {
        let span_name = format!("agent_turn {}", data.turn_id);

        // Extract span context from EventContext
        let trace_id = event
            .context
            .trace_id
            .as_deref()
            .unwrap_or("unknown");
        let span_id = event.context.span_id.as_deref().unwrap_or("");

        // Calculate token totals
        let (input_tokens, output_tokens, total_tokens) = data
            .usage
            .as_ref()
            .map(|u| {
                (
                    u.input_tokens,
                    u.output_tokens,
                    u.input_tokens + u.output_tokens,
                )
            })
            .unwrap_or((0, 0, 0));

        let span = tracing::info_span!(
            "agent_turn",
            "otel.name" = %span_name,
            "otel.kind" = "internal",

            // === OpenTelemetry gen-ai semantic conventions ===
            "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
            "gen_ai.conversation.id" = %event.session_id,
            "gen_ai.usage.input_tokens" = input_tokens,
            "gen_ai.usage.output_tokens" = output_tokens,

            // === OpenInference semantic conventions (Arize Phoenix) ===
            "openinference.span.kind" = openinference::span_kind::AGENT,
            "llm.token_count.prompt" = input_tokens,
            "llm.token_count.completion" = output_tokens,
            "llm.token_count.total" = total_tokens,
            "session.id" = %event.session_id,

            // === Span context (for cross-correlation) ===
            "everruns.trace_id" = %trace_id,
            "everruns.span_id" = %span_id,

            // === Turn context ===
            "turn.id" = %data.turn_id,
            "turn.iterations" = data.iterations,
            "turn.status" = "completed",
            "duration_ms" = data.duration_ms.unwrap_or(0),
        );

        let _guard = span.enter();
        tracing::debug!(
            turn_id = %data.turn_id,
            iterations = data.iterations,
            input_tokens = input_tokens,
            output_tokens = output_tokens,
            "Turn completed"
        );
    }

    /// Handle turn.failed event - create AGENT span with error
    fn handle_turn_failed(&self, event: &Event, data: &TurnFailedData) {
        let span_name = format!("agent_turn {}", data.turn_id);

        // Extract span context from EventContext
        let trace_id = event
            .context
            .trace_id
            .as_deref()
            .unwrap_or("unknown");
        let span_id = event.context.span_id.as_deref().unwrap_or("");

        let span = tracing::error_span!(
            "agent_turn",
            "otel.name" = %span_name,
            "otel.kind" = "internal",

            // === OpenTelemetry gen-ai semantic conventions ===
            "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
            "gen_ai.conversation.id" = %event.session_id,

            // === OpenInference semantic conventions (Arize Phoenix) ===
            "openinference.span.kind" = openinference::span_kind::AGENT,
            "session.id" = %event.session_id,

            // === Span context (for cross-correlation) ===
            "everruns.trace_id" = %trace_id,
            "everruns.span_id" = %span_id,

            // === Turn context ===
            "turn.id" = %data.turn_id,
            "turn.status" = "failed",
            "error" = %data.error,
            "error.code" = data.error_code.as_deref().unwrap_or(""),
        );

        let _guard = span.enter();
        tracing::error!(
            turn_id = %data.turn_id,
            error = %data.error,
            error_code = ?data.error_code,
            "Turn failed"
        );
    }

    /// Handle turn.cancelled event - create AGENT span with cancelled status
    fn handle_turn_cancelled(&self, event: &Event, data: &TurnCancelledData) {
        let span_name = format!("agent_turn {}", data.turn_id);

        // Extract span context from EventContext
        let trace_id = event
            .context
            .trace_id
            .as_deref()
            .unwrap_or("unknown");
        let span_id = event.context.span_id.as_deref().unwrap_or("");

        // Calculate token totals if available
        let (input_tokens, output_tokens, total_tokens) = data
            .usage
            .as_ref()
            .map(|u| {
                (
                    u.input_tokens,
                    u.output_tokens,
                    u.input_tokens + u.output_tokens,
                )
            })
            .unwrap_or((0, 0, 0));

        let span = tracing::warn_span!(
            "agent_turn",
            "otel.name" = %span_name,
            "otel.kind" = "internal",

            // === OpenTelemetry gen-ai semantic conventions ===
            "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
            "gen_ai.conversation.id" = %event.session_id,
            "gen_ai.usage.input_tokens" = input_tokens,
            "gen_ai.usage.output_tokens" = output_tokens,

            // === OpenInference semantic conventions (Arize Phoenix) ===
            "openinference.span.kind" = openinference::span_kind::AGENT,
            "llm.token_count.prompt" = input_tokens,
            "llm.token_count.completion" = output_tokens,
            "llm.token_count.total" = total_tokens,
            "session.id" = %event.session_id,

            // === Span context (for cross-correlation) ===
            "everruns.trace_id" = %trace_id,
            "everruns.span_id" = %span_id,

            // === Turn context ===
            "turn.id" = %data.turn_id,
            "turn.status" = "cancelled",
            "cancellation.reason" = data.reason.as_deref().unwrap_or(""),
        );

        let _guard = span.enter();
        tracing::warn!(
            turn_id = %data.turn_id,
            reason = ?data.reason,
            input_tokens = input_tokens,
            output_tokens = output_tokens,
            "Turn cancelled"
        );
    }

    /// Handle llm.generation event - create LLM span
    ///
    /// Emits both OpenTelemetry gen-ai and OpenInference attributes for compatibility
    /// with both standard OTEL backends (Jaeger, Tempo) and Arize Phoenix.
    fn handle_llm_generation(&self, event: &Event, data: &LlmGenerationData) {
        let model = &data.metadata.model;
        let provider = data.metadata.provider.as_deref().unwrap_or("unknown");

        // Determine output type based on response content
        let output_type = if !data.output.tool_calls.is_empty() {
            "tool_calls"
        } else {
            gen_ai::output_type::TEXT
        };

        // Calculate token totals
        let input_tokens = data
            .metadata
            .usage
            .as_ref()
            .map(|u| u.input_tokens)
            .unwrap_or(0);
        let output_tokens = data
            .metadata
            .usage
            .as_ref()
            .map(|u| u.output_tokens)
            .unwrap_or(0);
        let total_tokens = input_tokens + output_tokens;

        // Extract span context from EventContext
        let trace_id = event.context.trace_id.as_deref().unwrap_or("");
        let span_id = event.context.span_id.as_deref().unwrap_or("");
        let parent_span_id = event.context.parent_span_id.as_deref().unwrap_or("");

        let span_name = format!("chat {}", model);

        let span = tracing::info_span!(
            "llm_generation",
            "otel.name" = %span_name,
            "otel.kind" = "client",

            // === OpenTelemetry gen-ai semantic conventions ===
            "gen_ai.operation.name" = gen_ai::operation::CHAT,
            "gen_ai.system" = %provider,
            "gen_ai.request.model" = %model,
            "gen_ai.response.model" = %model,
            "gen_ai.response.id" = data.metadata.response_id.as_deref().unwrap_or(""),
            "gen_ai.response.finish_reasons" = ?data.metadata.finish_reasons,
            "gen_ai.usage.input_tokens" = input_tokens,
            "gen_ai.usage.output_tokens" = output_tokens,
            "gen_ai.output.type" = %output_type,
            "gen_ai.conversation.id" = %event.session_id,

            // === OpenInference semantic conventions (Arize Phoenix) ===
            "openinference.span.kind" = openinference::span_kind::LLM,
            "llm.model_name" = %model,
            "llm.system" = %provider,
            "llm.token_count.prompt" = input_tokens,
            "llm.token_count.completion" = output_tokens,
            "llm.token_count.total" = total_tokens,
            "session.id" = %event.session_id,

            // === Span context (for cross-correlation) ===
            "everruns.trace_id" = %trace_id,
            "everruns.span_id" = %span_id,
            "everruns.parent_span_id" = %parent_span_id,

            // === Custom attributes ===
            "duration_ms" = data.metadata.duration_ms.unwrap_or(0),
            "time_to_first_token_ms" = data.metadata.time_to_first_token_ms.unwrap_or(0),
            "success" = data.metadata.success,
        );

        let _guard = span.enter();
        tracing::debug!(
            model = %model,
            provider = %provider,
            success = %data.metadata.success,
            input_tokens = input_tokens,
            output_tokens = output_tokens,
            tool_calls = %data.output.tool_calls.len(),
            "LLM generation completed"
        );
    }

    /// Handle tool.call_started event - create TOOL span
    fn handle_tool_call_started(&self, event: &Event, data: &ToolCallStartedData) {
        let tool_name = &data.tool_call.name;
        let tool_call_id = &data.tool_call.id;

        // Extract span context from EventContext
        let trace_id = event.context.trace_id.as_deref().unwrap_or("");
        let span_id = event.context.span_id.as_deref().unwrap_or("");
        let parent_span_id = event.context.parent_span_id.as_deref().unwrap_or("");

        let span_name = format!("tool {}", tool_name);

        let span = tracing::info_span!(
            "tool_execution",
            "otel.name" = %span_name,
            "otel.kind" = "internal",

            // === OpenTelemetry gen-ai semantic conventions ===
            "gen_ai.operation.name" = gen_ai::operation::EXECUTE_TOOL,
            "gen_ai.tool.name" = %tool_name,
            "gen_ai.tool.type" = gen_ai::tool_type::FUNCTION,
            "gen_ai.tool.call.id" = %tool_call_id,
            "gen_ai.conversation.id" = %event.session_id,

            // === OpenInference semantic conventions (Arize Phoenix) ===
            "openinference.span.kind" = openinference::span_kind::TOOL,
            "tool.name" = %tool_name,
            "session.id" = %event.session_id,

            // === Span context (for cross-correlation) ===
            "everruns.trace_id" = %trace_id,
            "everruns.span_id" = %span_id,
            "everruns.parent_span_id" = %parent_span_id,

            // === Tool context ===
            "tool.status" = "started",
        );

        let _guard = span.enter();
        tracing::debug!(
            tool_name = %tool_name,
            tool_call_id = %tool_call_id,
            "Tool execution started"
        );
    }

    /// Handle tool.call_completed event - create TOOL span with result
    fn handle_tool_call_completed(&self, event: &Event, data: &ToolCallCompletedData) {
        // Extract span context from EventContext
        let trace_id = event.context.trace_id.as_deref().unwrap_or("");
        let span_id = event.context.span_id.as_deref().unwrap_or("");
        let parent_span_id = event.context.parent_span_id.as_deref().unwrap_or("");

        let span_name = format!("tool {}", data.tool_name);

        let span = tracing::info_span!(
            "tool_execution",
            "otel.name" = %span_name,
            "otel.kind" = "internal",

            // === OpenTelemetry gen-ai semantic conventions ===
            "gen_ai.operation.name" = gen_ai::operation::EXECUTE_TOOL,
            "gen_ai.tool.name" = %data.tool_name,
            "gen_ai.tool.type" = gen_ai::tool_type::FUNCTION,
            "gen_ai.tool.call.id" = %data.tool_call_id,
            "gen_ai.conversation.id" = %event.session_id,

            // === OpenInference semantic conventions (Arize Phoenix) ===
            "openinference.span.kind" = openinference::span_kind::TOOL,
            "tool.name" = %data.tool_name,
            "tool.success" = %data.success,
            "tool.status" = %data.status,
            "session.id" = %event.session_id,

            // === Span context (for cross-correlation) ===
            "everruns.trace_id" = %trace_id,
            "everruns.span_id" = %span_id,
            "everruns.parent_span_id" = %parent_span_id,
        );

        let _guard = span.enter();

        if data.success {
            tracing::debug!(
                tool_name = %data.tool_name,
                tool_call_id = %data.tool_call_id,
                "Tool execution succeeded"
            );
        } else {
            tracing::warn!(
                tool_name = %data.tool_name,
                tool_call_id = %data.tool_call_id,
                error = ?data.error,
                "Tool execution failed"
            );
        }
    }
}

#[async_trait]
impl EventListener for OtelEventListener {
    async fn on_event(&self, event: &Event) {
        match &event.data {
            EventData::TurnStarted(data) => {
                self.handle_turn_started(event, data);
            }
            EventData::TurnCompleted(data) => {
                self.handle_turn_completed(event, data);
            }
            EventData::TurnFailed(data) => {
                self.handle_turn_failed(event, data);
            }
            EventData::TurnCancelled(data) => {
                self.handle_turn_cancelled(event, data);
            }
            EventData::LlmGeneration(data) => {
                self.handle_llm_generation(event, data);
            }
            EventData::ToolCallStarted(data) => {
                self.handle_tool_call_started(event, data);
            }
            EventData::ToolCallCompleted(data) => {
                self.handle_tool_call_completed(event, data);
            }
            // Other events don't generate spans
            _ => {}
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        // Listen to events needed for spans
        Some(vec![
            TURN_STARTED,
            TURN_COMPLETED,
            TURN_FAILED,
            TURN_CANCELLED,
            LLM_GENERATION,
            TOOL_CALL_STARTED,
            TOOL_CALL_COMPLETED,
        ])
    }

    fn name(&self) -> &'static str {
        "OtelEventListener"
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{
        EventContext, LlmGenerationMetadata, LlmGenerationOutput, TokenUsage, TurnCancelledData,
        TurnFailedData,
    };
    use crate::message::Message;
    use crate::tool_types::ToolCall;
    use crate::typed_id::{MessageId, TurnId};
    use serde_json::json;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_otel_listener_creation() {
        let listener = OtelEventListener::new();
        assert_eq!(listener.name(), "OtelEventListener");
    }

    #[tokio::test]
    async fn test_otel_listener_is_stateless() {
        // Verify the listener has no state (unit struct proves this)
        let _listener1 = OtelEventListener;
        let _listener2 = OtelEventListener::new();
        // Both are identical - no shared state
    }

    #[tokio::test]
    async fn test_otel_listener_event_types() {
        let listener = OtelEventListener::new();
        let types = listener.event_types().unwrap();
        assert_eq!(types.len(), 7);
        assert!(types.contains(&TURN_STARTED));
        assert!(types.contains(&TURN_COMPLETED));
        assert!(types.contains(&TURN_FAILED));
        assert!(types.contains(&TURN_CANCELLED));
        assert!(types.contains(&LLM_GENERATION));
        assert!(types.contains(&TOOL_CALL_STARTED));
        assert!(types.contains(&TOOL_CALL_COMPLETED));
    }

    #[tokio::test]
    async fn test_turn_lifecycle() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();
        let turn_id = TurnId::from_uuid(Uuid::now_v7());

        // Start turn - creates span
        let started_data = TurnStartedData {
            turn_id,
            input_message_id: MessageId::from_uuid(Uuid::now_v7()),
        };
        let start_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnStarted(started_data),
        );
        listener.on_event(&start_event).await;

        // Complete turn - creates another span (stateless, no tracking)
        let completed_data = TurnCompletedData {
            turn_id,
            iterations: 2,
            duration_ms: Some(500),
            usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: None,
                cache_creation_tokens: None,
            }),
        };
        let complete_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnCompleted(completed_data),
        );
        listener.on_event(&complete_event).await;
        // No assertion needed - just verify no panic
    }

    #[tokio::test]
    async fn test_llm_generation() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();

        let llm_data = LlmGenerationData {
            messages: vec![Message::user("Hello")],
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
            },
        };

        // Create event with span context
        let context = EventContext::empty().with_span(
            "trace_123".to_string(),
            "span_456".to_string(),
            Some("parent_789".to_string()),
        );

        let llm_event = Event::new(session_id, context, EventData::LlmGeneration(llm_data));
        listener.on_event(&llm_event).await;
        // No assertion needed - just verify no panic
    }

    #[tokio::test]
    async fn test_tool_call_lifecycle() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();

        // Start tool call
        let tool_started = ToolCallStartedData {
            tool_call: ToolCall {
                id: "call_123".to_string(),
                name: "get_weather".to_string(),
                arguments: json!({"city": "Tokyo"}),
            },
        };
        let tool_start_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::ToolCallStarted(tool_started),
        );
        listener.on_event(&tool_start_event).await;

        // Complete tool call
        let tool_completed = ToolCallCompletedData {
            tool_call_id: "call_123".to_string(),
            tool_name: "get_weather".to_string(),
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
        };
        let tool_complete_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::ToolCallCompleted(tool_completed),
        );
        listener.on_event(&tool_complete_event).await;
        // No assertion needed - just verify no panic
    }

    #[tokio::test]
    async fn test_turn_failed() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();
        let turn_id = TurnId::from_uuid(Uuid::now_v7());

        let failed_data = TurnFailedData {
            turn_id,
            error: "LLM rate limit exceeded".to_string(),
            error_code: Some("rate_limit".to_string()),
        };
        let fail_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnFailed(failed_data),
        );
        listener.on_event(&fail_event).await;
        // No assertion needed - just verify no panic
    }

    #[tokio::test]
    async fn test_turn_cancelled() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();
        let turn_id = TurnId::from_uuid(Uuid::now_v7());

        let cancelled_data = TurnCancelledData {
            turn_id,
            reason: Some("User cancelled".to_string()),
            usage: Some(TokenUsage {
                input_tokens: 50,
                output_tokens: 0,
                cache_read_tokens: None,
                cache_creation_tokens: None,
            }),
        };
        let cancel_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnCancelled(cancelled_data),
        );
        listener.on_event(&cancel_event).await;
        // No assertion needed - just verify no panic
    }

    #[tokio::test]
    async fn test_full_agentic_loop() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();
        let turn_id = TurnId::from_uuid(Uuid::now_v7());
        let trace_id = turn_id.to_string();

        // 1. Turn starts
        let mut ctx = EventContext::empty();
        ctx.trace_id = Some(trace_id.clone());
        ctx.span_id = Some(turn_id.to_string());
        listener
            .on_event(&Event::new(
                session_id,
                ctx.clone(),
                EventData::TurnStarted(TurnStartedData {
                    turn_id,
                    input_message_id: MessageId::from_uuid(Uuid::now_v7()),
                }),
            ))
            .await;

        // 2. First LLM generation (decides to call tool)
        let mut llm_ctx = EventContext::empty();
        llm_ctx.trace_id = Some(trace_id.clone());
        llm_ctx.span_id = Some(Uuid::now_v7().to_string());
        llm_ctx.parent_span_id = Some(turn_id.to_string());
        listener
            .on_event(&Event::new(
                session_id,
                llm_ctx,
                EventData::LlmGeneration(LlmGenerationData {
                    messages: vec![],
                    tools: vec![],
                    output: LlmGenerationOutput {
                        text: Some("Let me check...".to_string()),
                        tool_calls: vec![ToolCall {
                            id: "call_1".to_string(),
                            name: "search".to_string(),
                            arguments: json!({}),
                        }],
                    },
                    metadata: LlmGenerationMetadata {
                        model: "gpt-4".to_string(),
                        provider: Some("openai".to_string()),
                        usage: Some(TokenUsage {
                            input_tokens: 50,
                            output_tokens: 20,
                            cache_read_tokens: None,
                            cache_creation_tokens: None,
                        }),
                        duration_ms: Some(100),
                        time_to_first_token_ms: Some(30),
                        success: true,
                        error: None,
                        finish_reasons: Some(vec!["tool_calls".to_string()]),
                        response_id: None,
                    },
                }),
            ))
            .await;

        // 3. Tool execution
        let mut tool_ctx = EventContext::empty();
        tool_ctx.trace_id = Some(trace_id.clone());
        tool_ctx.span_id = Some(Uuid::now_v7().to_string());
        tool_ctx.parent_span_id = Some(turn_id.to_string());
        listener
            .on_event(&Event::new(
                session_id,
                tool_ctx.clone(),
                EventData::ToolCallStarted(ToolCallStartedData {
                    tool_call: ToolCall {
                        id: "call_1".to_string(),
                        name: "search".to_string(),
                        arguments: json!({}),
                    },
                }),
            ))
            .await;

        listener
            .on_event(&Event::new(
                session_id,
                tool_ctx,
                EventData::ToolCallCompleted(ToolCallCompletedData {
                    tool_call_id: "call_1".to_string(),
                    tool_name: "search".to_string(),
                    success: true,
                    status: "success".to_string(),
                    result: None,
                    error: None,
                }),
            ))
            .await;

        // 4. Second LLM generation (final response)
        let mut llm2_ctx = EventContext::empty();
        llm2_ctx.trace_id = Some(trace_id.clone());
        llm2_ctx.span_id = Some(Uuid::now_v7().to_string());
        llm2_ctx.parent_span_id = Some(turn_id.to_string());
        listener
            .on_event(&Event::new(
                session_id,
                llm2_ctx,
                EventData::LlmGeneration(LlmGenerationData {
                    messages: vec![],
                    tools: vec![],
                    output: LlmGenerationOutput {
                        text: Some("Here's what I found...".to_string()),
                        tool_calls: vec![],
                    },
                    metadata: LlmGenerationMetadata {
                        model: "gpt-4".to_string(),
                        provider: Some("openai".to_string()),
                        usage: Some(TokenUsage {
                            input_tokens: 100,
                            output_tokens: 80,
                            cache_read_tokens: None,
                            cache_creation_tokens: None,
                        }),
                        duration_ms: Some(200),
                        time_to_first_token_ms: Some(40),
                        success: true,
                        error: None,
                        finish_reasons: Some(vec!["stop".to_string()]),
                        response_id: None,
                    },
                }),
            ))
            .await;

        // 5. Turn completes
        let mut complete_ctx = EventContext::empty();
        complete_ctx.trace_id = Some(trace_id);
        complete_ctx.span_id = Some(turn_id.to_string());
        listener
            .on_event(&Event::new(
                session_id,
                complete_ctx,
                EventData::TurnCompleted(TurnCompletedData {
                    turn_id,
                    iterations: 2,
                    duration_ms: Some(500),
                    usage: Some(TokenUsage {
                        input_tokens: 150,
                        output_tokens: 100,
                        cache_read_tokens: None,
                        cache_creation_tokens: None,
                    }),
                }),
            ))
            .await;

        // Stateless - no assertions about state needed
    }
}
