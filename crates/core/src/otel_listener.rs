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

use crate::events::{
    Event, EventData, LlmGenerationData, ToolCallCompletedData, ToolCallStartedData,
    TurnCompletedData, TurnStartedData, LLM_GENERATION, TOOL_CALL_COMPLETED, TOOL_CALL_STARTED,
    TURN_COMPLETED, TURN_STARTED,
};
use crate::telemetry::gen_ai;
use crate::traits::EventListener;

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
pub struct OtelEventListener {
    /// Track in-flight tool calls for span correlation
    /// Key: tool_call_id, Value: span handle info
    #[allow(dead_code)]
    tool_call_spans: Mutex<HashMap<String, ToolCallSpanInfo>>,

    /// Track in-flight turns for span correlation
    /// Key: turn_id, Value: span handle info
    #[allow(dead_code)]
    turn_spans: Mutex<HashMap<Uuid, TurnSpanInfo>>,
}

#[allow(dead_code)]
struct ToolCallSpanInfo {
    tool_name: String,
    started_at: std::time::Instant,
}

#[allow(dead_code)]
struct TurnSpanInfo {
    session_id: Uuid,
    agent_id: Option<Uuid>,
    started_at: std::time::Instant,
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

    #[tokio::test]
    async fn test_otel_listener_creation() {
        let listener = OtelEventListener::new();
        assert_eq!(listener.name(), "OtelEventListener");
    }

    #[tokio::test]
    async fn test_otel_listener_event_types() {
        let listener = OtelEventListener::new();
        let types = listener.event_types().unwrap();
        assert!(types.contains(&LLM_GENERATION));
        assert!(types.contains(&TOOL_CALL_STARTED));
        assert!(types.contains(&TOOL_CALL_COMPLETED));
        assert!(types.contains(&TURN_STARTED));
        assert!(types.contains(&TURN_COMPLETED));
    }

    #[tokio::test]
    async fn test_handle_llm_generation() {
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
}
