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

See `crates/core/src/message.rs` for the full definition, wire values, and legacy deserialization mappings.

Two variants: `Commentary` (intermediate, before/between tool calls) and `FinalAnswer` (completed response, no more tool calls).

## Model Profile Flag

`supports_phases: bool` on `LlmModelProfile` — indicates whether the model accepts phase values in the provider API. Phase support is a model-level capability, not a driver-level one — the same OpenAI Responses API driver serves both phase-capable and non-phase models. See `crates/core/src/llm_models.rs` for the model profile definitions.

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
2. Phase is **preserved from the API response** when available (extracted from `response.completed` output items via `LlmCompletionMetadata.phase`). Falls back to derivation: `ExecutionPhase::from_has_tool_calls(has_tool_calls)`
3. Phase is stored on the `Message` (persisted via events)
4. On next iteration, message history is converted to `LlmMessage` — phase is preserved
5. Driver converts `LlmMessage` to provider format:
   - OpenAI Responses (GPT-5.4+): maps `ExecutionPhase` → `"commentary"` / `"final_answer"` string
   - Others: phase field is ignored by the driver

### Why preserve, not derive?

OpenAI docs require that the `phase` value returned in response output items must be preserved and sent back as-is in subsequent requests. While our derivation heuristic (tool calls → commentary, no tool calls → final_answer) likely matches the API's assignment, preserving the API value is the correct behavior — it protects against future divergence and follows the provider contract.
