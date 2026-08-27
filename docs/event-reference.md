---
title: Event Reference
description: Complete reference for all Everruns event types, input, output, tool, lifecycle, and error events emitted during execution. Includes schemas and SSE examples.
---

This page documents all event types in the Everruns event protocol.

## Input Events

### input.message

Emitted when a user message is submitted to the session.

| Field | Type | Description |
|-------|------|-------------|
| `message` | Message | The user message object |

```json
{
  "type": "input.message",
  "data": {
    "message": {
      "id": "message_...",
      "role": "user",
      "content": [{"type": "text", "text": "Hello!"}],
      "created_at": "2024-01-15T10:30:00.000Z"
    }
  }
}
```

## Output Events

### output.message.started

Emitted when the LLM starts generating a response. This marks the start of
generation, not model reasoning: reasoning has its own events (see [Reasoning
Events](#reasoning-events)) and its own channel.

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn ID this output belongs to |
| `model` | string? | Optional model name being used |
| `phase` | string? | Best-effort phase hint: `commentary` or `final_answer`. Absent means unclassified, never "reasoning". |

```json
{
  "type": "output.message.started",
  "data": {
    "turn_id": "turn_...",
    "model": "gpt-5.2"
  }
}
```

### output.message.delta

Incremental text update during LLM generation. Events are batched (~100ms).

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn ID this delta belongs to |
| `delta` | string | New text since last delta |
| `accumulated` | string | Total text so far |

```json
{
  "type": "output.message.delta",
  "data": {
    "turn_id": "turn_...",
    "delta": "Hello",
    "accumulated": "Hello"
  }
}
```

### output.message.completed

Emitted when the agent response is complete.

| Field | Type | Description |
|-------|------|-------------|
| `message` | Message | The complete agent message |
| `metadata` | ModelMetadata? | Model information |
| `usage` | TokenUsage? | Token usage statistics |

`message.phase` is authoritative for whether this message is intermediate
`commentary` or the turn's `final_answer`, and `message.phase_source` says
whether the provider reported that phase (`provider`) or the runtime inferred it
from tool-call presence (`derived`). Reasoning artifacts appear as `reasoning`
content parts inside `message.content`, in the order the provider emitted them.

```json
{
  "type": "output.message.completed",
  "data": {
    "message": {
      "id": "message_...",
      "role": "assistant",
      "content": [{"type": "text", "text": "Hello! How can I help?"}]
    },
    "usage": {
      "input_tokens": 50,
      "output_tokens": 25
    }
  }
}
```

## Turn Lifecycle Events

### turn.started

Emitted when a turn begins execution.

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn identifier |
| `input_message_id` | string | Message that triggered this turn |
| `input_content` | string? | Optional input content preview |

```json
{
  "type": "turn.started",
  "data": {
    "turn_id": "turn_...",
    "input_message_id": "message_...",
    "input_content": "Hello!"
  }
}
```

### turn.completed

Emitted when a turn completes successfully.

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn identifier |
| `iterations` | integer | Number of reason-act iterations |
| `duration_ms` | integer? | Duration in milliseconds |
| `usage` | TokenUsage? | Aggregated token usage |
| `input_content` | string? | Optional input content |
| `final_message_id` | string? | Canonical final assistant message ID |
| `final_answer_preview` | string? | Bounded final answer preview |
| `time_to_first_token_ms` | integer? | First-token latency |
| `tool_call_count` | integer? | Completed tool-call count |
| `llm_call_count` | integer? | LLM generation count |
| `status` | string? | Optional completion status |

```json
{
  "type": "turn.completed",
  "data": {
    "turn_id": "turn_...",
    "iterations": 3,
    "duration_ms": 1500,
    "usage": {
      "input_tokens": 500,
      "output_tokens": 200
    },
    "final_message_id": "message_...",
    "final_answer_preview": "Done.",
    "time_to_first_token_ms": 120,
    "tool_call_count": 2,
    "llm_call_count": 3,
    "status": "completed"
  }
}
```

### turn.failed

Emitted when a turn fails with an error.

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn identifier |
| `error` | string | Error message |
| `error_code` | string? | Optional error code |

```json
{
  "type": "turn.failed",
  "data": {
    "turn_id": "turn_...",
    "error": "Rate limit exceeded",
    "error_code": "RATE_LIMIT"
  }
}
```

### turn.cancelled

Emitted when a turn is cancelled by the user.

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn identifier |
| `reason` | string? | Cancellation reason |
| `usage` | TokenUsage? | Usage before cancellation |

```json
{
  "type": "turn.cancelled",
  "data": {
    "turn_id": "turn_...",
    "reason": "User requested",
    "usage": {
      "input_tokens": 100,
      "output_tokens": 50
    }
  }
}
```

## Reasoning Events

Model reasoning, as distinct from the `reason.started` / `reason.completed`
lifecycle events further down, which mark an LLM *inference step* in the
reason/act loop and are unrelated to whether the model reasoned.

These events are emitted by models that expose reasoning (Anthropic Claude with
thinking enabled, OpenAI GPT-5.x and o-series with reasoning effort configured,
Gemini with a thinking budget, and Chat Completions models that return
`reasoning_content`). Everything here belongs to the reasoning channel and must
never be rendered as assistant text.

### reason.thinking.started

Emitted when extended thinking begins.

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn ID |
| `model` | string? | Model name |

```json
{
  "type": "reason.thinking.started",
  "data": {
    "turn_id": "turn_...",
    "model": "claude-4-opus"
  }
}
```

### reason.thinking.delta

Streams incremental thinking content.

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn ID |
| `delta` | string | New thinking text |
| `accumulated` | string | Total thinking so far |

```json
{
  "type": "reason.thinking.delta",
  "data": {
    "turn_id": "turn_...",
    "delta": "Let me think about this...",
    "accumulated": "Let me think about this..."
  }
}
```

### reason.thinking.completed

Emitted when extended thinking completes.

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn ID |
| `thinking` | string | Complete thinking content |

```json
{
  "type": "reason.thinking.completed",
  "data": {
    "turn_id": "turn_...",
    "thinking": "I need to consider the user's request carefully..."
  }
}
```

### reason.item

Emitted when one reasoning artifact completes. One event per provider reasoning
block, in emission order.

Carries identity and safe summary text only. The opaque payloads that make the
artifact replayable — provider signatures and encrypted reasoning context — are
deliberately excluded: they are replay state, not content, and never appear in
events or on any API surface.

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn ID this artifact belongs to |
| `provider` | string | Provider that produced it (`anthropic`, `openai`, `google`) |
| `model` | string? | Model reported by the provider |
| `item_id` | string | Provider-assigned identifier, when the provider issues one |
| `summary` | string[] | Provider-curated summary segments. Never raw chain-of-thought. |
| `token_count` | integer? | Reasoning tokens attributed to this artifact |

```json
{
  "type": "reason.item",
  "data": {
    "turn_id": "turn_...",
    "provider": "openai",
    "model": "gpt-5.2",
    "item_id": "rs_68a1f...",
    "summary": ["Checking the build logs before answering."],
    "token_count": 412
  }
}
```

## Atom Lifecycle Events

These events mark steps of the reason/act execution loop. `reason.*` here means
"the LLM inference step", not model reasoning — for that see [Reasoning
Events](#reasoning-events).

### reason.started

Emitted when LLM inference begins.

| Field | Type | Description |
|-------|------|-------------|
| `agent_id` | string | Agent ID |
| `metadata` | ModelMetadata? | Model information |

```json
{
  "type": "reason.started",
  "data": {
    "agent_id": "agent_...",
    "metadata": {
      "model": "gpt-5.2"
    }
  }
}
```

### reason.completed

Emitted when LLM inference completes.

| Field | Type | Description |
|-------|------|-------------|
| `success` | boolean | Whether the call succeeded |
| `text_preview` | string? | First 200 chars of response |
| `has_tool_calls` | boolean | Whether tools were requested |
| `tool_call_count` | integer | Number of tool calls |
| `error` | string? | Error if failed |
| `duration_ms` | integer? | Duration |
| `usage` | TokenUsage? | Token usage |

```json
{
  "type": "reason.completed",
  "data": {
    "success": true,
    "text_preview": "Hello! I can help you with...",
    "has_tool_calls": false,
    "tool_call_count": 0,
    "duration_ms": 1200,
    "usage": {
      "input_tokens": 100,
      "output_tokens": 50
    }
  }
}
```

### act.started

Emitted when tool execution batch begins.

| Field | Type | Description |
|-------|------|-------------|
| `tool_calls` | ToolCallSummary[] | Tools to be executed |

```json
{
  "type": "act.started",
  "data": {
    "tool_calls": [
      {"id": "tc_1", "name": "get_weather"},
      {"id": "tc_2", "name": "search_web"}
    ]
  }
}
```

### act.completed

Emitted when tool execution batch completes.

| Field | Type | Description |
|-------|------|-------------|
| `completed` | boolean | All tools completed |
| `success_count` | integer | Successful tool calls |
| `error_count` | integer | Failed tool calls |
| `duration_ms` | integer? | Total duration |

```json
{
  "type": "act.completed",
  "data": {
    "completed": true,
    "success_count": 2,
    "error_count": 0,
    "duration_ms": 500
  }
}
```

### tool.started

Emitted when individual tool execution begins.

| Field | Type | Description |
|-------|------|-------------|
| `tool_call` | ToolCall | Full tool call with arguments |

```json
{
  "type": "tool.started",
  "data": {
    "tool_call": {
      "id": "tc_1",
      "name": "get_weather",
      "arguments": {"city": "London"}
    }
  }
}
```

### tool.completed

Emitted when individual tool execution completes.

| Field | Type | Description |
|-------|------|-------------|
| `tool_call_id` | string | Tool call ID |
| `tool_name` | string | Tool name |
| `success` | boolean | Whether it succeeded |
| `status` | string | "success", "error", "timeout", "cancelled" |
| `result` | ContentPart[]? | Result content |
| `error` | string? | Error message |
| `duration_ms` | integer? | Duration |

```json
{
  "type": "tool.completed",
  "data": {
    "tool_call_id": "tc_1",
    "tool_name": "get_weather",
    "success": true,
    "status": "success",
    "result": [{"type": "text", "text": "Sunny, 22°C"}],
    "duration_ms": 250
  }
}
```

## LLM Events

### llm.generation

Full visibility into LLM API calls. Emitted after each call.

| Field | Type | Description |
|-------|------|-------------|
| `messages` | Message[] | Messages sent to LLM |
| `tools` | ToolDefinitionSummary[] | Available tools |
| `output` | LlmGenerationOutput | LLM response |
| `metadata` | LlmGenerationMetadata | Call metadata |

```json
{
  "type": "llm.generation",
  "data": {
    "messages": [...],
    "tools": [{"name": "get_weather", "description": "..."}],
    "output": {
      "text": "Hello!",
      "tool_calls": []
    },
    "metadata": {
      "model": "gpt-5.2",
      "provider": "openai",
      "usage": {"input_tokens": 100, "output_tokens": 50},
      "duration_ms": 1200,
      "time_to_first_token_ms": 150,
      "success": true,
      "finish_reasons": ["stop"]
    }
  }
}
```

## Session Events

### session.started

Emitted when a session begins.

| Field | Type | Description |
|-------|------|-------------|
| `agent_id` | string | Agent ID |
| `model_id` | string? | Model ID if specified |

```json
{
  "type": "session.started",
  "data": {
    "agent_id": "agent_..."
  }
}
```

### session.activated

Emitted when a session becomes active (turn started).

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Turn that activated session |
| `input_message_id` | string | Triggering message |

```json
{
  "type": "session.activated",
  "data": {
    "turn_id": "turn_...",
    "input_message_id": "message_..."
  }
}
```

### session.idled

Emitted when a session becomes idle (turn completed).

| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | string | Completed turn |
| `iterations` | integer? | Iterations in turn |
| `usage` | TokenUsage? | Cumulative session usage |

```json
{
  "type": "session.idled",
  "data": {
    "turn_id": "turn_...",
    "iterations": 3,
    "usage": {
      "input_tokens": 1500,
      "output_tokens": 800
    }
  }
}
```

## Subagent Events (retired)

The `subagent.spawned`, `subagent.completed`, `subagent.failed`, and
`subagent.cancelled` events have been **retired**. The subagent flow is now
modeled as Session Tasks, which emit `task.*` lifecycle events
(`task.created`, `task.updated`, `task.message.sent`, `task.message.received`)
on the parent session instead. New sessions never emit `subagent.*`.

These event types are no longer produced or part of the supported contract.
Historical `subagent.*` events recorded in older session logs remain in storage,
but are filtered out of the events and SSE APIs like any unsupported type, they
are not returned to consumers (aggregate counters such as `error_count` still
include them). Consumers should read `task.*` events going forward.

## Supporting Types

### TokenUsage

Token consumption statistics.

| Field | Type | Description |
|-------|------|-------------|
| `input_tokens` | integer | Input/prompt tokens |
| `output_tokens` | integer | Output/completion tokens |
| `cache_read_tokens` | integer? | Tokens read from cache |
| `cache_creation_tokens` | integer? | Tokens written to cache (Anthropic) |

### ModelMetadata

Information about the model used.

| Field | Type | Description |
|-------|------|-------------|
| `model` | string | Model name (e.g., "gpt-5.2") |
| `model_id` | string? | Internal model ID |
| `provider_id` | string? | Internal provider ID |

### ToolCallSummary

Compact tool call representation.

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Tool call ID |
| `name` | string | Tool name |

### ToolCall

Full tool call with arguments.

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Tool call ID |
| `name` | string | Tool name |
| `arguments` | object | Tool arguments (JSON) |
