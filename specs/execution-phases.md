# Execution Phases Specification

## Abstract

Execution phases tag assistant messages with their intent — intermediate commentary or final answer. This helps LLMs (especially in multi-step tool-calling flows) distinguish between working commentary and completed responses when conversation history is replayed.

## Design

### Problem

In agentic loops with multiple reason→act iterations, earlier assistant messages are commentary that precedes tool calls. Without phase annotations, models may misinterpret intermediate commentary as final answers, causing early stopping or degraded performance on complex workflows.

### Solution: Derive-and-Map

Phases are **always derived from state** in the ReasonAtom, then **mapped to provider-specific wire values** when the model profile has `supports_phases: true`. Models without native phase support still benefit from internal phase tracking (events, observability).

```
ReasonAtom (derives phase from state)
    │
    ├── has tool calls → ExecutionPhase::Commentary
    └── no tool calls  → ExecutionPhase::FinalAnswer
                              │
                    ┌─────────┴─────────┐
                    │                   │
            supports_phases      !supports_phases
            (model profile)      (model profile)
                    │                   │
            Send to provider     Track internally
            (wire format)        (events, UI)
```

## ExecutionPhase Enum

See `crates/core/src/message.rs` for the full definition.

| Variant | Wire value | Description |
|---|---|---|
| `Commentary` | `"commentary"` | Intermediate update — preamble or working commentary before/between tool calls |
| `FinalAnswer` | `"final_answer"` | Completed response — no more tool calls expected |

### Backward Compatibility

Legacy values are accepted during deserialization:
- `"in_progress"` → `Commentary`
- `"completed"` → `FinalAnswer`

## Model Profile Flag

`supports_phases: bool` on `LlmModelProfile` — indicates whether the model accepts phase values in the provider API.

| Model | `supports_phases` | Notes |
|---|---|---|
| GPT-5.4 | `true` | Native support via `phase` field on input messages |
| GPT-5.4 Pro | `true` | Native support via `phase` field on input messages |
| All other models | `false` | Phase tracked internally only |

This replaced the previous `LlmDriver::supports_phases()` trait method. Phase support is a model-level capability, not a driver-level one — the same OpenAI Responses API driver serves both phase-capable (GPT-5.4) and non-phase models.

## Provider Mapping

### OpenAI Responses API (GPT-5.4+)

The `phase` field is serialized as a string on assistant messages in the `input` array:
- `ExecutionPhase::Commentary` → `"commentary"`
- `ExecutionPhase::FinalAnswer` → `"final_answer"`

For non-GPT-5.4 models on the same driver, the `phase` field is omitted from the wire format.

See `crates/core/src/openresponses_protocol.rs` for the mapping.

### Anthropic / Gemini

Phase is not sent to the provider API. The `ExecutionPhase` value is still set on `Message.phase` for internal use (events, observability, UI).

## Flow

1. ReasonAtom completes LLM streaming, collects text and tool calls
2. Phase is derived: `ExecutionPhase::from_has_tool_calls(has_tool_calls)`
3. Phase is stored on the `Message` (persisted via events)
4. On next iteration, message history is converted to `LlmMessage` — phase is preserved
5. Driver converts `LlmMessage` to provider format:
   - OpenAI Responses (GPT-5.4+): maps `ExecutionPhase` → `"commentary"` / `"final_answer"` string
   - Others: phase field is ignored by the driver
