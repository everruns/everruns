---
type: Specification
title: "Observability Providers"
description: "Observability providers."
tags:
  - everruns
  - operations
---
# Observability Providers

Everruns supports multiple observability backends via the `EventListener` pattern. Each provider receives the same agentic loop events and maps them to provider-specific span/trace formats.

For the canonical event type registry and EventContext definition, see `knowledge/execution/events.md` and `crates/core/src/events.rs`.

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

All events within a single turn share the same `turn_id` as their trace root. See `knowledge/execution/events.md` for EventContext fields (`trace_id`, `span_id`, `parent_span_id`). Parent-child relationships:

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

For the complete attribute tables per span type (invoke_agent, chat, execute_tool, reason, act, thinking), see `crates/observability/src/otel.rs`. Key attributes follow the OTel Gen-AI semantic conventions:

- **All spans**: `gen_ai.operation.name`, `gen_ai.conversation.id`, `duration_ms`
- **invoke_agent**: `gen_ai.agent.id`, `gen_ai.agent.name`, `turn.id`, usage tokens
- **chat**: `gen_ai.request.model`, `gen_ai.response.model`, `gen_ai.usage.*`, cache tokens
- **execute_tool**: `gen_ai.tool.name`, `gen_ai.tool.call.id`, `tool.success`
- **Content (opt-in)**: `gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.tool.call.arguments/result`

### Trace Context Propagation

Spans use the `tracing` crate's native parent-child mechanism. The `OtelEventListener` maintains a `HashMap<String, tracing::Span>` to track active spans. When a child event arrives, the listener looks up the parent span, enters its context, and creates the child span (inheriting the parent). See `crates/observability/src/otel.rs` for the full implementation.

HTTP-layer correlation identifiers (`request_id`, `session_id`) are recorded as span fields on every HTTP request span and propagated into durable execution. See [`knowledge/operations/correlation-ids.md`](correlation-ids.md) for the full contract.

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
- **Cache token attributes**: `gen_ai.usage.cache_read_tokens` and `gen_ai.usage.cache_creation_tokens` included, not in standard yet but essential for cost monitoring.
- **Phase spans**: reason/act/thinking spans aren't in OTel Gen-AI spec but provide the hierarchy that makes traces useful for debugging agent behavior.

### Implementation

| File | Purpose |
|------|---------|
| `crates/observability/src/otel.rs` | `OtelEventListener`, all span creation/lifecycle |
| `crates/observability/src/telemetry.rs` | OTLP exporter wiring, tracing-subscriber layers, config, init |
| `crates/core/src/telemetry.rs` | Neutral gen-AI semantic conventions and span-name helpers |
| `crates/server/src/main.rs` | Listener registration |

Ownership boundary (EVE-876): core holds only the neutral observability contracts, the `EventListener` trait, event types, and gen-AI span conventions. `everruns-observability` owns telemetry initialization, exporter dependencies, and the `CompositeEventListener` fan-out. The `check-observability-isolation.sh` guard (pre-push + CI) keeps exporter crates out of core and out of Framework/provider dependency trees, so default Framework builds stay offline.

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
| `BRAINTRUST_ENABLED` | No | enabled when `BRAINTRUST_API_KEY` is set | Explicit Braintrust on/off switch |
| `BRAINTRUST_API_KEY` | Yes (to enable) | - | API key from Braintrust organization settings |
| `BRAINTRUST_PROJECT_NAME` | No | `My Project` | Project name (resolved to ID at startup) |
| `BRAINTRUST_PROJECT_ID` | No | - | Direct project UUID (skips name resolution) |
| `BRAINTRUST_API_URL` | No | `https://api.braintrust.dev` | API base URL |
| `BRAINTRUST_QUEUE_CAPACITY` | No | `1024` | Max in-memory events buffered before new exports are dropped |
| `BRAINTRUST_MAX_BATCH_SIZE` | No | `50` | Max events per `project_logs.insert` request |
| `BRAINTRUST_FLUSH_INTERVAL_MS` | No | `500` | Max wait before flushing a partial batch |
| `BRAINTRUST_REQUEST_TIMEOUT_MS` | No | `10000` | Per-request timeout for Braintrust insert calls |
| `BRAINTRUST_MAX_RETRIES` | No | `3` | Retry attempts for `429`, `5xx`, and timeout/connect errors |
| `BRAINTRUST_RETRY_BASE_DELAY_MS` | No | `250` | Base retry backoff before jitter |
| `BRAINTRUST_RETRY_MAX_DELAY_MS` | No | `5000` | Retry backoff cap |
| `BRAINTRUST_RECORD_CONTENT` | No | `false` | Record raw turn/LLM text content |
| `BRAINTRUST_RECORD_THINKING` | No | `none` | Extended thinking export mode: `none`, `summary`, `full` |
| `BRAINTRUST_TOOL_ARGS_MODE` | No | `redacted` | Tool argument export mode: `full`, `redacted`, `none` |
| `BRAINTRUST_TOOL_RESULTS_MODE` | No | `summary` | Tool result export mode: `full`, `summary`, `redacted`, `none` |
| `BRAINTRUST_DEBUG_PAYLOADS` | No | `false` | Emit full outbound Braintrust payloads to local debug logs |

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

For the complete field-by-field mapping (LLM generation, tool events, thinking events), see `crates/observability/src/braintrust.rs`. Key mappings:

- **Token usage**: `metadata.usage.*` → `metrics.prompt_tokens`, `metrics.completion_tokens`, `metrics.tokens`
- **Cache tokens**: `metadata.usage.cache_read_tokens` → `metrics.cache_read_tokens`
- **Timing**: `metadata.duration_ms` → `metrics.start`/`metrics.end`; `metadata.time_to_first_token_ms` → `metrics.time_to_first_token` (seconds)
- **Messages**: Converted to Braintrust/OpenAI-compatible payloads only when `BRAINTRUST_RECORD_CONTENT=true`; otherwise the exporter sends structural summaries without raw text previews
- **Tool args/results**: Controlled independently via `BRAINTRUST_TOOL_ARGS_MODE` and `BRAINTRUST_TOOL_RESULTS_MODE`, including tool-call/tool-result payloads nested inside recorded LLM input/output
- **Thinking content**: Controlled independently via `BRAINTRUST_RECORD_THINKING`
- **Root turn metadata**: Every exported turn root includes `session_id`; when available it also includes `input_message_id`, monotonic session ordering fields, deployment grade, session status, model/provider summary, retry markers, and compaction markers
- **Session lifecycle markers**: `session.started`, `session.activated`, and `session.idled` are exported as lightweight session lifecycle logs to preserve grouped-session flow

### Session Grouping Contract

Braintrust grouping is session-first but still turn-scoped:

- one Braintrust trace per Everruns turn
- every root turn span carries `metadata.session_id`
- session ordering uses persisted Everruns event sequence metadata instead of inventing a second counter
- session lifecycle logs (`started`, `activated`, `idled`) use the same `session_id`

Consumers should group by `metadata.session_id` and use Braintrust timeline/thread views for the cross-turn session view. Everruns does not collapse an unbounded session into one monolithic Braintrust trace.

### Design Decisions

- **Full agentic loop tracing**: Traces turns, reason/act phases, LLM calls, and tool executions, matches Braintrust's OpenAI Agents integration feature parity.
- **Project name as primary config**: `BRAINTRUST_PROJECT_NAME` with default "My Project" matches the JS SDK pattern. Name-to-ID resolution at startup.
- **Bounded async delivery**: Events enter a bounded in-memory queue, flush in batches to `project_logs.insert`, retry on `429`, `5xx`, and timeout/connect failures, and log dropped/retried/permanent-failure counters.
- **Conservative defaults**: Raw content, reasoning text, and full tool payloads are off or reduced by default. Local debug logs never include full outbound Braintrust payloads unless `BRAINTRUST_DEBUG_PAYLOADS=true`.
- **Session-grouped turns, not giant session traces**: Session analysis is a metadata contract, not a single ever-growing trace.
- **EventListener pattern**: Observability is orthogonal to LLM execution, listens to completed events, consistent with OtelEventListener.
- **Blocking HTTP at startup**: `tokio::task::block_in_place` for one-time project name resolution. Simpler than async init.

### Implementation

- **File**: `crates/observability/src/braintrust.rs`
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

See `crates/observability/src/braintrust.rs` for the full request/response format.

---

## Test Coverage

### OTel Tests (in otel.rs)

Tests verify span creation, attributes, hierarchy, lifecycle, and content recording. Key scenarios: all 13 event types, turn lifecycle (started/completed/failed/cancelled), span hierarchy parent-child, multiple iterations, concurrent tool calls, orphaned completed events, and end-to-end full agent traces.

### Braintrust Tests

Tests verify delivery and mapping correctness:

1. **turn_id propagation**: All events within a turn share the same turn_id
2. **trace_id consistency**: All child events have trace_id = turn_id
3. **parent_span_id correctness**: Each event type references the correct parent
4. **span_id sharing**: started/completed pairs share the same span_id
5. **batching**: multiple events can flush in one insert request
6. **retry behavior**: `429` and timeout paths retry before failing
7. **privacy controls**: redaction/summary modes strip raw tool payloads by default
8. **session grouping metadata**: root turn spans carry stable `session_id` and session-ordering metadata

Both test suites must cover the task worker's DEV_MODE direct-store and full-mode gRPC-store execution paths.
