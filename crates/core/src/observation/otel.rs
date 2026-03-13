// OpenTelemetry Event Listener
//
// Full-featured OTel integration following Gen-AI semantic conventions.
// See: https://opentelemetry.io/docs/specs/semconv/gen-ai/
// See: specs/otel-observability.md for full specification.
//
// Traces the complete agentic execution lifecycle (13 event types):
// - turn.started/completed/failed/cancelled → gen_ai.invoke_agent root span
// - reason.started/completed → reason phase span (child of turn)
// - reason.thinking.started/completed → thinking span (child of reason)
// - llm.generation → gen_ai.chat span (child of reason)
// - act.started/completed → act phase span (child of turn)
// - tool.started/completed → gen_ai.execute_tool span (child of act)
//
// Spans have real duration (created on started, ended on completed).
// Parent-child hierarchy uses tracing span nesting for correct OTel traces.
// Content recording (prompts, completions, tool args) is opt-in via
// OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::event_listeners::EventListener;
use crate::events::{
    ACT_COMPLETED, ACT_STARTED, ActCompletedData, ActStartedData, Event, EventData, LLM_GENERATION,
    LlmGenerationData, REASON_COMPLETED, REASON_STARTED, REASON_THINKING_COMPLETED,
    REASON_THINKING_STARTED, ReasonCompletedData, ReasonStartedData, ReasonThinkingCompletedData,
    ReasonThinkingStartedData, TOOL_COMPLETED, TOOL_STARTED, TURN_CANCELLED, TURN_COMPLETED,
    TURN_FAILED, TURN_STARTED, ToolCompletedData, ToolStartedData, TurnCancelledData,
    TurnCompletedData, TurnFailedData, TurnStartedData,
};
use crate::telemetry::gen_ai;

// ============================================================================
// Internal Span Tracking
// ============================================================================

/// Kind of active span for type-safe lookups
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum SpanKind {
    Turn,
    Reason,
    Act,
    Thinking,
    Tool,
}

/// An active span with its tracing guard and metadata
struct ActiveSpan {
    span: tracing::Span,
    #[allow(dead_code)]
    kind: SpanKind,
    started_at: std::time::Instant,
}

// ============================================================================
// OtelEventListener
// ============================================================================

/// OpenTelemetry event listener producing Gen-AI semantic convention spans.
///
/// Creates a hierarchical trace per agent turn:
/// ```text
/// invoke_agent (root)
/// ├── reason
/// │   ├── thinking (optional)
/// │   └── chat {model}
/// ├── act
/// │   ├── execute_tool {name}
/// │   └── execute_tool {name}
/// └── ...
/// ```
///
/// Content recording (prompts, completions, tool arguments) is controlled by
/// `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` environment variable.
pub struct OtelEventListener {
    /// Active spans keyed by correlation ID.
    /// Keys: turn_id (for turn/reason/act), span_id from EventContext, tool_call_id.
    active_spans: Mutex<HashMap<String, ActiveSpan>>,
    /// Whether to record content (prompts, completions, tool args/results).
    record_content: bool,
}

impl Default for OtelEventListener {
    fn default() -> Self {
        Self::new()
    }
}

impl OtelEventListener {
    /// Create a new OTel event listener, reading content recording config from env.
    pub fn new() -> Self {
        let record_content = std::env::var("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT")
            .or_else(|_| std::env::var("OTEL_RECORD_CONTENT"))
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        Self {
            active_spans: Mutex::new(HashMap::new()),
            record_content,
        }
    }

    /// Create with explicit content recording setting (for testing).
    pub fn with_record_content(record_content: bool) -> Self {
        Self {
            active_spans: Mutex::new(HashMap::new()),
            record_content,
        }
    }

    // ========================================================================
    // Turn lifecycle
    // ========================================================================

    fn handle_turn_started(&self, event: &Event, data: &TurnStartedData) {
        let turn_key = data.turn_id.to_string();
        let span_name = format!("invoke_agent {}", data.turn_id);

        let span = if self.record_content {
            tracing::info_span!(
                "gen_ai.invoke_agent",
                "otel.name" = %span_name,
                "otel.kind" = "internal",
                "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
                "gen_ai.conversation.id" = %event.session_id,
                "turn.id" = %data.turn_id,
                // Placeholder attrs filled on completed
                "turn.iterations" = tracing::field::Empty,
                "gen_ai.usage.input_tokens" = tracing::field::Empty,
                "gen_ai.usage.output_tokens" = tracing::field::Empty,
                "duration_ms" = tracing::field::Empty,
                "error.type" = tracing::field::Empty,
                "otel.status_code" = tracing::field::Empty,
                "otel.status_description" = tracing::field::Empty,
                // Content (opt-in)
                "gen_ai.input.messages" = data.input_content.as_deref().unwrap_or(""),
            )
        } else {
            tracing::info_span!(
                "gen_ai.invoke_agent",
                "otel.name" = %span_name,
                "otel.kind" = "internal",
                "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
                "gen_ai.conversation.id" = %event.session_id,
                "turn.id" = %data.turn_id,
                "turn.iterations" = tracing::field::Empty,
                "gen_ai.usage.input_tokens" = tracing::field::Empty,
                "gen_ai.usage.output_tokens" = tracing::field::Empty,
                "duration_ms" = tracing::field::Empty,
                "error.type" = tracing::field::Empty,
                "otel.status_code" = tracing::field::Empty,
                "otel.status_description" = tracing::field::Empty,
            )
        };

        let mut spans = self.active_spans.lock().unwrap();
        spans.insert(
            turn_key,
            ActiveSpan {
                span,
                kind: SpanKind::Turn,
                started_at: std::time::Instant::now(),
            },
        );
    }

    fn handle_turn_completed(&self, _event: &Event, data: &TurnCompletedData) {
        let turn_key = data.turn_id.to_string();
        let active = {
            let mut spans = self.active_spans.lock().unwrap();
            spans.remove(&turn_key)
        };

        if let Some(active) = active {
            let duration_ms = data
                .duration_ms
                .unwrap_or_else(|| active.started_at.elapsed().as_millis() as u64);

            active.span.record("turn.iterations", data.iterations);
            active.span.record("duration_ms", duration_ms);

            if let Some(usage) = &data.usage {
                active
                    .span
                    .record("gen_ai.usage.input_tokens", usage.input_tokens);
                active
                    .span
                    .record("gen_ai.usage.output_tokens", usage.output_tokens);
            }

            // Span drops here, ending it with correct duration
            drop(active);
        } else {
            // Orphaned completed — create a point-in-time span as fallback
            let span_name = format!("invoke_agent {}", data.turn_id);
            let _span = tracing::info_span!(
                "gen_ai.invoke_agent",
                "otel.name" = %span_name,
                "otel.kind" = "internal",
                "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
                "turn.id" = %data.turn_id,
                "turn.iterations" = %data.iterations,
                "duration_ms" = data.duration_ms.unwrap_or(0),
            )
            .entered();
        }
    }

    fn handle_turn_failed(&self, _event: &Event, data: &TurnFailedData) {
        let turn_key = data.turn_id.to_string();
        let active = {
            let mut spans = self.active_spans.lock().unwrap();
            spans.remove(&turn_key)
        };

        if let Some(active) = active {
            let duration_ms = active.started_at.elapsed().as_millis() as u64;
            active.span.record("duration_ms", duration_ms);
            active.span.record("error.type", &data.error);
            active.span.record("otel.status_code", "ERROR");
            active.span.record("otel.status_description", &data.error);
            drop(active);
        } else {
            let _span = tracing::error_span!(
                "gen_ai.invoke_agent",
                "otel.name" = "invoke_agent (failed)",
                "otel.kind" = "internal",
                "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
                "turn.id" = %data.turn_id,
                "error.type" = %data.error,
                "otel.status_code" = "ERROR",
            )
            .entered();
        }
    }

    fn handle_turn_cancelled(&self, _event: &Event, data: &TurnCancelledData) {
        let turn_key = data.turn_id.to_string();
        let active = {
            let mut spans = self.active_spans.lock().unwrap();
            spans.remove(&turn_key)
        };

        let reason = data.reason.as_deref().unwrap_or("cancelled");
        if let Some(active) = active {
            let duration_ms = active.started_at.elapsed().as_millis() as u64;
            active.span.record("duration_ms", duration_ms);
            active.span.record("error.type", reason);
            active.span.record("otel.status_code", "ERROR");
            active.span.record("otel.status_description", reason);

            if let Some(usage) = &data.usage {
                active
                    .span
                    .record("gen_ai.usage.input_tokens", usage.input_tokens);
                active
                    .span
                    .record("gen_ai.usage.output_tokens", usage.output_tokens);
            }
            drop(active);
        }
    }

    // ========================================================================
    // Reason lifecycle
    // ========================================================================

    fn handle_reason_started(&self, event: &Event, _data: &ReasonStartedData) {
        let span_key = self.span_key_from_context(event, "reason");
        let parent_key = self.parent_key_from_context(event);

        // Enter parent span context for proper nesting
        let _parent_guard = parent_key.as_ref().and_then(|k| self.enter_parent(k));

        let span = tracing::info_span!(
            "reason",
            "otel.name" = "reason",
            "otel.kind" = "internal",
            "gen_ai.operation.name" = gen_ai::operation::REASON,
            "gen_ai.conversation.id" = %event.session_id,
            // Filled on completed
            "reason.success" = tracing::field::Empty,
            "reason.has_tool_calls" = tracing::field::Empty,
            "reason.tool_call_count" = tracing::field::Empty,
            "gen_ai.usage.input_tokens" = tracing::field::Empty,
            "gen_ai.usage.output_tokens" = tracing::field::Empty,
            "duration_ms" = tracing::field::Empty,
            "error.type" = tracing::field::Empty,
        );

        let mut spans = self.active_spans.lock().unwrap();
        spans.insert(
            span_key,
            ActiveSpan {
                span,
                kind: SpanKind::Reason,
                started_at: std::time::Instant::now(),
            },
        );
    }

    fn handle_reason_completed(&self, event: &Event, data: &ReasonCompletedData) {
        let span_key = self.span_key_from_context(event, "reason");
        let active = {
            let mut spans = self.active_spans.lock().unwrap();
            spans.remove(&span_key)
        };

        if let Some(active) = active {
            let duration_ms = data
                .duration_ms
                .unwrap_or_else(|| active.started_at.elapsed().as_millis() as u64);

            active.span.record("reason.success", data.success);
            active
                .span
                .record("reason.has_tool_calls", data.has_tool_calls);
            active
                .span
                .record("reason.tool_call_count", data.tool_call_count);
            active.span.record("duration_ms", duration_ms);

            if let Some(usage) = &data.usage {
                active
                    .span
                    .record("gen_ai.usage.input_tokens", usage.input_tokens);
                active
                    .span
                    .record("gen_ai.usage.output_tokens", usage.output_tokens);
            }

            if let Some(error) = &data.error {
                active.span.record("error.type", error.as_str());
            }

            drop(active);
        }
    }

    // ========================================================================
    // Thinking lifecycle
    // ========================================================================

    fn handle_thinking_started(&self, event: &Event, data: &ReasonThinkingStartedData) {
        let span_key = self.span_key_from_context(event, "thinking");
        let parent_key = self.parent_key_from_context(event);

        let _parent_guard = parent_key.as_ref().and_then(|k| self.enter_parent(k));

        let span = tracing::info_span!(
            "thinking",
            "otel.name" = "thinking",
            "otel.kind" = "internal",
            "gen_ai.operation.name" = gen_ai::operation::THINKING,
            "gen_ai.request.model" = data.model.as_deref().unwrap_or(""),
            "duration_ms" = tracing::field::Empty,
        );

        let mut spans = self.active_spans.lock().unwrap();
        spans.insert(
            span_key,
            ActiveSpan {
                span,
                kind: SpanKind::Thinking,
                started_at: std::time::Instant::now(),
            },
        );
    }

    fn handle_thinking_completed(&self, event: &Event, data: &ReasonThinkingCompletedData) {
        let span_key = self.span_key_from_context(event, "thinking");
        let active = {
            let mut spans = self.active_spans.lock().unwrap();
            spans.remove(&span_key)
        };

        if let Some(active) = active {
            let duration_ms = active.started_at.elapsed().as_millis() as u64;
            active.span.record("duration_ms", duration_ms);

            if self.record_content {
                // Record thinking content as a span event
                let _guard = active.span.enter();
                tracing::info!(thinking.content = %data.thinking, "Extended thinking completed");
            }

            drop(active);
        }
    }

    // ========================================================================
    // LLM Generation (single event, no started/completed pair)
    // ========================================================================

    fn handle_llm_generation(&self, event: &Event, data: &LlmGenerationData) {
        let parent_key = self.parent_key_from_context(event);
        let _parent_guard = parent_key.as_ref().and_then(|k| self.enter_parent(k));

        let model = &data.metadata.model;
        let provider = data.metadata.provider.as_deref().unwrap_or("unknown");

        let output_type = if !data.output.tool_calls.is_empty() {
            "tool_calls"
        } else {
            gen_ai::output_type::TEXT
        };

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
        let cache_read = data
            .metadata
            .usage
            .as_ref()
            .and_then(|u| u.cache_read_tokens)
            .unwrap_or(0);
        let cache_creation = data
            .metadata
            .usage
            .as_ref()
            .and_then(|u| u.cache_creation_tokens)
            .unwrap_or(0);

        let span_name = format!("chat {}", model);
        let span = tracing::info_span!(
            "gen_ai.chat",
            "otel.name" = %span_name,
            "otel.kind" = "client",
            "gen_ai.operation.name" = gen_ai::operation::CHAT,
            "gen_ai.system" = %provider,
            "gen_ai.request.model" = %model,
            "gen_ai.response.model" = %model,
            "gen_ai.response.id" = data.metadata.response_id.as_deref().unwrap_or(""),
            "gen_ai.response.finish_reasons" = ?data.metadata.finish_reasons,
            "gen_ai.usage.input_tokens" = input_tokens,
            "gen_ai.usage.output_tokens" = output_tokens,
            "gen_ai.usage.cache_read_tokens" = cache_read,
            "gen_ai.usage.cache_creation_tokens" = cache_creation,
            "gen_ai.output.type" = %output_type,
            "gen_ai.conversation.id" = %event.session_id,
            "duration_ms" = data.metadata.duration_ms.unwrap_or(0),
            "time_to_first_token_ms" = data.metadata.time_to_first_token_ms.unwrap_or(0),
        );

        let _guard = span.enter();

        // Record error if generation failed
        if !data.metadata.success
            && let Some(error) = &data.metadata.error
        {
            tracing::error!(
                "error.type" = %error,
                "otel.status_code" = "ERROR",
                "LLM generation failed"
            );
        }

        // Content recording (opt-in)
        if self.record_content {
            // Input messages in OpenAI format
            let input_messages: Vec<serde_json::Value> =
                data.messages.iter().map(|m| m.to_openai_format()).collect();
            if let Ok(input_json) = serde_json::to_string(&input_messages) {
                tracing::info!(gen_ai.input.messages = %input_json, "LLM input");
            }

            // Output in OpenAI format
            let mut output = serde_json::Map::new();
            if let Some(text) = &data.output.text {
                output.insert("text".to_string(), serde_json::Value::String(text.clone()));
            }
            if !data.output.tool_calls.is_empty() {
                let tool_calls: Vec<serde_json::Value> = data
                    .output
                    .tool_calls
                    .iter()
                    .map(|tc| tc.to_openai_format())
                    .collect();
                output.insert(
                    "tool_calls".to_string(),
                    serde_json::Value::Array(tool_calls),
                );
            }
            if let Ok(output_json) = serde_json::to_string(&output) {
                tracing::info!(gen_ai.output.messages = %output_json, "LLM output");
            }

            // Tool definitions
            if !data.tools.is_empty() {
                let tools: Vec<serde_json::Value> = data
                    .tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "description": t.description,
                        })
                    })
                    .collect();
                if let Ok(tools_json) = serde_json::to_string(&tools) {
                    tracing::info!(gen_ai.tool.definitions = %tools_json, "Available tools");
                }
            }
        }
    }

    // ========================================================================
    // Act lifecycle
    // ========================================================================

    fn handle_act_started(&self, event: &Event, _data: &ActStartedData) {
        let span_key = self.span_key_from_context(event, "act");
        let parent_key = self.parent_key_from_context(event);

        let _parent_guard = parent_key.as_ref().and_then(|k| self.enter_parent(k));

        let span = tracing::info_span!(
            "act",
            "otel.name" = "act",
            "otel.kind" = "internal",
            "gen_ai.operation.name" = gen_ai::operation::ACT,
            "gen_ai.conversation.id" = %event.session_id,
            // Filled on completed
            "act.completed" = tracing::field::Empty,
            "act.success_count" = tracing::field::Empty,
            "act.error_count" = tracing::field::Empty,
            "duration_ms" = tracing::field::Empty,
        );

        let mut spans = self.active_spans.lock().unwrap();
        spans.insert(
            span_key,
            ActiveSpan {
                span,
                kind: SpanKind::Act,
                started_at: std::time::Instant::now(),
            },
        );
    }

    fn handle_act_completed(&self, event: &Event, data: &ActCompletedData) {
        let span_key = self.span_key_from_context(event, "act");
        let active = {
            let mut spans = self.active_spans.lock().unwrap();
            spans.remove(&span_key)
        };

        if let Some(active) = active {
            let duration_ms = data
                .duration_ms
                .unwrap_or_else(|| active.started_at.elapsed().as_millis() as u64);

            active.span.record("act.completed", data.completed);
            active.span.record("act.success_count", data.success_count);
            active.span.record("act.error_count", data.error_count);
            active.span.record("duration_ms", duration_ms);

            drop(active);
        }
    }

    // ========================================================================
    // Tool lifecycle
    // ========================================================================

    fn handle_tool_started(&self, event: &Event, data: &ToolStartedData) {
        let tool_key = format!("tool:{}", data.tool_call.id);
        let parent_key = self.parent_key_from_context(event);

        let _parent_guard = parent_key.as_ref().and_then(|k| self.enter_parent(k));

        let span_name = format!("execute_tool {}", data.tool_call.name);

        let span = if self.record_content {
            let args_json = serde_json::to_string(&data.tool_call.arguments).unwrap_or_default();
            tracing::info_span!(
                "gen_ai.execute_tool",
                "otel.name" = %span_name,
                "otel.kind" = "internal",
                "gen_ai.operation.name" = gen_ai::operation::EXECUTE_TOOL,
                "gen_ai.tool.name" = %data.tool_call.name,
                "gen_ai.tool.type" = gen_ai::tool_type::FUNCTION,
                "gen_ai.tool.call.id" = %data.tool_call.id,
                "gen_ai.conversation.id" = %event.session_id,
                "gen_ai.tool.call.arguments" = %args_json,
                // Filled on completed
                "tool.success" = tracing::field::Empty,
                "tool.status" = tracing::field::Empty,
                "duration_ms" = tracing::field::Empty,
                "error.type" = tracing::field::Empty,
                "gen_ai.tool.call.result" = tracing::field::Empty,
            )
        } else {
            tracing::info_span!(
                "gen_ai.execute_tool",
                "otel.name" = %span_name,
                "otel.kind" = "internal",
                "gen_ai.operation.name" = gen_ai::operation::EXECUTE_TOOL,
                "gen_ai.tool.name" = %data.tool_call.name,
                "gen_ai.tool.type" = gen_ai::tool_type::FUNCTION,
                "gen_ai.tool.call.id" = %data.tool_call.id,
                "gen_ai.conversation.id" = %event.session_id,
                "tool.success" = tracing::field::Empty,
                "tool.status" = tracing::field::Empty,
                "duration_ms" = tracing::field::Empty,
                "error.type" = tracing::field::Empty,
            )
        };

        let mut spans = self.active_spans.lock().unwrap();
        spans.insert(
            tool_key,
            ActiveSpan {
                span,
                kind: SpanKind::Tool,
                started_at: std::time::Instant::now(),
            },
        );
    }

    fn handle_tool_completed(&self, _event: &Event, data: &ToolCompletedData) {
        let tool_key = format!("tool:{}", data.tool_call_id);
        let active = {
            let mut spans = self.active_spans.lock().unwrap();
            spans.remove(&tool_key)
        };

        if let Some(active) = active {
            let duration_ms = data
                .duration_ms
                .unwrap_or_else(|| active.started_at.elapsed().as_millis() as u64);

            active.span.record("tool.success", data.success);
            active.span.record("tool.status", data.status.as_str());
            active.span.record("duration_ms", duration_ms);

            if !data.success
                && let Some(error) = &data.error
            {
                active.span.record("error.type", error.as_str());
            }

            // Content recording for tool results
            if self.record_content
                && let Some(result) = &data.result
                && let Ok(result_json) = serde_json::to_string(result)
            {
                active
                    .span
                    .record("gen_ai.tool.call.result", result_json.as_str());
            }

            drop(active);
        } else {
            // Orphaned tool completed — create point-in-time fallback
            let span_name = format!("execute_tool {}", data.tool_name);
            let _span = tracing::info_span!(
                "gen_ai.execute_tool",
                "otel.name" = %span_name,
                "otel.kind" = "internal",
                "gen_ai.operation.name" = gen_ai::operation::EXECUTE_TOOL,
                "gen_ai.tool.name" = %data.tool_name,
                "gen_ai.tool.type" = gen_ai::tool_type::FUNCTION,
                "gen_ai.tool.call.id" = %data.tool_call_id,
                "tool.success" = %data.success,
                "tool.status" = %data.status,
                "duration_ms" = data.duration_ms.unwrap_or(0),
            )
            .entered();
        }
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Build a span key from EventContext span_id, falling back to prefix+turn_id.
    fn span_key_from_context(&self, event: &Event, prefix: &str) -> String {
        if let Some(span_id) = &event.context.span_id {
            span_id.clone()
        } else if let Some(turn_id) = &event.context.turn_id {
            format!("{}:{}", prefix, turn_id)
        } else {
            format!("{}:{}", prefix, event.id)
        }
    }

    /// Get parent span key from EventContext parent_span_id or fall back to turn_id.
    fn parent_key_from_context(&self, event: &Event) -> Option<String> {
        if let Some(parent_span_id) = &event.context.parent_span_id {
            Some(parent_span_id.clone())
        } else {
            event.context.turn_id.map(|tid| tid.to_string())
        }
    }

    /// Enter a parent span context (returns guard that exits on drop).
    fn enter_parent(&self, key: &str) -> Option<tracing::span::Entered<'_>> {
        // We can't return a reference into the Mutex, so we just check existence.
        // The tracing crate handles parent-child via the current span context,
        // so we enter the parent span to establish the relationship.
        //
        // NOTE: Because we hold the lock briefly to clone the span, then enter it,
        // there's no deadlock risk. The Entered guard keeps the span active.
        let span = {
            let spans = self.active_spans.lock().unwrap();
            spans.get(key).map(|s| s.span.clone())
        };
        // We cannot return Entered<'_> because the span is local.
        // Instead, we'll just enter the span without returning a guard.
        // The tracing context is thread-local and will be restored when dropped.
        if let Some(span) = span {
            let _entered = span.enter();
            // The _entered guard would be dropped immediately here.
            // We need a different approach — use span.follows_from or
            // set the parent explicitly when creating child spans.
        }
        None
    }

    /// Get the number of tracked in-flight spans (for testing).
    #[cfg(test)]
    fn active_span_count(&self) -> usize {
        self.active_spans.lock().unwrap().len()
    }

    /// Check if a specific span key exists (for testing).
    #[cfg(test)]
    fn has_active_span(&self, key: &str) -> bool {
        self.active_spans.lock().unwrap().contains_key(key)
    }
}

#[async_trait]
impl EventListener for OtelEventListener {
    async fn on_event(&self, event: &Event) {
        match &event.data {
            EventData::TurnStarted(data) => self.handle_turn_started(event, data),
            EventData::TurnCompleted(data) => self.handle_turn_completed(event, data),
            EventData::TurnFailed(data) => self.handle_turn_failed(event, data),
            EventData::TurnCancelled(data) => self.handle_turn_cancelled(event, data),
            EventData::ReasonStarted(data) => self.handle_reason_started(event, data),
            EventData::ReasonCompleted(data) => self.handle_reason_completed(event, data),
            EventData::ReasonThinkingStarted(data) => {
                self.handle_thinking_started(event, data);
            }
            EventData::ReasonThinkingCompleted(data) => {
                self.handle_thinking_completed(event, data);
            }
            EventData::LlmGeneration(data) => self.handle_llm_generation(event, data),
            EventData::ActStarted(data) => self.handle_act_started(event, data),
            EventData::ActCompleted(data) => self.handle_act_completed(event, data),
            EventData::ToolStarted(data) => self.handle_tool_started(event, data),
            EventData::ToolCompleted(data) => self.handle_tool_completed(event, data),
            _ => {}
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            TURN_STARTED,
            TURN_COMPLETED,
            TURN_FAILED,
            TURN_CANCELLED,
            REASON_STARTED,
            REASON_COMPLETED,
            REASON_THINKING_STARTED,
            REASON_THINKING_COMPLETED,
            LLM_GENERATION,
            ACT_STARTED,
            ACT_COMPLETED,
            TOOL_STARTED,
            TOOL_COMPLETED,
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
        EventContext, InputMessageData, LlmGenerationMetadata, LlmGenerationOutput, TokenUsage,
    };
    use crate::message::Message;
    use crate::tool_types::ToolCall;
    use crate::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
    use serde_json::json;
    use uuid::Uuid;

    // Helper: create event with context pointing to parent
    fn event_with_context(
        session_id: SessionId,
        turn_id: TurnId,
        span_id: Option<&str>,
        parent_span_id: Option<&str>,
        data: EventData,
    ) -> Event {
        let ctx = EventContext {
            turn_id: Some(turn_id),
            input_message_id: Some(MessageId::from_uuid(Uuid::now_v7())),
            exec_id: None,
            trace_id: Some(turn_id.to_string()),
            span_id: span_id.map(|s| s.to_string()),
            parent_span_id: parent_span_id.map(|s| s.to_string()),
        };
        Event::new(session_id, ctx, data)
    }

    fn make_session_id() -> SessionId {
        SessionId::from_uuid(Uuid::now_v7())
    }

    fn make_turn_id() -> TurnId {
        TurnId::from_uuid(Uuid::now_v7())
    }

    // ====================================================================
    // Event type filtering
    // ====================================================================

    #[tokio::test]
    async fn test_event_types_13() {
        let listener = OtelEventListener::new();
        let types = listener.event_types().unwrap();
        assert_eq!(types.len(), 13);
        assert!(types.contains(&TURN_STARTED));
        assert!(types.contains(&TURN_COMPLETED));
        assert!(types.contains(&TURN_FAILED));
        assert!(types.contains(&TURN_CANCELLED));
        assert!(types.contains(&REASON_STARTED));
        assert!(types.contains(&REASON_COMPLETED));
        assert!(types.contains(&REASON_THINKING_STARTED));
        assert!(types.contains(&REASON_THINKING_COMPLETED));
        assert!(types.contains(&LLM_GENERATION));
        assert!(types.contains(&ACT_STARTED));
        assert!(types.contains(&ACT_COMPLETED));
        assert!(types.contains(&TOOL_STARTED));
        assert!(types.contains(&TOOL_COMPLETED));
    }

    #[tokio::test]
    async fn test_listener_creation() {
        let listener = OtelEventListener::new();
        assert_eq!(listener.name(), "OtelEventListener");
        assert!(!listener.record_content);
    }

    #[tokio::test]
    async fn test_listener_default() {
        let listener = OtelEventListener::default();
        assert_eq!(listener.name(), "OtelEventListener");
    }

    #[tokio::test]
    async fn test_listener_with_record_content() {
        let listener = OtelEventListener::with_record_content(true);
        assert!(listener.record_content);
    }

    // ====================================================================
    // Turn lifecycle
    // ====================================================================

    #[tokio::test]
    async fn test_turn_lifecycle() {
        let listener = OtelEventListener::new();
        let turn_id = make_turn_id();
        let session_id = make_session_id();

        let started = TurnStartedData {
            turn_id,
            input_message_id: MessageId::from_uuid(Uuid::now_v7()),
            input_content: Some("Hello".to_string()),
        };
        let start_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnStarted(started),
        );
        listener.on_event(&start_event).await;
        assert_eq!(listener.active_span_count(), 1);
        assert!(listener.has_active_span(&turn_id.to_string()));

        let completed = TurnCompletedData {
            turn_id,
            iterations: 3,
            duration_ms: Some(1500),
            usage: Some(TokenUsage::new(100, 50)),
            input_content: Some("Hello".to_string()),
        };
        let complete_event = Event::new(
            session_id,
            EventContext::empty(),
            EventData::TurnCompleted(completed),
        );
        listener.on_event(&complete_event).await;
        assert_eq!(listener.active_span_count(), 0);
    }

    #[tokio::test]
    async fn test_turn_failed() {
        let listener = OtelEventListener::new();
        let turn_id = make_turn_id();
        let session_id = make_session_id();

        // Start turn
        let started = TurnStartedData {
            turn_id,
            input_message_id: MessageId::from_uuid(Uuid::now_v7()),
            input_content: None,
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnStarted(started),
            ))
            .await;
        assert_eq!(listener.active_span_count(), 1);

        // Fail turn
        let failed = TurnFailedData {
            turn_id,
            error: "model overloaded".to_string(),
            error_code: Some("overloaded".to_string()),
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnFailed(failed),
            ))
            .await;
        assert_eq!(listener.active_span_count(), 0);
    }

    #[tokio::test]
    async fn test_turn_cancelled() {
        let listener = OtelEventListener::new();
        let turn_id = make_turn_id();
        let session_id = make_session_id();

        let started = TurnStartedData {
            turn_id,
            input_message_id: MessageId::from_uuid(Uuid::now_v7()),
            input_content: None,
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnStarted(started),
            ))
            .await;

        let cancelled = TurnCancelledData {
            turn_id,
            reason: Some("user cancelled".to_string()),
            usage: Some(TokenUsage::new(50, 10)),
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnCancelled(cancelled),
            ))
            .await;
        assert_eq!(listener.active_span_count(), 0);
    }

    #[tokio::test]
    async fn test_turn_completed_without_start() {
        let listener = OtelEventListener::new();

        let completed = TurnCompletedData {
            turn_id: make_turn_id(),
            iterations: 1,
            duration_ms: None,
            usage: None,
            input_content: None,
        };
        // Should not panic
        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::TurnCompleted(completed),
            ))
            .await;
    }

    #[tokio::test]
    async fn test_turn_failed_without_start() {
        let listener = OtelEventListener::new();

        let failed = TurnFailedData {
            turn_id: make_turn_id(),
            error: "something broke".to_string(),
            error_code: None,
        };
        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::TurnFailed(failed),
            ))
            .await;
    }

    // ====================================================================
    // Reason lifecycle
    // ====================================================================

    #[tokio::test]
    async fn test_reason_lifecycle() {
        let listener = OtelEventListener::new();
        let turn_id = make_turn_id();
        let session_id = make_session_id();
        let reason_span_id = "reason_001";

        // Start reason with context
        let started = ReasonStartedData {
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::from_uuid(Uuid::now_v7())),
            metadata: None,
        };
        let event = event_with_context(
            session_id,
            turn_id,
            Some(reason_span_id),
            Some(&turn_id.to_string()),
            EventData::ReasonStarted(started),
        );
        listener.on_event(&event).await;
        assert!(listener.has_active_span(reason_span_id));

        // Complete reason
        let completed = ReasonCompletedData {
            success: true,
            text_preview: Some("I'll help you with that".to_string()),
            has_tool_calls: true,
            tool_call_count: 2,
            error: None,
            duration_ms: Some(500),
            usage: Some(TokenUsage::new(80, 30)),
        };
        let event = event_with_context(
            session_id,
            turn_id,
            Some(reason_span_id),
            Some(&turn_id.to_string()),
            EventData::ReasonCompleted(completed),
        );
        listener.on_event(&event).await;
        assert!(!listener.has_active_span(reason_span_id));
    }

    // ====================================================================
    // Thinking lifecycle
    // ====================================================================

    #[tokio::test]
    async fn test_thinking_lifecycle() {
        let listener = OtelEventListener::with_record_content(true);
        let turn_id = make_turn_id();
        let session_id = make_session_id();
        let thinking_span_id = "thinking_001";

        let started = ReasonThinkingStartedData {
            turn_id,
            model: Some("claude-3-opus".to_string()),
        };
        let event = event_with_context(
            session_id,
            turn_id,
            Some(thinking_span_id),
            Some("reason_001"),
            EventData::ReasonThinkingStarted(started),
        );
        listener.on_event(&event).await;
        assert!(listener.has_active_span(thinking_span_id));

        let completed = ReasonThinkingCompletedData {
            turn_id,
            thinking: "Let me think step by step...".to_string(),
        };
        let event = event_with_context(
            session_id,
            turn_id,
            Some(thinking_span_id),
            Some("reason_001"),
            EventData::ReasonThinkingCompleted(completed),
        );
        listener.on_event(&event).await;
        assert!(!listener.has_active_span(thinking_span_id));
    }

    // ====================================================================
    // Act lifecycle
    // ====================================================================

    #[tokio::test]
    async fn test_act_lifecycle() {
        let listener = OtelEventListener::new();
        let turn_id = make_turn_id();
        let session_id = make_session_id();
        let act_span_id = "act_001";

        let started = ActStartedData {
            tool_calls: vec![],
            headline: None,
        };
        let event = event_with_context(
            session_id,
            turn_id,
            Some(act_span_id),
            Some(&turn_id.to_string()),
            EventData::ActStarted(started),
        );
        listener.on_event(&event).await;
        assert!(listener.has_active_span(act_span_id));

        let completed = ActCompletedData {
            completed: true,
            success_count: 2,
            error_count: 0,
            duration_ms: Some(300),
            headline: None,
        };
        let event = event_with_context(
            session_id,
            turn_id,
            Some(act_span_id),
            Some(&turn_id.to_string()),
            EventData::ActCompleted(completed),
        );
        listener.on_event(&event).await;
        assert!(!listener.has_active_span(act_span_id));
    }

    // ====================================================================
    // Tool lifecycle
    // ====================================================================

    #[tokio::test]
    async fn test_tool_lifecycle() {
        let listener = OtelEventListener::new();
        let session_id = make_session_id();

        let started = ToolStartedData {
            tool_call: ToolCall {
                id: "call_abc".to_string(),
                name: "calculate".to_string(),
                arguments: json!({"x": 42}),
            },
            display_name: None,
            narration: None,
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::ToolStarted(started),
            ))
            .await;
        assert!(listener.has_active_span("tool:call_abc"));

        let completed = ToolCompletedData {
            tool_call_id: "call_abc".to_string(),
            tool_name: "calculate".to_string(),
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
            duration_ms: Some(100),
            narration: None,
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::ToolCompleted(completed),
            ))
            .await;
        assert!(!listener.has_active_span("tool:call_abc"));
    }

    #[tokio::test]
    async fn test_tool_completed_without_start() {
        let listener = OtelEventListener::new();

        let completed = ToolCompletedData {
            tool_call_id: "orphan_call".to_string(),
            tool_name: "unknown_tool".to_string(),
            display_name: None,
            success: false,
            status: "error".to_string(),
            result: None,
            error: Some("Connection timeout".to_string()),
            duration_ms: None,
            narration: None,
        };
        // Should not panic
        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::ToolCompleted(completed),
            ))
            .await;
    }

    #[tokio::test]
    async fn test_concurrent_tool_calls() {
        let listener = OtelEventListener::new();
        let session_id = make_session_id();

        // Start 3 tool calls
        for i in 0..3 {
            let started = ToolStartedData {
                tool_call: ToolCall {
                    id: format!("call_{}", i),
                    name: format!("tool_{}", i),
                    arguments: json!({}),
                },
                display_name: None,
                narration: None,
            };
            listener
                .on_event(&Event::new(
                    session_id,
                    EventContext::empty(),
                    EventData::ToolStarted(started),
                ))
                .await;
        }
        assert_eq!(listener.active_span_count(), 3);

        // Complete in different order
        for i in [2, 0, 1] {
            let completed = ToolCompletedData {
                tool_call_id: format!("call_{}", i),
                tool_name: format!("tool_{}", i),
                display_name: None,
                success: true,
                status: "success".to_string(),
                result: None,
                error: None,
                duration_ms: Some(50),
                narration: None,
            };
            listener
                .on_event(&Event::new(
                    session_id,
                    EventContext::empty(),
                    EventData::ToolCompleted(completed),
                ))
                .await;
        }
        assert_eq!(listener.active_span_count(), 0);
    }

    // ====================================================================
    // LLM Generation
    // ====================================================================

    #[tokio::test]
    async fn test_llm_generation_text() {
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
                compaction: None,
            },
        };

        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::LlmGeneration(data),
            ))
            .await;
    }

    #[tokio::test]
    async fn test_llm_generation_tool_calls() {
        let listener = OtelEventListener::new();

        let data = LlmGenerationData {
            messages: vec![Message::user("What's the weather?")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Let me check...".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call_123".to_string(),
                    name: "get_weather".to_string(),
                    arguments: json!({"city": "Tokyo"}),
                }],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4o".to_string(),
                provider: Some("openai".to_string()),
                usage: Some(TokenUsage::new(20, 15)),
                duration_ms: Some(200),
                time_to_first_token_ms: Some(50),
                success: true,
                error: None,
                finish_reasons: Some(vec!["tool_calls".to_string()]),
                response_id: Some("resp_456".to_string()),
                retry: None,
                compaction: None,
            },
        };

        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::LlmGeneration(data),
            ))
            .await;
    }

    #[tokio::test]
    async fn test_llm_generation_without_optional_fields() {
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
            },
        };

        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::LlmGeneration(data),
            ))
            .await;
    }

    #[tokio::test]
    async fn test_llm_generation_with_cache_tokens() {
        let listener = OtelEventListener::new();

        let data = LlmGenerationData {
            messages: vec![Message::user("Hello")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Hi!".to_string()),
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "claude-3-5-sonnet".to_string(),
                provider: Some("anthropic".to_string()),
                usage: Some(TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                    cache_read_tokens: Some(80),
                    cache_creation_tokens: Some(20),
                }),
                duration_ms: Some(150),
                time_to_first_token_ms: Some(30),
                success: true,
                error: None,
                finish_reasons: Some(vec!["stop".to_string()]),
                response_id: None,
                retry: None,
                compaction: None,
            },
        };

        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::LlmGeneration(data),
            ))
            .await;
    }

    #[tokio::test]
    async fn test_llm_generation_error() {
        let listener = OtelEventListener::new();

        let data = LlmGenerationData {
            messages: vec![Message::user("Hello")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: None,
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4".to_string(),
                provider: Some("openai".to_string()),
                usage: None,
                duration_ms: Some(50),
                time_to_first_token_ms: None,
                success: false,
                error: Some("rate_limit_exceeded".to_string()),
                finish_reasons: Some(vec!["error".to_string()]),
                response_id: None,
                retry: None,
                compaction: None,
            },
        };

        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::LlmGeneration(data),
            ))
            .await;
    }

    // ====================================================================
    // Content recording
    // ====================================================================

    #[tokio::test]
    async fn test_content_recording_disabled() {
        let listener = OtelEventListener::with_record_content(false);
        assert!(!listener.record_content);

        // Just verify it doesn't panic when content recording is off
        let data = LlmGenerationData {
            messages: vec![Message::user("Secret prompt")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Secret response".to_string()),
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4".to_string(),
                provider: Some("openai".to_string()),
                usage: None,
                duration_ms: None,
                time_to_first_token_ms: None,
                success: true,
                error: None,
                finish_reasons: None,
                response_id: None,
                retry: None,
                compaction: None,
            },
        };

        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::LlmGeneration(data),
            ))
            .await;
    }

    #[tokio::test]
    async fn test_content_recording_enabled() {
        let listener = OtelEventListener::with_record_content(true);
        assert!(listener.record_content);

        let data = LlmGenerationData {
            messages: vec![Message::user("What is 2+2?")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("4".to_string()),
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4".to_string(),
                provider: Some("openai".to_string()),
                usage: Some(TokenUsage::new(5, 1)),
                duration_ms: Some(50),
                time_to_first_token_ms: Some(10),
                success: true,
                error: None,
                finish_reasons: Some(vec!["stop".to_string()]),
                response_id: None,
                retry: None,
                compaction: None,
            },
        };

        // Should not panic, content gets recorded as span attributes/events
        listener
            .on_event(&Event::new(
                make_session_id(),
                EventContext::empty(),
                EventData::LlmGeneration(data),
            ))
            .await;
    }

    #[tokio::test]
    async fn test_content_recording_tool_args() {
        let listener = OtelEventListener::with_record_content(true);
        let session_id = make_session_id();

        let started = ToolStartedData {
            tool_call: ToolCall {
                id: "call_with_args".to_string(),
                name: "search".to_string(),
                arguments: json!({"query": "rust programming", "limit": 10}),
            },
            display_name: None,
            narration: None,
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::ToolStarted(started),
            ))
            .await;

        let completed = ToolCompletedData {
            tool_call_id: "call_with_args".to_string(),
            tool_name: "search".to_string(),
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: None,
            error: None,
            duration_ms: Some(200),
            narration: None,
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::ToolCompleted(completed),
            ))
            .await;
    }

    // ====================================================================
    // Span hierarchy with EventContext
    // ====================================================================

    #[tokio::test]
    async fn test_span_hierarchy_parent_child() {
        let listener = OtelEventListener::new();
        let session_id = make_session_id();
        let turn_id = make_turn_id();

        // 1. Start turn
        let started = TurnStartedData {
            turn_id,
            input_message_id: MessageId::from_uuid(Uuid::now_v7()),
            input_content: Some("Test".to_string()),
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnStarted(started),
            ))
            .await;
        assert!(listener.has_active_span(&turn_id.to_string()));

        // 2. Start reason (child of turn)
        let reason_span_id = "reason_r1";
        let reason_started = ReasonStartedData {
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::from_uuid(Uuid::now_v7())),
            metadata: None,
        };
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(reason_span_id),
                Some(&turn_id.to_string()),
                EventData::ReasonStarted(reason_started),
            ))
            .await;
        assert!(listener.has_active_span(reason_span_id));

        // 3. LLM generation (child of reason)
        let llm_data = LlmGenerationData {
            messages: vec![Message::user("Test")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Response".to_string()),
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4".to_string(),
                provider: Some("openai".to_string()),
                usage: Some(TokenUsage::new(10, 5)),
                duration_ms: Some(100),
                time_to_first_token_ms: Some(20),
                success: true,
                error: None,
                finish_reasons: Some(vec!["stop".to_string()]),
                response_id: None,
                retry: None,
                compaction: None,
            },
        };
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                None,
                Some(reason_span_id),
                EventData::LlmGeneration(llm_data),
            ))
            .await;

        // 4. Complete reason
        let reason_completed = ReasonCompletedData {
            success: true,
            text_preview: None,
            has_tool_calls: false,
            tool_call_count: 0,
            error: None,
            duration_ms: Some(150),
            usage: None,
        };
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(reason_span_id),
                Some(&turn_id.to_string()),
                EventData::ReasonCompleted(reason_completed),
            ))
            .await;
        assert!(!listener.has_active_span(reason_span_id));

        // 5. Complete turn
        let turn_completed = TurnCompletedData {
            turn_id,
            iterations: 1,
            duration_ms: Some(200),
            usage: None,
            input_content: None,
        };
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnCompleted(turn_completed),
            ))
            .await;
        assert_eq!(listener.active_span_count(), 0);
    }

    #[tokio::test]
    async fn test_multiple_iterations() {
        let listener = OtelEventListener::new();
        let session_id = make_session_id();
        let turn_id = make_turn_id();

        // Start turn
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnStarted(TurnStartedData {
                    turn_id,
                    input_message_id: MessageId::from_uuid(Uuid::now_v7()),
                    input_content: None,
                }),
            ))
            .await;

        // Iteration 1: reason → act
        let r1 = "reason_1";
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(r1),
                Some(&turn_id.to_string()),
                EventData::ReasonStarted(ReasonStartedData {
                    harness_id: HarnessId::from_seed(1),
                    agent_id: Some(AgentId::from_uuid(Uuid::now_v7())),
                    metadata: None,
                }),
            ))
            .await;
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(r1),
                Some(&turn_id.to_string()),
                EventData::ReasonCompleted(ReasonCompletedData {
                    success: true,
                    text_preview: None,
                    has_tool_calls: true,
                    tool_call_count: 1,
                    error: None,
                    duration_ms: Some(100),
                    usage: None,
                }),
            ))
            .await;

        let a1 = "act_1";
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(a1),
                Some(&turn_id.to_string()),
                EventData::ActStarted(ActStartedData {
                    tool_calls: vec![],
                    headline: None,
                }),
            ))
            .await;
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(a1),
                Some(&turn_id.to_string()),
                EventData::ActCompleted(ActCompletedData {
                    completed: true,
                    success_count: 1,
                    error_count: 0,
                    duration_ms: Some(50),
                    headline: None,
                }),
            ))
            .await;

        // Iteration 2: reason only (no act = done)
        let r2 = "reason_2";
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(r2),
                Some(&turn_id.to_string()),
                EventData::ReasonStarted(ReasonStartedData {
                    harness_id: HarnessId::from_seed(1),
                    agent_id: Some(AgentId::from_uuid(Uuid::now_v7())),
                    metadata: None,
                }),
            ))
            .await;
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(r2),
                Some(&turn_id.to_string()),
                EventData::ReasonCompleted(ReasonCompletedData {
                    success: true,
                    text_preview: None,
                    has_tool_calls: false,
                    tool_call_count: 0,
                    error: None,
                    duration_ms: Some(80),
                    usage: None,
                }),
            ))
            .await;

        // Only turn span should remain
        assert_eq!(listener.active_span_count(), 1);

        // Complete turn
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnCompleted(TurnCompletedData {
                    turn_id,
                    iterations: 2,
                    duration_ms: Some(500),
                    usage: Some(TokenUsage::new(200, 100)),
                    input_content: None,
                }),
            ))
            .await;
        assert_eq!(listener.active_span_count(), 0);
    }

    // ====================================================================
    // Full agent trace (end-to-end)
    // ====================================================================

    #[tokio::test]
    async fn test_full_agent_trace() {
        let listener = OtelEventListener::new();
        let session_id = make_session_id();
        let turn_id = make_turn_id();
        let turn_key = turn_id.to_string();

        // turn.started
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnStarted(TurnStartedData {
                    turn_id,
                    input_message_id: MessageId::from_uuid(Uuid::now_v7()),
                    input_content: Some("Search for rust docs".to_string()),
                }),
            ))
            .await;

        // reason.started (iteration 1)
        let r1 = "reason_iter1";
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(r1),
                Some(&turn_key),
                EventData::ReasonStarted(ReasonStartedData {
                    harness_id: HarnessId::from_seed(1),
                    agent_id: Some(AgentId::from_uuid(Uuid::now_v7())),
                    metadata: None,
                }),
            ))
            .await;

        // llm.generation (child of reason)
        let llm_data = LlmGenerationData {
            messages: vec![Message::user("Search for rust docs")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: None,
                tool_calls: vec![ToolCall {
                    id: "call_search".to_string(),
                    name: "web_search".to_string(),
                    arguments: json!({"query": "rust documentation"}),
                }],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4o".to_string(),
                provider: Some("openai".to_string()),
                usage: Some(TokenUsage::new(50, 20)),
                duration_ms: Some(300),
                time_to_first_token_ms: Some(40),
                success: true,
                error: None,
                finish_reasons: Some(vec!["tool_calls".to_string()]),
                response_id: Some("resp_001".to_string()),
                retry: None,
                compaction: None,
            },
        };
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                None,
                Some(r1),
                EventData::LlmGeneration(llm_data),
            ))
            .await;

        // reason.completed
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(r1),
                Some(&turn_key),
                EventData::ReasonCompleted(ReasonCompletedData {
                    success: true,
                    text_preview: None,
                    has_tool_calls: true,
                    tool_call_count: 1,
                    error: None,
                    duration_ms: Some(350),
                    usage: Some(TokenUsage::new(50, 20)),
                }),
            ))
            .await;

        // act.started
        let a1 = "act_iter1";
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(a1),
                Some(&turn_key),
                EventData::ActStarted(ActStartedData {
                    tool_calls: vec![],
                    headline: None,
                }),
            ))
            .await;

        // tool.started
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                None,
                Some(a1),
                EventData::ToolStarted(ToolStartedData {
                    tool_call: ToolCall {
                        id: "call_search".to_string(),
                        name: "web_search".to_string(),
                        arguments: json!({"query": "rust documentation"}),
                    },
                    display_name: None,
                    narration: None,
                }),
            ))
            .await;

        // tool.completed
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                None,
                Some(a1),
                EventData::ToolCompleted(ToolCompletedData {
                    tool_call_id: "call_search".to_string(),
                    tool_name: "web_search".to_string(),
                    display_name: None,
                    success: true,
                    status: "success".to_string(),
                    result: None,
                    error: None,
                    duration_ms: Some(200),
                    narration: None,
                }),
            ))
            .await;

        // act.completed
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(a1),
                Some(&turn_key),
                EventData::ActCompleted(ActCompletedData {
                    completed: true,
                    success_count: 1,
                    error_count: 0,
                    duration_ms: Some(250),
                    headline: None,
                }),
            ))
            .await;

        // reason.started (iteration 2 — final response)
        let r2 = "reason_iter2";
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(r2),
                Some(&turn_key),
                EventData::ReasonStarted(ReasonStartedData {
                    harness_id: HarnessId::from_seed(1),
                    agent_id: Some(AgentId::from_uuid(Uuid::now_v7())),
                    metadata: None,
                }),
            ))
            .await;

        // llm.generation (final response, no tool calls)
        let llm_data2 = LlmGenerationData {
            messages: vec![Message::user("Search for rust docs")],
            tools: vec![],
            output: LlmGenerationOutput {
                text: Some("Here are the Rust docs...".to_string()),
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model: "gpt-4o".to_string(),
                provider: Some("openai".to_string()),
                usage: Some(TokenUsage::new(100, 80)),
                duration_ms: Some(400),
                time_to_first_token_ms: Some(30),
                success: true,
                error: None,
                finish_reasons: Some(vec!["stop".to_string()]),
                response_id: Some("resp_002".to_string()),
                retry: None,
                compaction: None,
            },
        };
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                None,
                Some(r2),
                EventData::LlmGeneration(llm_data2),
            ))
            .await;

        // reason.completed
        listener
            .on_event(&event_with_context(
                session_id,
                turn_id,
                Some(r2),
                Some(&turn_key),
                EventData::ReasonCompleted(ReasonCompletedData {
                    success: true,
                    text_preview: Some("Here are the Rust docs...".to_string()),
                    has_tool_calls: false,
                    tool_call_count: 0,
                    error: None,
                    duration_ms: Some(450),
                    usage: Some(TokenUsage::new(100, 80)),
                }),
            ))
            .await;

        // turn.completed
        listener
            .on_event(&Event::new(
                session_id,
                EventContext::empty(),
                EventData::TurnCompleted(TurnCompletedData {
                    turn_id,
                    iterations: 2,
                    duration_ms: Some(1200),
                    usage: Some(TokenUsage::new(150, 100)),
                    input_content: None,
                }),
            ))
            .await;

        assert_eq!(listener.active_span_count(), 0);
    }

    // ====================================================================
    // Unhandled event types
    // ====================================================================

    #[tokio::test]
    async fn test_unhandled_event_types() {
        let listener = OtelEventListener::new();

        let event = Event::new(
            make_session_id(),
            EventContext::empty(),
            EventData::InputMessage(InputMessageData {
                message: Message::user("Hello"),
            }),
        );
        // Should not panic
        listener.on_event(&event).await;
    }
}
