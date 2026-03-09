# Execution Phases

Execution phases distinguish intermediate working commentary from completed answers in multi-step tool-calling agent flows.

## Motivation

OpenAI's Responses API introduced the `phase` field for GPT-5.x models to prevent early stopping in long-running agent tasks. The phase signals whether an assistant message is intermediate work (will be followed by tool calls) or a final answer.

Reference: [OpenAI Prompt Guidance — Runtime and API integration notes](https://developers.openai.com/api/docs/guides/prompt-guidance#use-runtime-and-api-integration-notes)

## Design

### Phase values

- `"in_progress"` — intermediate assistant message with tool calls; the agent is still working
- `"completed"` — final answer; no tool calls follow

### Where phase is set

Phase is set in `ReasonAtom::execute_llm_call()` on every assistant message based on whether tool calls are present. See `crates/core/src/atoms/reason.rs`.

### Data flow

```
ReasonAtom (sets phase on Message)
  → OutputMessageCompletedData.message.phase (event)
  → LlmMessage.phase (provider-agnostic)
  → OpenResponses convert_message() (assistant-only, input to API)
```

Phase is **input-only** for the OpenAI Responses API: set on messages sent to the API, not parsed from responses.

### Iteration tracking

`OutputMessageStartedData.iteration` carries the 1-based iteration number within the current turn. This lets the UI show which iteration the agent is on during multi-step tool-calling flows. See `crates/core/src/events.rs` for the event type.

## Affected types

- `Message.phase: Option<String>` — `crates/core/src/message.rs`
- `LlmMessage.phase: Option<String>` — `crates/core/src/llm_driver_registry.rs`
- `ResponsesInputItem::Message.phase` — `crates/core/src/openresponses_protocol.rs`
- `ReasonInput.iteration: u32` — `crates/core/src/atoms/reason.rs`
- `OutputMessageStartedData.iteration: Option<u32>` — `crates/core/src/events.rs`

## UI

- `Message.phase?: string` and `OutputMessageStartedData.iteration?: number` — `apps/ui/src/lib/api/types.ts`
- Phase shown in message info tooltip and trajectory agent message nodes
- Iteration shown in streaming indicator when > 1
- `streamingIteration` state tracked in session context from `output.message.started` events

## Provider support

| Provider | Phase support |
|----------|-------------|
| OpenAI Responses API | Sent on assistant messages via `convert_message()` |
| Anthropic | Stored on `LlmMessage` but not sent (Anthropic API has no phase field) |
| Gemini | Stored on `LlmMessage` but not sent (Gemini API has no phase field) |

Phase is always set on the internal `Message` regardless of provider, enabling consistent UI behavior.
