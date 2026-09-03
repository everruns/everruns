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
│   └── llm call (LLM API call)            # llm.generation
│       └── thinking (if extended thinking) # reason.thinking.started → completed
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

Extended thinking happens inside the model call, so the thinking span nests
under the LLM call span. The `llm.generation` event is emitted once the call
returns; exporters that want the call span to have real duration open it at
`reason.thinking.started` when thinking is on, or backdate it by the event's
`duration_ms` otherwise (the OpenTelemetry listener does both).

### Started/Completed Merging

Events with both started and completed phases share the same `span_id`, so providers merge them into a single span with duration:

- `reason.started` / `reason.completed` share `reason_span_id`
- `act.started` / `act.completed` share `act_span_id`
- `tool.started` / `tool.completed` share `tool_span_id`
- `turn.started` / `turn.completed` share `turn_id`

### Trace Correlation

All events within a single turn share the same `turn_id` as their trace root. See `knowledge/execution/events.md` for EventContext fields (`trace_id`, `span_id`, `parent_span_id`). Parent-child relationships:

- reason events: `parent_span_id` = `turn_id`
- llm.generation: `parent_span_id` = `reason_span_id`
- act events: `parent_span_id` = `turn_id`
- tool events: `parent_span_id` = `act_span_id`
- thinking events carry no span ids, only the reason phase's `exec_id`;
  exporters resolve their parent through the phase that owns that exec

The general parent rule an exporter applies is: `parent_span_id`, else the
phase span that owns the event's `exec_id`, else the turn.

---

## OpenTelemetry

`OtelEventListener` (`crates/host/src/observability/otel.rs`) turns the agentic
event stream into spans that follow the OpenTelemetry Gen-AI agent and
inference conventions and, on the same spans, the OpenInference conventions
that Arize Phoenix reads. One OTLP stream therefore renders in Gen-AI-aware
backends (Grafana Tempo, Jaeger, Datadog, Langfuse) and in Phoenix alike.

### References

- [Gen-AI agent and framework spans](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-agent-spans.md)
- [Gen-AI inference and tool spans](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-spans.md)
- [Gen-AI attribute registry](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/registry/attributes/gen-ai.md)
- [OpenInference semantic conventions](https://arize-ai.github.io/openinference/spec/semantic_conventions.html)

### Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Yes (to enable) | - | OTLP/HTTP endpoint (e.g., `http://localhost:4318`) |
| `OTEL_SERVICE_NAME` | No | `everruns` | Service name in traces |
| `OTEL_SERVICE_VERSION` | No | crate version | Service version |
| `OTEL_ENVIRONMENT` | No | - | Deployment environment |
| `OTEL_SDK_DISABLED` | No | `false` | Disable OTel entirely |
| `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` | No | `false` | Record instructions, messages, tool args/results, reasoning text |
| `EVERRUNS_TRACE_CONVENTIONS` | No | `gen_ai,openinference` | Attribute vocabularies to write: `gen_ai`, `openinference`, or both |

`OTEL_RECORD_CONTENT` is kept as a legacy alias of the content-capture switch.

### Span model

| Event | Span name | Kind | `gen_ai.operation.name` | `openinference.span.kind` |
|-------|-----------|------|-------------------------|---------------------------|
| turn | `invoke_agent {agent name}` (`invoke_agent` when unknown) | `INTERNAL` | `invoke_agent` | `AGENT` |
| reason | `reason` | `INTERNAL` | none (`everruns.phase=reason`) | `CHAIN` |
| llm.generation | `chat {model}` | `CLIENT` | `chat` | `LLM` |
| thinking | `thinking` | `INTERNAL` | none (`everruns.phase=thinking`) | `CHAIN` |
| act | `act` | `INTERNAL` | none (`everruns.phase=act`) | `CHAIN` |
| tool | `execute_tool {name}` | `INTERNAL` | `execute_tool` | `TOOL` |

Nesting follows the hierarchy above: `chat` under `reason`, `thinking` under
the `chat` it belongs to, `execute_tool` under `act`. The reason/act phase
spans are Everruns intermediates the agent conventions permit; they carry no
`gen_ai.operation.name` so backends do not count them as Gen-AI operations.

Spans are built with the OpenTelemetry API rather than `tracing` spans so they
start and end at the timestamps of the events they record: the turn root at
`turn.started`, the chat span at `reason.thinking.started` when thinking is
on or backdated by the generation's `duration_ms` otherwise, and so on. The
listener therefore needs the global tracer provider that `init_telemetry`
installs; without an OTLP endpoint it runs on the no-op tracer.

### Attributes

The exact attribute set per span lives in `otel.rs`
(`chat_detail_attributes`, `tool_attributes`, `turn_usage_attributes`), the
vocabulary in `crates/core/src/telemetry.rs` (`gen_ai`) and
`crates/host/src/observability/openinference.rs`. What each span carries:

- **invoke_agent**: `gen_ai.agent.id/name/description` (from the agent
  identity on `turn.started`), `gen_ai.conversation.id`, cumulative
  `gen_ai.usage.*`, `error.type`; OpenInference `agent.name`, `session.id`,
  `llm.token_count.*`, `metadata`.
- **chat**: `gen_ai.provider.name` (plus the deprecated `gen_ai.system`
  spelling, which several backends still key on), request/response model,
  `gen_ai.response.id`, `gen_ai.response.finish_reasons` as a string array,
  `gen_ai.usage.input_tokens/output_tokens/cache_read.input_tokens/cache_write.input_tokens`,
  `gen_ai.request.temperature/max_tokens/reasoning.level/stream`,
  `gen_ai.response.time_to_first_chunk`, `gen_ai.conversation.compacted`;
  OpenInference `llm.model_name/provider/system`, `llm.token_count.*`,
  `llm.cost.total`, `llm.invocation_parameters`, `llm.tools.N.tool.json_schema`.
- **execute_tool**: `gen_ai.tool.name/type/call.id`, `gen_ai.tool.description`
  (learned from the turn's `llm.generation` tool list), `gen_ai.agent.name`;
  OpenInference `tool.name/description`.
- **Everruns extras** live under `everruns.*`: turn/exec/input-message ids,
  `everruns.phase`, iteration and tool-call counters, `everruns.tool.status`,
  retry counts, `everruns.usage.cost_usd`, and the diagnostic markers
  `everruns.span.orphaned` (terminal event without a start) and
  `everruns.span.unterminated` (closed by its turn ending).

Provider names map from Everruns driver ids to the registry's values
(`gemini` → `gcp.gemini`, `bedrock` → `aws.bedrock`, `azure_openai` →
`azure.ai.openai`); drivers without a registry entry keep their id, which the
spec allows as a custom value.

Errors set `error.type` (an explicit error code, else an HTTP status found in
the message, else `timeout`, else `_OTHER`), an error span status carrying the
message, and an `exception` span event. Tool failures use the tool status
(`error`, `timeout`, `cancelled`) as `error.type`.

### Content recording

Disabled by default. With `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true`
the chat span carries `gen_ai.system_instructions`, `gen_ai.input.messages`,
`gen_ai.output.messages`, and `gen_ai.tool.definitions` in the spec's
`{role, parts}` JSON (built by `everruns_core::telemetry::content`), plus the
OpenInference `input.value`/`output.value` and flattened
`llm.input_messages.N.message.*` / `llm.output_messages.0.message.*`. Tool
spans carry `gen_ai.tool.call.arguments/result` and `input.value`/`output.value`;
the turn root carries the input message and the final answer preview; the
thinking span carries the reasoning text as `output.value`.

Two deliberate choices: Everruns models the agent's instructions separately
from the chat history, so system-role messages go to
`gen_ai.system_instructions` and are excluded from `gen_ai.input.messages`;
and the model's reasoning text becomes a `reasoning` part of the output
message. Image bytes are never copied into telemetry (base64 images become a
`blob` part naming only the media type).

### Not emitted

- `server.address`/`server.port`: provider endpoints are not on the events.
- `gen_ai.request.model` on `invoke_agent`: sessions can switch models
  mid-session, and the spec says to omit it for agents with dynamic models.
- `gen_ai.output.type`: Everruns does not request structured output modes.
- Parameter schemas inside `gen_ai.tool.definitions`: tool summaries on the
  event carry name and description only.

### Design decisions

- **OpenTelemetry API instead of `tracing` spans**: only the API lets spans
  start and end at event timestamps; the `tracing` bridge stamps listener
  time. `init_telemetry` installs the provider globally for this, while HTTP
  request spans keep flowing through the `tracing` layer.
- **Agent identity on `turn.started`**: the turn root is the only place to
  name the agent, and the listener has no store access, so the emitters
  snapshot `agent_id`, `agent_name`, and `agent_description` on the event.
- **Both vocabularies by default**: Gen-AI and OpenInference attributes are
  cheap without content capture, and one stream serving every backend beats a
  per-backend switch. `EVERRUNS_TRACE_CONVENTIONS` narrows it.
- **Thinking under chat, opened early**: thinking is part of the model call;
  opening the chat span at `reason.thinking.started` gives it its real start
  and lets thinking nest inside it. A chat span left pending when the reason
  phase ends without a generation record is closed with an error.
- **Turn end closes stragglers**: whatever is still open under a turn when it
  completes, fails, or is cancelled is ended at the turn's end timestamp and
  marked `everruns.span.unterminated`, so no span leaks and no trace stays
  half-open.
- **Phase spans stay**: reason/act/thinking are not in the spec but give
  traces the structure that makes agent behavior debuggable.

### Implementation

| File | Purpose |
|------|---------|
| `crates/host/src/observability/otel.rs` | `OtelEventListener`, span lifecycle, attribute assembly, `TraceConventions` |
| `crates/host/src/observability/openinference.rs` | OpenInference vocabulary and flattened message builders |
| `crates/host/src/observability/telemetry.rs` | OTLP exporter wiring, global tracer provider, tracing-subscriber layers, config, init |
| `crates/core/src/telemetry.rs` | Gen-AI attribute names, provider mapping, `content` JSON builders, `error_type` |
| `crates/server/src/app_builder.rs` | Listener registration |

Ownership boundary: core holds only the neutral observability contracts, the `EventListener` trait, event types, and gen-AI span conventions. `everruns-host::observability` owns telemetry initialization, exporter dependencies, and the `CompositeEventListener` fan-out behind an opt-in feature. The isolation guard keeps exporter crates out of core and default Framework/provider dependency trees.

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

For the complete field-by-field mapping (LLM generation, tool events, thinking events), see `crates/host/src/observability/braintrust.rs`. Key mappings:

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

- **File**: `crates/host/src/observability/braintrust.rs`
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

See `crates/host/src/observability/braintrust.rs` for the full request/response format.

---

## Test Coverage

### OTel Tests (in otel.rs)

Tests run the listener against an in-memory span exporter and assert on the exported spans: names, kinds, parent ids, event-derived start/end times, Gen-AI and OpenInference attributes, error status and `exception` events, content capture off by default and spec-shaped when on, orphaned and unterminated span handling, and convention narrowing. `crates/core/src/telemetry.rs` covers the content JSON builders, provider mapping, and `error_type`.

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
