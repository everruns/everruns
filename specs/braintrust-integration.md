# Braintrust Integration

Observability integration for sending agentic loop events to Braintrust.

## Abstract

Everruns integrates with [Braintrust](https://www.braintrust.dev/) to provide LLM observability, evaluation, and logging capabilities. The integration sends agentic loop events (turns, LLM generations, tool executions) to Braintrust's project logs API, enabling trace visualization, token usage tracking, and model performance analysis.

## References

- **Braintrust Documentation**: https://www.braintrust.dev/docs
- **API Reference**: https://www.braintrust.dev/docs/api-reference/introduction
- **Insert Project Logs**: https://www.braintrust.dev/docs/api-reference/logs/insert-project-logs-events
- **List Projects API**: https://www.braintrust.dev/docs/reference/api/Projects
- **TypeScript SDK**: https://github.com/braintrustdata/braintrust-sdk (reference implementation)
- **OpenAI Agents Integration**: https://github.com/braintrustdata/braintrust-sdk/tree/main/integrations/openai-agents-js

## Requirements

### Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BRAINTRUST_API_KEY` | Yes (to enable) | - | API key from Braintrust organization settings |
| `BRAINTRUST_PROJECT_NAME` | No | `My Project` | Project name (resolved to ID at startup) |
| `BRAINTRUST_PROJECT_ID` | No | - | Direct project UUID (skips name resolution) |
| `BRAINTRUST_API_URL` | No | `https://api.braintrust.dev` | API base URL |

### Event Types

The integration traces the full agentic loop with the following event types:

| Event Type | Span Type | Description |
|------------|-----------|-------------|
| `turn.started` | `task` | Root span - agent turn begins |
| `turn.completed` | `task` | Root span - agent turn succeeds |
| `turn.failed` | `task` | Root span - agent turn fails |
| `turn.cancelled` | `task` | Root span - agent turn cancelled |
| `reason.started` | `task` | Child span - LLM reasoning phase begins |
| `reason.completed` | `task` | Child span - LLM reasoning phase ends |
| `reason.thinking.started` | `task` | Child span - Extended thinking begins (within reason) |
| `reason.thinking.completed` | `task` | Child span - Extended thinking ends (within reason) |
| `act.started` | `task` | Child span - Tool execution phase begins |
| `act.completed` | `task` | Child span - Tool execution phase ends |
| `llm.generation` | `llm` | Child span - LLM API call |
| `tool.started` | `tool` | Child span - Tool invocation begins |
| `tool.completed` | `tool` | Child span - Tool invocation completes |

### Span Hierarchy

The agentic loop produces a hierarchical trace structure where all events within a turn share the same `turn_id` as their trace root. Multiple iterations of reason/act create sibling spans under the turn root.

```
agent turn (root)
├── reason (iteration 1)
│   ├── thinking (if extended thinking enabled)
│   └── llm.generation (gpt-4o)
├── act (iteration 1)
│   ├── tool.call (search)
│   └── tool.call (fetch)
├── reason (iteration 2)
│   ├── thinking (if extended thinking enabled)
│   └── llm.generation (gpt-4o)
├── act (iteration 2)
│   └── tool.call (compute)
├── reason (iteration 3)
│   ├── thinking (if extended thinking enabled)
│   └── llm.generation (gpt-4o)
└── (no act - turn complete)
```

**Note:** Thinking spans only appear when `reasoning_effort` is configured and using a model that supports extended thinking (e.g., Anthropic Claude).

### Span ID Relationships

| Event Type | span_id | root_span_id | span_parents |
|------------|---------|--------------|--------------|
| `turn.started/completed` | turn_id | turn_id | `null` (root) |
| `reason.started/completed` | reason_span_id | turn_id | `[turn_id]` |
| `reason.thinking.started/completed` | thinking_span_id | turn_id | `[reason_span_id]` |
| `llm.generation` | llm_span_id | turn_id | `[reason_span_id]` |
| `act.started/completed` | act_span_id | turn_id | `[turn_id]` |
| `tool.started/completed` | tool_span_id | turn_id | `[act_span_id]` |

### Trace Correlation Requirements

All events within a single turn **MUST** share the same `turn_id` to appear in the same trace:

1. **turn_id creation**: Created once by the `input` activity when processing begins
2. **turn_id propagation**: Passed through `DurableTurnInput` to subsequent activities
3. **Event context**: Each atom (reason, act) receives the turn_id via `AtomContext`
4. **Span fields**: Events use `TurnId::to_string()` format (prefixed, e.g., `turn_abc123`)

**Critical**: The `trace_id` and `parent_span_id` fields in events must use the same ID format as the root span's `span_id`. Using different formats (e.g., hyphenated UUID vs prefixed ID) will cause spans to appear as disconnected traces.

### Started/Completed Event Merging

Events that have both started and completed phases share the same `span_id` so Braintrust merges them into a single span with timing:

| Started Event | Completed Event | Shared span_id |
|---------------|-----------------|----------------|
| `reason.started` | `reason.completed` | `reason_span_id` |
| `act.started` | `act.completed` | `act_span_id` |
| `tool.started` | `tool.completed` | `tool_span_id` |

The `turn.started` and `turn.completed` events both use `turn_id` as their log ID to merge into a single root span.

**Note**: Braintrust API requires both `span_id` and `root_span_id` together. Root spans use self-referencing IDs (span_id = root_span_id = turn_id) to establish themselves as trace roots. Child spans reference the root via root_span_id and their parent via span_parents.

### Data Mapping

#### LLM Generation Events

| Everruns Event Field | Braintrust Field | Notes |
|---------------------|------------------|-------|
| `event.id` | `id`, `span_id` | Event UUID |
| `event.ts` | `created` | ISO 8601 timestamp |
| `data.messages` | `input` | Full message array |
| `data.output.text` | `output.text` | LLM response text |
| `data.output.tool_calls` | `output.tool_calls` | Tool call array |
| `metadata.model` | `metadata.model` | Model identifier |
| `metadata.provider` | `metadata.provider` | Provider name |
| `metadata.usage.input_tokens` | `metrics.prompt_tokens` | Input token count |
| `metadata.usage.output_tokens` | `metrics.completion_tokens` | Output token count |
| `metadata.usage.cache_read_tokens` | `metrics.cache_read_tokens` | Prompt cache hits |
| `metadata.usage.cache_creation_tokens` | `metrics.cache_creation_tokens` | Prompt cache writes |
| `metadata.duration_ms` | `metrics.start`, `metrics.end` | Calculated from duration |
| `metadata.time_to_first_token_ms` | `metrics.time_to_first_token` | TTFT in seconds |
| `metadata.error` | `error` | Error message if failed |
| `event.session_id` | `metadata.session_id` | Everruns session ID |
| `event.context.turn_id` | `metadata.turn_id`, `root_span_id` | Turn within session |

#### Tool Events

| Everruns Event Field | Braintrust Field | Notes |
|---------------------|------------------|-------|
| `data.tool_call_id` | `input.tool_call_id` | Tool call identifier |
| `data.tool_name` | `input.tool_name`, `span_attributes.name` | Tool name |
| `data.success` | `metadata.success`, `output.status` | Execution result |
| `data.result` | `output.result` | Tool result (on success) |
| `data.error` | `error`, `output.error` | Error message (on failure) |

#### Thinking Events

Extended thinking events (`reason.thinking.*`) are converted to Braintrust spans when models use reasoning mode.

| Everruns Event Field | Braintrust Field | Notes |
|---------------------|------------------|-------|
| `data.turn_id` | `metadata.turn_id` | Turn context |
| `data.thinking` | `output.thinking` | Complete thinking content (on completed) |
| `data.model` | `metadata.model` | Model name (on started) |
| `event.context.span_id` | `span_id` | Thinking span ID |
| `event.context.parent_span_id` | `span_parents` | Parent is reason span |

**Note:** Thinking spans are child spans of the reason phase. They appear between `reason.started` and `llm.generation` in the span hierarchy.

### Span Attributes

Events are sent with span attributes based on type:

| Event Type | `span_attributes.type` | `span_attributes.name` |
|------------|------------------------|------------------------|
| Turn events | `task` | `"agent turn"` |
| Reason events | `task` | `"reason"` |
| Thinking events | `task` | `"thinking"` |
| Act events | `task` | `"act"` |
| LLM generation | `llm` | `"chat {model}"` (e.g., "chat gpt-4o") |
| Tool events | `tool` | `"tool {name}"` (e.g., "tool search") |

## Design Decisions

### Full Agentic Loop Tracing

**Decision**: Trace the complete agentic loop (turns, reason/act phases, LLM calls, tool executions) rather than just LLM generations.

**Rationale**:
- Provides complete visibility into agent behavior
- Enables debugging of multi-step workflows
- Matches feature parity with Braintrust's OpenAI Agents integration
- Hierarchical span structure shows parent-child relationships

### Prompt Caching Metrics

**Decision**: Include `cache_read_tokens` and `cache_creation_tokens` in metrics.

**Rationale**:
- Critical for Claude models with prompt caching
- Enables tracking of cache efficiency and cost savings
- Matches Braintrust SDK's handling of cached tokens

### Project Name as Primary Configuration

**Decision**: Use `BRAINTRUST_PROJECT_NAME` with default "My Project" instead of requiring `BRAINTRUST_PROJECT_ID`.

**Rationale**:
- Project names are human-readable and visible in Braintrust UI
- Onboarding flow doesn't expose project IDs prominently
- Matches the JS SDK pattern (`initLogger({ projectName: "..." })`)
- Name-to-ID resolution happens once at startup (negligible overhead)

### Async Event Delivery

**Decision**: Send events asynchronously via `tokio::spawn` to avoid blocking the main event processing flow.

**Rationale**:
- Event delivery to external APIs should not block database event persistence
- Transient network failures shouldn't affect core system functionality
- Failed deliveries are logged but don't cause retries (fire-and-forget)

### EventListener Pattern

**Decision**: Implement as an `EventListener` rather than a driver wrapper.

**Rationale**:
- Observability is orthogonal to LLM execution
- Listens to completed events, not in-flight requests
- Consistent with existing patterns (OtelEventListener, UsageTrackingListener)
- Easy to enable/disable via configuration

### Blocking HTTP at Startup

**Decision**: Use `tokio::task::block_in_place` for project name resolution at startup.

**Rationale**:
- Resolution happens once during initialization
- Simpler than async initialization patterns
- `block_in_place` tells tokio to handle blocking work properly
- Alternative (async init) would require significant refactoring

### Message Format Conversion

Messages are converted to OpenAI-compatible format before sending to Braintrust:

| Internal Role | OpenAI Role | Notes |
|--------------|-------------|-------|
| `agent` | `assistant` | Assistant responses |
| `tool_result` | `tool` | Includes `tool_call_id` at message level |
| `system` | `system` | No change |
| `user` | `user` | No change |

Conversion is handled via `Message::to_openai_format()` and `ToolCall::to_openai_format()` methods defined in the core crate.

## Implementation

- **File**: `crates/core/src/observation/braintrust.rs`
- **Registration**: `crates/control-plane/src/main.rs` (event listener setup)
- **Configuration**: `docs/sre/environment-variables.md`
- **Format conversion**: `crates/core/src/message.rs` (`Message::to_openai_format()`)

## API Endpoints Used

### Project Resolution

```
GET /v1/project?project_name={name}
Authorization: Bearer {api_key}
```

Response:
```json
{
  "objects": [
    { "id": "uuid", "name": "My Project", ... }
  ]
}
```

### Insert Logs

```
POST /v1/project_logs/{project_id}/insert
Authorization: Bearer {api_key}
Content-Type: application/json

{
  "events": [
    {
      "id": "event-uuid",
      "span_id": "span-uuid",
      "root_span_id": "turn-uuid",
      "span_parents": ["turn-uuid"],
      "created": "2024-01-15T10:30:00Z",
      "input": [...],
      "output": {...},
      "metadata": {...},
      "metrics": {
        "prompt_tokens": 100,
        "completion_tokens": 50,
        "tokens": 150,
        "cache_read_tokens": 80,
        "cache_creation_tokens": 20,
        "time_to_first_token": 0.045
      },
      "span_attributes": { "type": "llm", "name": "chat gpt-4o" }
    }
  ]
}
```

Response:
```json
{
  "row_ids": ["inserted-row-id"]
}
```

## Test Coverage Requirements

### Event Relationship Tests

Tests MUST verify that events emitted during agentic execution have correct ID relationships:

1. **turn_id propagation**: All events within a turn share the same turn_id
2. **trace_id consistency**: All child events have trace_id = turn_id (prefixed format)
3. **parent_span_id correctness**:
   - reason events: parent_span_id = turn_id
   - llm.generation events: parent_span_id = reason_span_id
   - act events: parent_span_id = turn_id
   - tool.call events: parent_span_id = act_span_id
4. **span_id sharing**: started/completed pairs share the same span_id

### Worker Coverage

Tests MUST cover both execution paths:

1. **dev_worker** (DEV_MODE): In-process execution with direct database access
2. **durable_worker** (Full mode): gRPC-based execution with durable workflow engine

### Test Scenarios

| Scenario | Events to Verify | ID Relationships |
|----------|------------------|------------------|
| Single LLM call (no tools) | turn.started, reason, llm.generation, reason.completed, turn.completed | All share same turn_id |
| One tool call | Above + act, tool.call, act.completed | Tool has parent=act, act has parent=turn |
| Multiple iterations | Multiple reason/act pairs | All iterations share same turn_id |
| Parallel tool calls | Multiple tool.call events | All tools share same act_span_id as parent |

### Implementation Notes

Event relationship tests should:
- Use mock/test event emitters to capture events
- Assert span_id, trace_id, parent_span_id values are correct
- Verify ID format consistency (all use TurnId::to_string() prefixed format)
- Test across multiple iterations to ensure turn_id propagation through workflow state
