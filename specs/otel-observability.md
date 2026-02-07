# OpenTelemetry Observability

Full-featured OpenTelemetry integration following the Gen-AI semantic conventions.

## Abstract

Everruns provides native OpenTelemetry (OTel) tracing for the complete agentic execution lifecycle. All 13 event types produce properly-nested spans with parent-child relationships, enabling full trace visualization in any OTel-compatible backend (Jaeger, Grafana Tempo, Datadog, Honeycomb, etc.). Content recording (prompts, completions, tool arguments) is opt-in via standard OTel environment variables.

## References

- **OTel Gen-AI Semantic Conventions**: https://opentelemetry.io/docs/specs/semconv/gen-ai/
- **Gen-AI Agent Spans**: https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-agent-spans/
- **Gen-AI Client Spans**: https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-spans/
- **Gen-AI Events**: https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-events/
- **Attribute Registry**: https://opentelemetry.io/docs/specs/semconv/registry/attributes/gen-ai/
- **Internal Braintrust Spec**: `specs/braintrust-integration.md` (reference for event hierarchy)

## Requirements

### Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Yes (to enable) | - | OTLP endpoint (e.g., `http://localhost:4318`) |
| `OTEL_SERVICE_NAME` | No | `everruns` | Service name in traces |
| `OTEL_SERVICE_VERSION` | No | crate version | Service version |
| `OTEL_ENVIRONMENT` | No | - | Deployment environment |
| `OTEL_SDK_DISABLED` | No | `false` | Disable OTel entirely |
| `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` | No | `false` | Record prompts, completions, tool args/results |

**Note:** `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` is the standard OTel env var for content capture. The existing `OTEL_RECORD_CONTENT` is kept as a legacy alias.

### Event Types

The integration traces the full agentic loop — same 13 event types as Braintrust:

| Event Type | OTel Span Name | Span Kind | Semantic Convention |
|------------|---------------|-----------|---------------------|
| `turn.started` | `invoke_agent {agent_name}` | `INTERNAL` | `gen_ai.invoke_agent` |
| `turn.completed` | (merges with started) | - | - |
| `turn.failed` | (merges with started, sets error) | - | - |
| `turn.cancelled` | (merges with started, sets error) | - | - |
| `reason.started` | `reason` | `INTERNAL` | custom (phase span) |
| `reason.completed` | (merges with started) | - | - |
| `reason.thinking.started` | `thinking` | `INTERNAL` | custom (phase span) |
| `reason.thinking.completed` | (merges with started) | - | - |
| `act.started` | `act` | `INTERNAL` | custom (phase span) |
| `act.completed` | (merges with started) | - | - |
| `llm.generation` | `chat {model}` | `CLIENT` | `gen_ai.chat` |
| `tool.started` | `execute_tool {name}` | `INTERNAL` | `gen_ai.execute_tool` |
| `tool.completed` | (merges with started) | - | - |

### Span Hierarchy

Traces form a tree matching the agentic execution model. Parent-child relationships use OTel's native `tracing` span nesting (not manual ID linking).

```
invoke_agent {agent_name} (root)         # turn.started → turn.completed
├── reason (iteration 1)                  # reason.started → reason.completed
│   ├── thinking (if extended thinking)   # reason.thinking.started → completed
│   └── chat {model} (LLM call)          # llm.generation
├── act (iteration 1)                     # act.started → act.completed
│   ├── execute_tool {name}              # tool.started → tool.completed
│   └── execute_tool {name}              # tool.started → tool.completed
├── reason (iteration 2)
│   └── chat {model}
├── act (iteration 2)
│   └── execute_tool {name}
├── reason (iteration 3)
│   └── chat {model}
└── (no act — turn complete)
```

### Span Lifecycle (Started/Completed Merging)

Unlike the previous point-in-time span approach, spans now have proper duration:

1. **On `*.started` event**: Create and enter a `tracing::Span`. Store it in a map keyed by the span's correlation ID (turn_id, span_id from EventContext, or tool_call_id).
2. **On `*.completed` event**: Look up the stored span, record completion attributes as span events/attributes, then drop the span (ending it).
3. **On `turn.failed` / `turn.cancelled`**: Look up the turn span, record `error.type` and `otel.status_code = ERROR`, then drop.

This produces real duration spans in the trace viewer.

### Span Attributes by Type

#### invoke_agent (Turn) Span

| Attribute | Source | Required |
|-----------|--------|----------|
| `gen_ai.operation.name` | `"invoke_agent"` | Yes |
| `gen_ai.provider.name` | from agent config | Yes |
| `gen_ai.agent.id` | `event.context.agent_id` or agent_id | Recommended |
| `gen_ai.agent.name` | agent name | Recommended |
| `gen_ai.conversation.id` | `event.session_id` | Recommended |
| `turn.id` | `data.turn_id` | Yes |
| `turn.iterations` | `data.iterations` (on completed) | Yes |
| `gen_ai.usage.input_tokens` | `data.usage.input_tokens` (on completed) | Recommended |
| `gen_ai.usage.output_tokens` | `data.usage.output_tokens` (on completed) | Recommended |
| `error.type` | error string (on failed/cancelled) | Conditional |
| `otel.status_code` | `ERROR` on failure | Conditional |
| `otel.status_description` | error message | Conditional |
| `duration_ms` | computed from started→completed | Recommended |

**Content (opt-in):**
| Attribute | Source | Notes |
|-----------|--------|-------|
| `gen_ai.input.messages` | `data.input_content` | User input for this turn |

#### chat (LLM Generation) Span

| Attribute | Source | Required |
|-----------|--------|----------|
| `gen_ai.operation.name` | `"chat"` | Yes |
| `gen_ai.provider.name` | `metadata.provider` | Yes |
| `gen_ai.system` | `metadata.provider` | Yes (legacy alias) |
| `gen_ai.request.model` | `metadata.model` | Conditional |
| `gen_ai.response.model` | `metadata.model` | Recommended |
| `gen_ai.response.id` | `metadata.response_id` | Recommended |
| `gen_ai.response.finish_reasons` | `metadata.finish_reasons` | Recommended |
| `gen_ai.usage.input_tokens` | `metadata.usage.input_tokens` | Recommended |
| `gen_ai.usage.output_tokens` | `metadata.usage.output_tokens` | Recommended |
| `gen_ai.usage.cache_read_tokens` | `metadata.usage.cache_read_tokens` | Recommended |
| `gen_ai.usage.cache_creation_tokens` | `metadata.usage.cache_creation_tokens` | Recommended |
| `gen_ai.output.type` | `"tool_calls"` or `"text"` | Conditional |
| `gen_ai.conversation.id` | `event.session_id` | Recommended |
| `duration_ms` | `metadata.duration_ms` | Recommended |
| `time_to_first_token_ms` | `metadata.time_to_first_token_ms` | Recommended |
| `error.type` | `metadata.error` (when !success) | Conditional |

**Content (opt-in):**
| Attribute | Source | Notes |
|-----------|--------|-------|
| `gen_ai.input.messages` | `data.messages` (OpenAI format) | Full chat history |
| `gen_ai.output.messages` | `data.output` (OpenAI format) | Model response |
| `gen_ai.tool.definitions` | `data.tools` | Available tools |

#### execute_tool Span

| Attribute | Source | Required |
|-----------|--------|----------|
| `gen_ai.operation.name` | `"execute_tool"` | Yes |
| `gen_ai.tool.name` | `data.tool_name` | Recommended |
| `gen_ai.tool.type` | `"function"` | Recommended |
| `gen_ai.tool.call.id` | `data.tool_call_id` | Recommended |
| `tool.success` | `data.success` | Yes |
| `tool.status` | `data.status` | Yes |
| `gen_ai.conversation.id` | `event.session_id` | Recommended |
| `duration_ms` | from started→completed | Recommended |
| `error.type` | `data.error` (when !success) | Conditional |

**Content (opt-in):**
| Attribute | Source | Notes |
|-----------|--------|-------|
| `gen_ai.tool.call.arguments` | `data.tool_call.arguments` (on started) | Tool input |
| `gen_ai.tool.call.result` | `data.result` (on completed) | Tool output |

#### reason Phase Span

| Attribute | Source | Required |
|-----------|--------|----------|
| `gen_ai.operation.name` | `"reason"` | Yes |
| `reason.success` | `data.success` (on completed) | Yes |
| `reason.has_tool_calls` | `data.has_tool_calls` | Yes |
| `reason.tool_call_count` | `data.tool_call_count` | Yes |
| `gen_ai.usage.input_tokens` | `data.usage` (on completed) | Recommended |
| `gen_ai.usage.output_tokens` | `data.usage` (on completed) | Recommended |
| `duration_ms` | `data.duration_ms` or computed | Recommended |
| `error.type` | `data.error` (when !success) | Conditional |

**Content (opt-in):**
| Attribute | Source | Notes |
|-----------|--------|-------|
| `reason.text_preview` | `data.text_preview` | Truncated LLM output preview |

#### act Phase Span

| Attribute | Source | Required |
|-----------|--------|----------|
| `gen_ai.operation.name` | `"act"` | Yes |
| `act.completed` | `data.completed` | Yes |
| `act.success_count` | `data.success_count` | Yes |
| `act.error_count` | `data.error_count` | Yes |
| `duration_ms` | `data.duration_ms` or computed | Recommended |

#### thinking Phase Span

| Attribute | Source | Required |
|-----------|--------|----------|
| `gen_ai.operation.name` | `"thinking"` | Yes |
| `gen_ai.request.model` | `data.model` | Recommended |

**Content (opt-in):**
| Attribute | Source | Notes |
|-----------|--------|-------|
| `thinking.content` | `data.thinking` (on completed) | Full thinking text |

### Trace Context Propagation

Spans use the `tracing` crate's native parent-child mechanism. The `EventContext` fields (`trace_id`, `span_id`, `parent_span_id`) are used to correlate started/completed pairs and establish parent-child relationships:

1. **Root span** (turn): Created on `turn.started`, stored by `turn_id`
2. **Child spans** (reason, act): Created inside the turn span context using `parent_span_id` from `EventContext` pointing to `turn_id`
3. **Grandchild spans** (chat, tool, thinking): Created inside their parent phase span using `parent_span_id` from `EventContext`

The `OtelEventListener` maintains a `HashMap<String, tracing::Span>` to track active spans. When a child event arrives, the listener:
1. Looks up the parent span by `parent_span_id`
2. Enters the parent span context
3. Creates the child span (inheriting the parent)
4. Stores the child span for later completion

### Content Recording

Content recording is **disabled by default** for privacy. When enabled, prompts, completions, tool arguments, and tool results are recorded as span attributes following the OTel Gen-AI semantic conventions.

**Enabling:**
```bash
export OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true
```

**What gets recorded:**

| Span Type | Attributes | Format |
|-----------|-----------|--------|
| chat | `gen_ai.input.messages`, `gen_ai.output.messages` | OpenAI-compatible JSON |
| chat | `gen_ai.system_instructions`, `gen_ai.tool.definitions` | JSON |
| execute_tool | `gen_ai.tool.call.arguments`, `gen_ai.tool.call.result` | JSON |
| thinking | `thinking.content` | Plain text |
| reason | `reason.text_preview` | Plain text (truncated) |
| invoke_agent | `gen_ai.input.messages` | Plain text |

Messages are converted to OpenAI-compatible format using `Message::to_openai_format()` before recording, ensuring consistent representation across backends.

## Design Decisions

### Real Duration Spans (not point-in-time)

**Decision**: Use proper span lifecycle (create on started, end on completed) instead of point-in-time spans.

**Rationale**: Point-in-time spans appear as zero-duration marks in trace viewers, losing the most useful information — how long each phase took. Real spans show actual timing in waterfall views.

### Native tracing Span Nesting

**Decision**: Use `tracing` crate's native parent-child span mechanism rather than manual trace_id/span_id attributes.

**Rationale**: The `tracing-opentelemetry` bridge automatically translates parent-child relationships into proper OTel trace context. This is more reliable than manual ID linking and produces correct traces in all OTel backends without custom configuration.

### Standard OTel Environment Variable for Content

**Decision**: Use `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` (the OTel standard) with `OTEL_RECORD_CONTENT` as a legacy alias.

**Rationale**: The standard variable is what OTel documentation recommends. Users familiar with OTel will expect it. The alias maintains backward compatibility.

### Cache Token Attributes

**Decision**: Include `gen_ai.usage.cache_read_tokens` and `gen_ai.usage.cache_creation_tokens` as span attributes.

**Rationale**: Prompt caching (Anthropic) and predicted outputs (OpenAI) significantly affect cost and latency. These metrics are essential for production monitoring. They are not in the standard Gen-AI semantic conventions yet but are widely used in practice.

### Phase Spans (reason, act, thinking)

**Decision**: Create spans for agentic loop phases even though they aren't in the OTel Gen-AI spec.

**Rationale**: The Gen-AI spec covers basic LLM calls and tool execution. The reason/act/thinking phases are specific to the agentic loop architecture. Without them, traces show a flat list of LLM calls and tool executions with no structure. The phase spans provide the hierarchy that makes traces actually useful for debugging agent behavior.

## Implementation

### Files

| File | Purpose |
|------|---------|
| `crates/core/src/observation/otel.rs` | `OtelEventListener` — all span creation/lifecycle |
| `crates/core/src/telemetry.rs` | Gen-AI semantic conventions, config, init |
| `crates/server/src/main.rs` | Listener registration |

### Internal Span Tracking

```rust
struct OtelEventListener {
    /// Active spans keyed by correlation ID (turn_id, span_id, tool_call_id)
    active_spans: Mutex<HashMap<String, ActiveSpanInfo>>,
    /// Whether to record content (prompts, completions, tool args)
    record_content: bool,
}

struct ActiveSpanInfo {
    span: tracing::Span,
    started_at: Instant,
    span_kind: SpanKind, // Turn, Reason, Act, Thinking, Tool
}
```

### Event Handling Flow

```
on_event(turn.started)      → create root span, store in active_spans[turn_id]
on_event(reason.started)    → enter parent(turn), create reason span, store
on_event(thinking.started)  → enter parent(reason), create thinking span, store
on_event(thinking.completed)→ lookup span, record attrs, drop
on_event(llm.generation)    → enter parent(reason), create+drop chat span (instant)
on_event(reason.completed)  → lookup span, record attrs, drop
on_event(act.started)       → enter parent(turn), create act span, store
on_event(tool.started)      → enter parent(act), create tool span, store
on_event(tool.completed)    → lookup span, record attrs, drop
on_event(act.completed)     → lookup span, record attrs, drop
on_event(turn.completed)    → lookup span, record attrs, drop
on_event(turn.failed)       → lookup span, set error, drop
on_event(turn.cancelled)    → lookup span, set cancelled, drop
```

Note: `llm.generation` is a single event (no started/completed pair), so it creates a span with the reported `duration_ms` as an attribute and drops immediately. The span still has proper parent-child nesting because it's created within the reason span's context.

## Test Coverage Requirements

### Unit Tests (in otel.rs)

Tests must verify span creation, attributes, hierarchy, and lifecycle:

| Test | Description |
|------|-------------|
| `test_event_types_13` | Listener subscribes to all 13 event types |
| `test_turn_lifecycle` | turn.started creates span, turn.completed drops it |
| `test_turn_failed` | turn.failed sets error attributes |
| `test_turn_cancelled` | turn.cancelled sets error attributes |
| `test_reason_lifecycle` | reason.started/completed with attributes |
| `test_act_lifecycle` | act.started/completed with attributes |
| `test_thinking_lifecycle` | thinking.started/completed with content |
| `test_tool_lifecycle` | tool.started/completed with attributes |
| `test_llm_generation_text` | LLM generation with text output |
| `test_llm_generation_tool_calls` | LLM generation producing tool calls |
| `test_llm_generation_without_optional_fields` | Graceful handling of missing fields |
| `test_llm_generation_with_cache_tokens` | Cache read/creation token attrs |
| `test_llm_generation_error` | Failed LLM call sets error.type |
| `test_span_hierarchy_parent_child` | Child spans reference correct parents |
| `test_multiple_iterations` | Multiple reason/act cycles share turn root |
| `test_concurrent_tool_calls` | Parallel tools under same act span |
| `test_orphaned_completed` | Completed without started doesn't panic |
| `test_content_recording_disabled` | No content attrs when disabled |
| `test_content_recording_enabled` | Content attrs present when enabled |
| `test_content_recording_llm_messages` | Input/output messages in OpenAI format |
| `test_content_recording_tool_args` | Tool arguments and results |
| `test_full_agent_trace` | End-to-end: turn→reason→chat→act→tool→reason→chat→complete |

### Smoke Test (with Jaeger)

Verify real traces appear correctly in Jaeger:

1. Start system with `just start-all` (includes Jaeger)
2. Create agent, send message, wait for completion
3. Query Jaeger API for traces by service name
4. Assert: trace has correct span count, hierarchy, and attributes
5. Assert: all spans have duration > 0 (not point-in-time)
6. Assert: parent-child relationships form expected tree

## Migration from Current Implementation

The current `OtelEventListener` handles 5 event types with point-in-time spans. The new implementation:

1. Expands to 13 event types (matching Braintrust)
2. Switches from point-in-time to duration spans
3. Adds proper parent-child nesting via tracing span context
4. Adds content recording support
5. Adds cache token metrics
6. Adds error/cancellation handling

This is a backward-compatible change — existing traces will simply have more spans and better structure. No configuration changes required.
