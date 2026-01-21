// OpenTelemetry Event Listener
//
// This listener generates OTel spans from events following the gen-ai semantic conventions.
// See: https://opentelemetry.io/docs/specs/semconv/gen-ai/
//
// IMPORTANT: Spans are hierarchical to show the full agentic loop in Phoenix:
// - AGENT span (turn) is the root span for each conversation turn
// - LLM spans (generations) are children of the AGENT span
// - TOOL spans (executions) are children of the AGENT span
//
// This hierarchy allows Phoenix to visualize:
// - The full agent iteration loop
// - Token usage aggregated per turn
// - Tool calls in context of the agent's reasoning

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::Span;
use uuid::Uuid;

use crate::event_listeners::EventListener;
use crate::events::{
    Event, EventData, LLM_GENERATION, LlmGenerationData, TOOL_CALL_COMPLETED, TOOL_CALL_STARTED,
    TURN_COMPLETED, TURN_STARTED, ToolCallCompletedData, ToolCallStartedData, TurnCompletedData,
    TurnStartedData,
};
use crate::telemetry::{gen_ai, openinference};

// ============================================================================
// Active Span Tracking
// ============================================================================

/// Active turn span - kept open for the duration of the turn
struct ActiveTurn {
    /// The tracing span for this turn (kept open until turn completes)
    span: Span,
    /// When the turn started
    started_at: std::time::Instant,
}

/// Active tool call span - kept open until tool execution completes
struct ActiveToolCall {
    /// The tracing span for this tool call
    span: Span,
    /// When the tool call started
    started_at: std::time::Instant,
}

// ============================================================================
// OtelEventListener
// ============================================================================

/// OpenTelemetry event listener that generates hierarchical gen-ai spans.
///
/// ## Span Hierarchy
///
/// This listener creates a proper span hierarchy for the agentic loop:
///
/// ```text
/// AGENT span (turn.started → turn.completed)
///   ├── LLM span (llm.generation)
///   ├── TOOL span (tool.call_started → tool.call_completed)
///   ├── LLM span (llm.generation)
///   └── ...
/// ```
///
/// This hierarchy is critical for Phoenix to display:
/// - The full conversation turn with all iterations
/// - Nested LLM calls within the agent context
/// - Tool executions as children of the agent turn
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
pub struct OtelEventListener {
    /// Active turns per session (only one turn active per session at a time)
    /// Key: session_id, Value: active turn info with span
    active_turns: Mutex<HashMap<Uuid, ActiveTurn>>,

    /// Active tool calls (can have multiple concurrent tool calls)
    /// Key: tool_call_id, Value: active tool call info with span
    active_tool_calls: Mutex<HashMap<String, ActiveToolCall>>,
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
            active_turns: Mutex::new(HashMap::new()),
            active_tool_calls: Mutex::new(HashMap::new()),
        }
    }

    /// Handle turn.started event - create and keep open an AGENT span
    ///
    /// This span becomes the parent for all LLM and TOOL spans within the turn.
    fn handle_turn_started(&self, event: &Event, data: &TurnStartedData) {
        let span_name = format!("agent_turn {}", data.turn_id);

        // Create the AGENT span - this will be kept open until turn.completed
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

            // === Turn context ===
            "turn.id" = %data.turn_id,
            "turn.input_message_id" = %data.input_message_id,

            // These will be filled in at turn.completed:
            "turn.iterations" = tracing::field::Empty,
            "gen_ai.usage.input_tokens" = tracing::field::Empty,
            "gen_ai.usage.output_tokens" = tracing::field::Empty,
            "llm.token_count.prompt" = tracing::field::Empty,
            "llm.token_count.completion" = tracing::field::Empty,
            "llm.token_count.total" = tracing::field::Empty,
            "duration_ms" = tracing::field::Empty,
        );

        // Store the active turn (span stays open)
        let mut turns = self.active_turns.lock().unwrap();
        turns.insert(
            event.session_id.uuid(),
            ActiveTurn {
                span,
                started_at: std::time::Instant::now(),
            },
        );

        tracing::debug!(
            turn_id = %data.turn_id,
            session_id = %event.session_id,
            "Turn started - AGENT span opened"
        );
    }

    /// Handle turn.completed event - close the AGENT span with final metrics
    fn handle_turn_completed(&self, event: &Event, data: &TurnCompletedData) {
        // Get and remove the active turn
        let active_turn = {
            let mut turns = self.active_turns.lock().unwrap();
            turns.remove(&event.session_id.uuid())
        };

        let Some(active_turn) = active_turn else {
            tracing::warn!(
                turn_id = %data.turn_id,
                session_id = %event.session_id,
                "Turn completed but no active turn found - span may be missing"
            );
            return;
        };

        // Calculate duration
        let duration_ms = data
            .duration_ms
            .unwrap_or_else(|| active_turn.started_at.elapsed().as_millis() as u64);

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

        // Record final attributes on the span
        active_turn.span.record("turn.iterations", data.iterations);
        active_turn
            .span
            .record("gen_ai.usage.input_tokens", input_tokens);
        active_turn
            .span
            .record("gen_ai.usage.output_tokens", output_tokens);
        active_turn
            .span
            .record("llm.token_count.prompt", input_tokens);
        active_turn
            .span
            .record("llm.token_count.completion", output_tokens);
        active_turn
            .span
            .record("llm.token_count.total", total_tokens);
        active_turn.span.record("duration_ms", duration_ms);

        // Enter the span briefly to log completion, then it closes when dropped
        let _guard = active_turn.span.enter();
        tracing::debug!(
            turn_id = %data.turn_id,
            iterations = data.iterations,
            duration_ms = duration_ms,
            input_tokens = input_tokens,
            output_tokens = output_tokens,
            "Turn completed - AGENT span closed"
        );

        // Span closes here when active_turn is dropped
    }

    /// Handle llm.generation event - create an LLM span as child of the active AGENT span
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

        // Get the active turn span to use as parent
        let turns = self.active_turns.lock().unwrap();
        let parent_span = turns.get(&event.session_id.uuid()).map(|t| &t.span);

        // Create the LLM span - either as child of AGENT span or standalone
        let span_name = format!("chat {}", model);

        let create_llm_span = || {
            tracing::info_span!(
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

                // === Custom attributes ===
                "duration_ms" = data.metadata.duration_ms.unwrap_or(0),
                "time_to_first_token_ms" = data.metadata.time_to_first_token_ms.unwrap_or(0),
                "success" = data.metadata.success,
            )
        };

        // Create span within parent context if available
        if let Some(parent) = parent_span {
            // Create LLM span as child of AGENT span
            parent.in_scope(|| {
                let span = create_llm_span();
                let _guard = span.enter();
                tracing::debug!(
                    model = %model,
                    provider = %provider,
                    success = %data.metadata.success,
                    input_tokens = input_tokens,
                    output_tokens = output_tokens,
                    tool_calls = %data.output.tool_calls.len(),
                    "LLM generation completed (child of AGENT span)"
                );
            });
        } else {
            // No active turn - create standalone span (shouldn't happen normally)
            let span = create_llm_span();
            let _guard = span.enter();
            tracing::debug!(
                model = %model,
                provider = %provider,
                success = %data.metadata.success,
                input_tokens = input_tokens,
                output_tokens = output_tokens,
                tool_calls = %data.output.tool_calls.len(),
                "LLM generation completed (standalone - no active turn)"
            );
        }
    }

    /// Handle tool.call_started event - create and keep open a TOOL span as child of AGENT
    fn handle_tool_call_started(&self, event: &Event, data: &ToolCallStartedData) {
        let tool_name = &data.tool_call.name;
        let tool_call_id = &data.tool_call.id;

        // Get the active turn span to use as parent
        let turns = self.active_turns.lock().unwrap();
        let parent_span = turns.get(&event.session_id.uuid()).map(|t| &t.span);

        let span_name = format!("tool {}", tool_name);

        let create_tool_span = || {
            tracing::info_span!(
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

                // These will be filled in at tool.call_completed:
                "tool.success" = tracing::field::Empty,
                "tool.status" = tracing::field::Empty,
                "duration_ms" = tracing::field::Empty,
            )
        };

        // Create span within parent context if available
        let span = if let Some(parent) = parent_span {
            parent.in_scope(create_tool_span)
        } else {
            create_tool_span()
        };

        // Store the active tool call (span stays open)
        let mut tool_calls = self.active_tool_calls.lock().unwrap();
        tool_calls.insert(
            tool_call_id.clone(),
            ActiveToolCall {
                span,
                started_at: std::time::Instant::now(),
            },
        );

        tracing::debug!(
            tool_name = %tool_name,
            tool_call_id = %tool_call_id,
            "Tool execution started - TOOL span opened"
        );
    }

    /// Handle tool.call_completed event - close the TOOL span with result
    fn handle_tool_call_completed(&self, event: &Event, data: &ToolCallCompletedData) {
        // Get and remove the active tool call
        let active_tool = {
            let mut tool_calls = self.active_tool_calls.lock().unwrap();
            tool_calls.remove(&data.tool_call_id)
        };

        let Some(active_tool) = active_tool else {
            // Tool call completed without a started event - create a point-in-time span
            tracing::debug!(
                tool_name = %data.tool_name,
                tool_call_id = %data.tool_call_id,
                "Tool completed but no active tool call found - creating point-in-time span"
            );

            // Get parent span for context
            let turns = self.active_turns.lock().unwrap();
            let parent_span = turns.get(&event.session_id.uuid()).map(|t| &t.span);

            let span_name = format!("tool {}", data.tool_name);
            let create_span = || {
                tracing::info_span!(
                    "tool_execution",
                    "otel.name" = %span_name,
                    "otel.kind" = "internal",
                    "gen_ai.operation.name" = gen_ai::operation::EXECUTE_TOOL,
                    "gen_ai.tool.name" = %data.tool_name,
                    "gen_ai.tool.call.id" = %data.tool_call_id,
                    "gen_ai.conversation.id" = %event.session_id,
                    "openinference.span.kind" = openinference::span_kind::TOOL,
                    "tool.name" = %data.tool_name,
                    "tool.success" = %data.success,
                    "tool.status" = %data.status,
                    "session.id" = %event.session_id,
                )
            };

            if let Some(parent) = parent_span {
                parent.in_scope(|| {
                    let span = create_span();
                    let _guard = span.enter();
                });
            } else {
                let span = create_span();
                let _guard = span.enter();
            }
            return;
        };

        // Calculate duration
        let duration_ms = active_tool.started_at.elapsed().as_millis() as u64;

        // Record final attributes on the span
        active_tool.span.record("tool.success", data.success);
        active_tool.span.record("tool.status", &data.status);
        active_tool.span.record("duration_ms", duration_ms);

        // Enter the span briefly to log completion
        let _guard = active_tool.span.enter();

        if data.success {
            tracing::debug!(
                tool_name = %data.tool_name,
                tool_call_id = %data.tool_call_id,
                duration_ms = duration_ms,
                "Tool execution succeeded - TOOL span closed"
            );
        } else {
            tracing::warn!(
                tool_name = %data.tool_name,
                tool_call_id = %data.tool_call_id,
                duration_ms = duration_ms,
                error = ?data.error,
                "Tool execution failed - TOOL span closed"
            );
        }

        // Span closes here when active_tool is dropped
    }

    /// Get the number of active turns (for testing)
    #[cfg(test)]
    fn active_turn_count(&self) -> usize {
        self.active_turns.lock().unwrap().len()
    }

    /// Get the number of active tool calls (for testing)
    #[cfg(test)]
    fn active_tool_call_count(&self) -> usize {
        self.active_tool_calls.lock().unwrap().len()
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
        // Listen to events needed for span hierarchy
        Some(vec![
            TURN_STARTED,
            TURN_COMPLETED,
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
    use crate::events::{EventContext, LlmGenerationMetadata, LlmGenerationOutput, TokenUsage};
    use crate::message::Message;
    use crate::tool_types::ToolCall;
    use crate::typed_id::{MessageId, TurnId};
    use serde_json::json;

    #[tokio::test]
    async fn test_otel_listener_creation() {
        let listener = OtelEventListener::new();
        assert_eq!(listener.name(), "OtelEventListener");
        assert_eq!(listener.active_turn_count(), 0);
        assert_eq!(listener.active_tool_call_count(), 0);
    }

    #[tokio::test]
    async fn test_otel_listener_event_types() {
        let listener = OtelEventListener::new();
        let types = listener.event_types().unwrap();
        assert_eq!(types.len(), 5);
        assert!(types.contains(&TURN_STARTED));
        assert!(types.contains(&TURN_COMPLETED));
        assert!(types.contains(&LLM_GENERATION));
        assert!(types.contains(&TOOL_CALL_STARTED));
        assert!(types.contains(&TOOL_CALL_COMPLETED));
    }

    #[tokio::test]
    async fn test_turn_lifecycle_creates_agent_span() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();
        let turn_id = TurnId::from_uuid(Uuid::now_v7());

        // Start turn
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

        // Verify turn is active
        assert_eq!(listener.active_turn_count(), 1);

        // Complete turn
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

        // Verify turn is closed
        assert_eq!(listener.active_turn_count(), 0);
    }

    #[tokio::test]
    async fn test_llm_generation_within_turn() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();
        let turn_id = TurnId::from_uuid(Uuid::now_v7());

        // Start turn first
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

        // LLM generation during turn
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
        let llm_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::LlmGeneration(llm_data),
        );
        listener.on_event(&llm_event).await;

        // Turn should still be active (LLM span is point-in-time child)
        assert_eq!(listener.active_turn_count(), 1);

        // Complete turn
        let completed_data = TurnCompletedData {
            turn_id,
            iterations: 1,
            duration_ms: Some(200),
            usage: None,
        };
        let complete_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnCompleted(completed_data),
        );
        listener.on_event(&complete_event).await;

        assert_eq!(listener.active_turn_count(), 0);
    }

    #[tokio::test]
    async fn test_tool_call_lifecycle_within_turn() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();
        let turn_id = TurnId::from_uuid(Uuid::now_v7());

        // Start turn
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

        // Verify tool call is active
        assert_eq!(listener.active_tool_call_count(), 1);
        assert_eq!(listener.active_turn_count(), 1);

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

        // Tool call closed, turn still active
        assert_eq!(listener.active_tool_call_count(), 0);
        assert_eq!(listener.active_turn_count(), 1);

        // Complete turn
        let completed_data = TurnCompletedData {
            turn_id,
            iterations: 1,
            duration_ms: Some(300),
            usage: None,
        };
        let complete_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnCompleted(completed_data),
        );
        listener.on_event(&complete_event).await;

        assert_eq!(listener.active_turn_count(), 0);
    }

    #[tokio::test]
    async fn test_full_agentic_loop() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();
        let turn_id = TurnId::from_uuid(Uuid::now_v7());

        // 1. Turn starts
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnStarted(TurnStartedData {
                    turn_id,
                    input_message_id: MessageId::from_uuid(Uuid::now_v7()),
                }),
            ))
            .await;

        // 2. First LLM generation (decides to call tool)
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
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
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
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
                EventContext::empty(),
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
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
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
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
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

        // All spans should be closed
        assert_eq!(listener.active_turn_count(), 0);
        assert_eq!(listener.active_tool_call_count(), 0);
    }

    #[tokio::test]
    async fn test_tool_completed_without_started() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();

        // Tool completed without started event (e.g., after restart)
        let completed_data = ToolCallCompletedData {
            tool_call_id: "orphan_call".to_string(),
            tool_name: "unknown_tool".to_string(),
            success: false,
            status: "error".to_string(),
            result: None,
            error: Some("Connection timeout".to_string()),
        };

        let event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::ToolCallCompleted(completed_data),
        );

        // Should not panic - creates point-in-time span
        listener.on_event(&event).await;
        assert_eq!(listener.active_tool_call_count(), 0);
    }

    #[tokio::test]
    async fn test_turn_completed_without_started() {
        let listener = OtelEventListener::new();
        let session_id = Uuid::now_v7();

        // Turn completed without started event
        let completed_data = TurnCompletedData {
            turn_id: TurnId::from_uuid(Uuid::now_v7()),
            iterations: 1,
            duration_ms: None,
            usage: None,
        };

        let event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnCompleted(completed_data),
        );

        // Should not panic - just logs warning
        listener.on_event(&event).await;
        assert_eq!(listener.active_turn_count(), 0);
    }
}
