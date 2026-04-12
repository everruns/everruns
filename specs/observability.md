# Observability Providers

Everruns supports multiple observability backends via the `EventListener` pattern. Each provider receives the same agentic loop events and maps them to provider-specific span/trace formats.

For the canonical event type registry and EventContext definition, see `specs/events.md` and `crates/core/src/events.rs`.

## Common Architecture

### Span Hierarchy

All providers trace the same agentic execution model. Events form a tree:

```
agent turn (root)                          # turn.started → turn.completed
├── reason (iteration 1)                   # reason.started → reason.completed
│   ├── thinking (if extended thinking)    # reason.thinking.started → completed
│   └── llm call (LLM API call)           # llm.generation
├── act (iteration 1)                      # act.started → act.completed
│   ├── tool call                          # tool.started → tool.completed
│   └── tool call                          # tool.started → tool.completed
├── reason (iteration 2)
│   └── llm call
├── act (iteration 2)
│   └── tool call
├── reason (iteration 3)
│   └── llm call
└── (no act — turn complete)
```

### Started/Completed Merging

Events with both started and completed phases share the same `span_id`, so providers merge them into a single span with duration:

- `reason.started` / `reason.completed` share `reason_span_id`
- `act.started` / `act.completed` share `act_span_id`
- `tool.started` / `tool.completed` share `tool_span_id`
- `turn.started` / `turn.completed` share `turn_id`

### Trace Correlation

All events within a single turn share the same `turn_id` as their trace root. See `specs/events.md` for EventContext fields (`trace_id`, `span_id`, `parent_span_id`). Parent-child relationships:

- reason events: `parent_span_id` = `turn_id`
- thinking events: `parent_span_id` = `reason_span_id`
- llm.generation: `parent_span_id` = `reason_span_id`
- act events: `parent_span_id` = `turn_id`
- tool events: `parent_span_id` = `act_span_id`

---

## OpenTelemetry

Full-featured OpenTelemetry integration following the [Gen-AI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/).

### References

- [Gen-AI Agent Spans](https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-agent-spans/)
- [Gen-AI Client Spans](https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-spans/)
- [Gen-AI Events](https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-events/)
- [Attribute Registry](https://opentelemetry.io/docs/specs/semconv/registry/attributes/gen-ai/)

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

### OTel Span Mapping

| Event Type | OTel Span Name | Span Kind | Semantic Convention |
|------------|---------------|-----------|---------------------|
| turn | `invoke_agent {agent_name}` | `INTERNAL` | `gen_ai.invoke_agent` |
| reason | `reason` | `INTERNAL` | custom (phase span) |
| thinking | `thinking` | `INTERNAL` | custom (phase span) |
| act | `act` | `INTERNAL` | custom (phase span) |
| llm.generation | `chat {model}` | `CLIENT` | `gen_ai.chat` |
| tool | `execute_tool {name}` | `INTERNAL` | `gen_ai.execute_tool` |

### Span Attributes by Type

For the complete attribute tables per span type (invoke_agent, chat, execute_tool, reason, act, thinking), see `crates/core/src/observation/otel.rs`. Key attributes follow the OTel Gen-AI semantic conventions:

- **All spans**: `gen_ai.operation.name`, `gen_ai.conversation.id`, `duration_ms`
- **invoke_agent**: `gen_ai.agent.id`, `gen_ai.agent.name`, `turn.id`, usage tokens
- **chat**: `gen_ai.request.model`, `gen_ai.response.model`, `gen_ai.usage.*`, cache tokens
- **execute_tool**: `gen_ai.tool.name`, `gen_ai.tool.call.id`, `tool.success`
- **Content (opt-in)**: `gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.tool.call.arguments/result`

### Trace Context Propagation

Spans use the `tracing` crate's native parent-child mechanism. The `OtelEventListener` maintains a `HashMap<String, tracing::Span>` to track active spans. When a child event arrives, the listener looks up the parent span, enters its context, and creates the child span (inheriting the parent). See `crates/core/src/observation/otel.rs` for the full implementation.

### Content Recording

Disabled by default for privacy. Enable with:
```bash
export OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true
```

Messages are converted to OpenAI-compatible format using `Message::to_openai_format()` before recording.

### Design Decisions

- **Real duration spans** (not point-in-time): Proper span lifecycle (create on started, end on completed) so trace viewers show actual timing in waterfall views.
- **Native tracing span nesting**: Uses `tracing` crate's native parent-child mechanism rather than manual ID linking. The `tracing-opentelemetry` bridge translates this into proper OTel trace context.
- **Standard env var for content**: `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` (the OTel standard) with `OTEL_RECORD_CONTENT` as legacy alias.
- **Cache token attributes**: `gen_ai.usage.cache_read_tokens` and `gen_ai.usage.cache_creation_tokens` included — not in standard yet but essential for cost monitoring.
- **Phase spans**: reason/act/thinking spans aren't in OTel Gen-AI spec but provide the hierarchy that makes traces useful for debugging agent behavior.

### Implementation

| File | Purpose |
|------|---------|
| `crates/core/src/observation/otel.rs` | `OtelEventListener` — all span creation/lifecycle |
| `crates/core/src/telemetry.rs` | Gen-AI semantic conventions, config, init |
| `crates/server/src/main.rs` | Listener registration |

---

## Braintrust

Integration with [Braintrust](https://www.braintrust.dev/) for LLM observability, evaluation, and logging.

### References

- [Braintrust Documentation](https://www.braintrust.dev/docs)
- [Insert Project Logs API](https://www.braintrust.dev/docs/api-reference/logs/insert-project-logs-events)
- [OpenAI Agents Integration](https://github.com/braintrustdata/braintrust-sdk/tree/main/integrations/openai-agents-js) (reference)

### Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BRAINTRUST_API_KEY` | Yes (to enable) | - | API key from Braintrust organization settings |
| `BRAINTRUST_PROJECT_NAME` | No | `My Project` | Project name (resolved to ID at startup) |
| `BRAINTRUST_PROJECT_ID` | No | - | Direct project UUID (skips name resolution) |
| `BRAINTRUST_API_URL` | No | `https://api.braintrust.dev` | API base URL |

### Braintrust Span Mapping

| Event Type | `span_attributes.type` | `span_attributes.name` |
|------------|------------------------|------------------------|
| Turn events | `task` | `"agent turn"` |
| Reason events | `task` | `"reason"` |
| Thinking events | `task` | `"thinking"` |
| Act events | `task` | `"act"` |
| LLM generation | `llm` | `"chat {model}"` |
| Tool events | `tool` | `"tool {name}"` |

### Data Mapping

For the complete field-by-field mapping (LLM generation, tool events, thinking events), see `crates/core/src/observation/braintrust.rs`. Key mappings:

- **Token usage**: `metadata.usage.*` → `metrics.prompt_tokens`, `metrics.completion_tokens`, `metrics.tokens`
- **Cache tokens**: `metadata.usage.cache_read_tokens` → `metrics.cache_read_tokens`
- **Timing**: `metadata.duration_ms` → `metrics.start`/`metrics.end`; `metadata.time_to_first_token_ms` → `metrics.time_to_first_token` (seconds)
- **Messages**: Converted to OpenAI format via `Message::to_openai_format()`

### Design Decisions

- **Full agentic loop tracing**: Traces turns, reason/act phases, LLM calls, and tool executions — matches Braintrust's OpenAI Agents integration feature parity.
- **Project name as primary config**: `BRAINTRUST_PROJECT_NAME` with default "My Project" matches the JS SDK pattern. Name-to-ID resolution at startup.
- **Async event delivery**: Events sent via `tokio::spawn` to avoid blocking main event processing. Fire-and-forget — failed deliveries are logged, not retried.
- **EventListener pattern**: Observability is orthogonal to LLM execution — listens to completed events, consistent with OtelEventListener.
- **Blocking HTTP at startup**: `tokio::task::block_in_place` for one-time project name resolution. Simpler than async init.

### Implementation

- **File**: `crates/core/src/observation/braintrust.rs`
- **Registration**: `crates/server/src/main.rs` (event listener setup)
- **Configuration**: `docs/sre/environment-variables.md`
- **Format conversion**: `crates/core/src/message.rs` (`Message::to_openai_format()`)

### API Endpoints Used

**Project Resolution:**
```
GET /v1/project?project_name={name}
Authorization: Bearer {api_key}
```

**Insert Logs:**
```
POST /v1/project_logs/{project_id}/insert
Authorization: Bearer {api_key}
Content-Type: application/json
```

See `crates/core/src/observation/braintrust.rs` for the full request/response format.

---

## Test Coverage

### OTel Tests (in otel.rs)

Tests verify span creation, attributes, hierarchy, lifecycle, and content recording. Key scenarios: all 13 event types, turn lifecycle (started/completed/failed/cancelled), span hierarchy parent-child, multiple iterations, concurrent tool calls, orphaned completed events, and end-to-end full agent traces.

### Braintrust Tests

Tests verify event relationship correctness:

1. **turn_id propagation**: All events within a turn share the same turn_id
2. **trace_id consistency**: All child events have trace_id = turn_id
3. **parent_span_id correctness**: Each event type references the correct parent
4. **span_id sharing**: started/completed pairs share the same span_id

Both test suites must cover dev_worker (DEV_MODE) and durable_worker (Full mode) execution paths.
