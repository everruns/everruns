// OpenTelemetry Event Listener
//
// This listener generates OTel spans from events following the gen-ai semantic conventions.
// See: https://opentelemetry.io/docs/specs/semconv/gen-ai/
//
// The listener reacts to events and creates spans:
// - llm.generation → gen_ai.chat span with model, tokens, messages
// - tool.call_started/completed → gen_ai.execute_tool span
// - turn.started/completed → gen_ai.invoke_agent span
// - reason.started/completed → nested chat spans within invoke_agent
//
// This approach decouples OTel instrumentation from business logic,
// making it easier to support other observability backends.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::event_listeners::EventListener;
use crate::events::{
    Event, EventData, LlmGenerationData, ToolCallCompletedData, ToolCallStartedData,
    TurnCompletedData, TurnStartedData, LLM_GENERATION, TOOL_CALL_COMPLETED, TOOL_CALL_STARTED,
    TURN_COMPLETED, TURN_STARTED,
};
use crate::telemetry::gen_ai;

// ============================================================================
// Internal Span Tracking
// ============================================================================

/// Info tracked for in-flight tool calls
#[allow(dead_code)]
struct ToolCallSpanInfo {
    tool_name: String,
    started_at: std::time::Instant,
}

/// Info tracked for in-flight turns
#[allow(dead_code)]
struct TurnSpanInfo {
    session_id: Uuid,
    agent_id: Option<Uuid>,
    started_at: std::time::Instant,
}

// ============================================================================
// OtelEventListener
// ============================================================================

/// OpenTelemetry event listener that generates gen-ai semantic convention spans.
///
/// This listener creates spans from events:
/// - `llm.generation` → `chat {model}` span
/// - `tool.call_started/completed` → `execute_tool {name}` span
/// - `turn.started/completed` → `invoke_agent {agent_id}` span
///
/// Spans are created synchronously when events are received.
///
/// # Example
///
/// ```ignore
/// use everruns_core::observation::OtelEventListener;
/// use everruns_core::EventListener;
///
/// let listener = OtelEventListener::new();
///
/// // Register with event service
/// event_service.add_listener(Arc::new(listener));
/// ```
pub struct OtelEventListener {
    /// Track in-flight tool calls for span correlation
    /// Key: tool_call_id, Value: span handle info
    tool_call_spans: Mutex<HashMap<String, ToolCallSpanInfo>>,

    /// Track in-flight turns for span correlation
    /// Key: turn_id, Value: span handle info
    turn_spans: Mutex<HashMap<Uuid, TurnSpanInfo>>,
}

impl Default for OtelEventListener {
    fn default() -> Self {
        Self::new()
    }
}

impl OtelEventListener {
    /// Create a new OTel event listener
    pub fn new() -> Self {
        Self {
            tool_call_spans: Mutex::new(HashMap::new()),
            turn_spans: Mutex::new(HashMap::new()),
        }
    }

    /// Handle llm.generation event - create a chat span
    fn handle_llm_generation(&self, event: &Event, data: &LlmGenerationData) {
        let model = &data.metadata.model;
        let provider = data.metadata.provider.as_deref().unwrap_or("unknown");

        // Determine output type based on response content
        let output_type = if !data.output.tool_calls.is_empty() {
            "tool_calls"
        } else {
            gen_ai::output_type::TEXT
        };

        // Create span with gen-ai semantic conventions
        let span_name = format!("chat {}", model);
        let span = tracing::info_span!(
            "gen_ai.chat",
            "otel.name" = %span_name,
            "otel.kind" = "client",
            // Operation and provider
            "gen_ai.operation.name" = gen_ai::operation::CHAT,
            "gen_ai.system" = %provider,
            // Request attributes
            "gen_ai.request.model" = %model,
            // Response attributes
            "gen_ai.response.model" = %model,
            "gen_ai.response.id" = data.metadata.response_id.as_deref().unwrap_or(""),
            "gen_ai.response.finish_reasons" = ?data.metadata.finish_reasons,
            // Usage metrics
            "gen_ai.usage.input_tokens" = data.metadata.usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
            "gen_ai.usage.output_tokens" = data.metadata.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
            // Output type
            "gen_ai.output.type" = %output_type,
            // Conversation context
            "gen_ai.conversation.id" = %event.session_id,
            // Duration
            "duration_ms" = data.metadata.duration_ms.unwrap_or(0),
        );

        // Enter and immediately exit the span (event is a point-in-time record)
        let _guard = span.enter();

        // Log event details at debug level within the span
        tracing::debug!(
            model = %model,
            provider = %provider,
            success = %data.metadata.success,
            input_tokens = data.metadata.usage.as_ref().map(|u| u.input_tokens),
            output_tokens = data.metadata.usage.as_ref().map(|u| u.output_tokens),
            tool_calls = %data.output.tool_calls.len(),
            "LLM generation completed"
        );
    }

    /// Handle tool.call_started event - record start time for duration calculation
    fn handle_tool_call_started(&self, _event: &Event, data: &ToolCallStartedData) {
        let mut spans = self.tool_call_spans.lock().unwrap();
        spans.insert(
            data.tool_call.id.clone(),
            ToolCallSpanInfo {
                tool_name: data.tool_call.name.clone(),
                started_at: std::time::Instant::now(),
            },
        );
    }

    /// Handle tool.call_completed event - create execute_tool span
    fn handle_tool_call_completed(&self, event: &Event, data: &ToolCallCompletedData) {
        // Get start info if available
        let start_info = {
            let mut spans = self.tool_call_spans.lock().unwrap();
            spans.remove(&data.tool_call_id)
        };

        let duration_ms = start_info
            .as_ref()
            .map(|info| info.started_at.elapsed().as_millis() as u64);

        // Create span with gen-ai semantic conventions
        let span_name = format!("execute_tool {}", data.tool_name);
        let span = tracing::info_span!(
            "gen_ai.execute_tool",
            "otel.name" = %span_name,
            "otel.kind" = "internal",
            // Operation
            "gen_ai.operation.name" = gen_ai::operation::EXECUTE_TOOL,
            // Tool attributes
            "gen_ai.tool.name" = %data.tool_name,
            "gen_ai.tool.type" = gen_ai::tool_type::FUNCTION,
            "gen_ai.tool.call.id" = %data.tool_call_id,
            // Result
            "tool.success" = %data.success,
            "tool.status" = %data.status,
            // Conversation context
            "gen_ai.conversation.id" = %event.session_id,
            // Duration
            "duration_ms" = duration_ms.unwrap_or(0),
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

    /// Handle turn.started event - record start for invoke_agent span
    fn handle_turn_started(&self, event: &Event, data: &TurnStartedData) {
        let mut spans = self.turn_spans.lock().unwrap();
        spans.insert(
            data.turn_id,
            TurnSpanInfo {
                session_id: event.session_id,
                agent_id: None, // Will be filled from context if available
                started_at: std::time::Instant::now(),
            },
        );
    }

    /// Handle turn.completed event - create invoke_agent span
    fn handle_turn_completed(&self, event: &Event, data: &TurnCompletedData) {
        // Get start info if available
        let start_info = {
            let mut spans = self.turn_spans.lock().unwrap();
            spans.remove(&data.turn_id)
        };

        let duration_ms = data.duration_ms.or_else(|| {
            start_info
                .as_ref()
                .map(|info| info.started_at.elapsed().as_millis() as u64)
        });

        // Create span with gen-ai semantic conventions
        let span_name = format!("invoke_agent {}", data.turn_id);
        let span = tracing::info_span!(
            "gen_ai.invoke_agent",
            "otel.name" = %span_name,
            "otel.kind" = "internal",
            // Operation
            "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
            // Turn context
            "turn.id" = %data.turn_id,
            "turn.iterations" = %data.iterations,
            // Conversation context
            "gen_ai.conversation.id" = %event.session_id,
            // Duration
            "duration_ms" = duration_ms.unwrap_or(0),
        );

        let _guard = span.enter();

        tracing::debug!(
            turn_id = %data.turn_id,
            iterations = %data.iterations,
            duration_ms = ?duration_ms,
            "Turn completed"
        );
    }

    /// Get the number of tracked in-flight tool calls (for testing)
    #[cfg(test)]
    fn pending_tool_calls(&self) -> usize {
        self.tool_call_spans.lock().unwrap().len()
    }

    /// Get the number of tracked in-flight turns (for testing)
    #[cfg(test)]
    fn pending_turns(&self) -> usize {
        self.turn_spans.lock().unwrap().len()
    }
}

#[async_trait]
impl EventListener for OtelEventListener {
    async fn on_event(&self, event: &Event) {
        match &event.data {
            EventData::LlmGeneration(data) => {
                self.handle_llm_generation(event, data);
            }
            EventData::ToolCallStarted(data) => {
                self.handle_tool_call_started(event, data);
            }
            EventData::ToolCallCompleted(data) => {
                self.handle_tool_call_completed(event, data);
            }
            EventData::TurnStarted(data) => {
                self.handle_turn_started(event, data);
            }
            EventData::TurnCompleted(data) => {
                self.handle_turn_completed(event, data);
            }
            // Other events don't generate spans
            _ => {}
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        // Only listen to events we generate spans for
        Some(vec![
            LLM_GENERATION,
            TOOL_CALL_STARTED,
            TOOL_CALL_COMPLETED,
            TURN_STARTED,
            TURN_COMPLETED,
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
    use crate::events::{EventContext, LlmGenerationMetadata, LlmGenerationOutput, TokenUsage};
    use crate::message::Message;
    use crate::tool_types::ToolCall;
    use serde_json::json;

    #[tokio::test]
    async fn test_otel_listener_creation() {
        let listener = OtelEventListener::new();
        assert_eq!(listener.name(), "OtelEventListener");
    }

    #[tokio::test]
    async fn test_otel_listener_default() {
        let listener = OtelEventListener::default();
        assert_eq!(listener.name(), "OtelEventListener");
    }

    #[tokio::test]
    async fn test_otel_listener_event_types() {
        let listener = OtelEventListener::new();
        let types = listener.event_types().unwrap();
        assert_eq!(types.len(), 5);
        assert!(types.contains(&LLM_GENERATION));
        assert!(types.contains(&TOOL_CALL_STARTED));
        assert!(types.contains(&TOOL_CALL_COMPLETED));
        assert!(types.contains(&TURN_STARTED));
        assert!(types.contains(&TURN_COMPLETED));
    }

    #[tokio::test]
    async fn test_handle_llm_generation_success() {
        let listener = OtelEventListener::new();

        let data = LlmGenerationData {
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
                }),
                duration_ms: Some(100),
                success: true,
                error: None,
                finish_reasons: Some(vec!["stop".to_string()]),
                response_id: Some("resp_123".to_string()),
            },
        };

        let event = Event::new(
            Uuid::now_v7(),
            EventContext::empty(),
            EventData::LlmGeneration(data),
        );

        // Should not panic
        listener.on_event(&event).await;
    }

    #[tokio::test]
    async fn test_handle_llm_generation_with_tool_calls() {
        let listener = OtelEventListener::new();

        let tool_calls = vec![ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: json!({"city": "Tokyo"}),
        }];

        let data = LlmGenerationData {
            messages: vec![Message::user("What's the weather?")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Let me check...".to_string()),
                tool_calls,
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4o".to_string(),
                provider: Some("openai".to_string()),
                usage: Some(TokenUsage {
                    input_tokens: 20,
                    output_tokens: 15,
                }),
                duration_ms: Some(200),
                success: true,
                error: None,
                finish_reasons: Some(vec!["tool_calls".to_string()]),
                response_id: Some("resp_456".to_string()),
            },
        };

        let event = Event::new(
            Uuid::now_v7(),
            EventContext::empty(),
            EventData::LlmGeneration(data),
        );

        listener.on_event(&event).await;
    }

    #[tokio::test]
    async fn test_handle_llm_generation_without_optional_fields() {
        let listener = OtelEventListener::new();

        let data = LlmGenerationData {
            messages: vec![Message::user("Hello")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Hi!".to_string()),
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "claude-3".to_string(),
                provider: None, // No provider
                usage: None,    // No usage
                duration_ms: None,
                success: true,
                error: None,
                finish_reasons: None, // No finish reasons
                response_id: None,    // No response ID
            },
        };

        let event = Event::new(
            Uuid::now_v7(),
            EventContext::empty(),
            EventData::LlmGeneration(data),
        );

        // Should not panic with missing optional fields
        listener.on_event(&event).await;
    }

    #[tokio::test]
    async fn test_handle_tool_call_lifecycle() {
        let listener = OtelEventListener::new();

        // Start a tool call
        let started_data = ToolCallStartedData {
            tool_call: ToolCall {
                id: "call_abc".to_string(),
                name: "calculate".to_string(),
                arguments: json!({}),
            },
        };

        let start_event = Event::new(
            Uuid::now_v7(),
            EventContext::empty(),
            EventData::ToolCallStarted(started_data),
        );

        listener.on_event(&start_event).await;
        assert_eq!(listener.pending_tool_calls(), 1);

        // Complete the tool call
        let completed_data = ToolCallCompletedData {
            tool_call_id: "call_abc".to_string(),
            tool_name: "calculate".to_string(),
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
        };

        let complete_event = Event::new(
            Uuid::now_v7(),
            EventContext::empty(),
            EventData::ToolCallCompleted(completed_data),
        );

        listener.on_event(&complete_event).await;
        assert_eq!(listener.pending_tool_calls(), 0);
    }

    #[tokio::test]
    async fn test_handle_tool_call_completed_without_start() {
        let listener = OtelEventListener::new();

        // Complete a tool call that was never started (e.g., after restart)
        let completed_data = ToolCallCompletedData {
            tool_call_id: "orphan_call".to_string(),
            tool_name: "unknown_tool".to_string(),
            success: false,
            status: "error".to_string(),
            result: None,
            error: Some("Connection timeout".to_string()),
        };

        let event = Event::new(
            Uuid::now_v7(),
            EventContext::empty(),
            EventData::ToolCallCompleted(completed_data),
        );

        // Should not panic
        listener.on_event(&event).await;
    }

    #[tokio::test]
    async fn test_handle_turn_lifecycle() {
        let listener = OtelEventListener::new();
        let turn_id = Uuid::now_v7();
        let session_id = Uuid::now_v7();

        // Start a turn
        let started_data = TurnStartedData {
            turn_id,
            input_message_id: Uuid::now_v7(),
        };

        let start_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnStarted(started_data),
        );

        listener.on_event(&start_event).await;
        assert_eq!(listener.pending_turns(), 1);

        // Complete the turn
        let completed_data = TurnCompletedData {
            turn_id,
            iterations: 3,
            duration_ms: Some(1500),
        };

        let complete_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnCompleted(completed_data),
        );

        listener.on_event(&complete_event).await;
        assert_eq!(listener.pending_turns(), 0);
    }

    #[tokio::test]
    async fn test_handle_turn_completed_without_start() {
        let listener = OtelEventListener::new();

        // Complete a turn that was never started
        let completed_data = TurnCompletedData {
            turn_id: Uuid::now_v7(),
            iterations: 1,
            duration_ms: None, // No duration provided
        };

        let event = Event::new(
            Uuid::now_v7(),
            EventContext::empty(),
            EventData::TurnCompleted(completed_data),
        );

        // Should not panic
        listener.on_event(&event).await;
    }

    #[tokio::test]
    async fn test_unhandled_event_types() {
        use crate::events::MessageUserData;

        let listener = OtelEventListener::new();

        // Send an event type that OtelEventListener doesn't handle
        let event = Event::new(
            Uuid::now_v7(),
            EventContext::empty(),
            EventData::MessageUser(MessageUserData {
                message: Message::user("Hello"),
            }),
        );

        // Should not panic, just ignore
        listener.on_event(&event).await;
    }

    #[tokio::test]
    async fn test_multiple_concurrent_tool_calls() {
        let listener = OtelEventListener::new();

        // Start multiple tool calls
        for i in 0..3 {
            let started_data = ToolCallStartedData {
                tool_call: ToolCall {
                    id: format!("call_{}", i),
                    name: format!("tool_{}", i),
                    arguments: json!({}),
                },
            };

            let event = Event::new(
                Uuid::now_v7(),
                EventContext::empty(),
                EventData::ToolCallStarted(started_data),
            );

            listener.on_event(&event).await;
        }

        assert_eq!(listener.pending_tool_calls(), 3);

        // Complete them in different order
        for i in [2, 0, 1] {
            let completed_data = ToolCallCompletedData {
                tool_call_id: format!("call_{}", i),
                tool_name: format!("tool_{}", i),
                success: true,
                status: "success".to_string(),
                result: None,
                error: None,
            };

            let event = Event::new(
                Uuid::now_v7(),
                EventContext::empty(),
                EventData::ToolCallCompleted(completed_data),
            );

            listener.on_event(&event).await;
        }

        assert_eq!(listener.pending_tool_calls(), 0);
    }
}
