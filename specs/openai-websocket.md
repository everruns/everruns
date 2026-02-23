# OpenAI WebSocket Mode Support

## Status: Draft

## Overview

OpenAI's Responses API supports a WebSocket transport (`wss://api.openai.com/v1/responses`) that eliminates per-request overhead and enables incremental input across multi-turn tool-calling workflows. Measured ~40% end-to-end latency reduction for 20+ tool call sequences.

This spec covers adding WebSocket transport as an optimization to the existing `OpenResponsesProtocolLlmDriver`, transparent to the rest of the system.

## References

- [OpenAI WebSocket Mode Guide](https://developers.openai.com/api/docs/guides/websocket-mode)
- `specs/llm-drivers.md` — LLM driver trait, provider implementations
- `specs/architecture.md` — System architecture
- `specs/events.md` — Event types and SSE streaming

## Key Properties of OpenAI WebSocket Mode

| Property | Value |
|---|---|
| Endpoint | `wss://api.openai.com/v1/responses` |
| Auth | `Authorization: Bearer {key}` header on upgrade |
| Client→Server message | `{"type": "response.create", ...}` |
| Server→Client events | Same as Responses API SSE events |
| Connection limit | 60 minutes, then `websocket_connection_limit_reached` |
| Concurrency | One in-flight response per connection |
| Continuation | `previous_response_id` + only new input items |
| Warmup | `generate: false` prepares state without output |
| Compaction compat | Works with `previous_response_id` and `/responses/compact` |

## Current Architecture

### Call path (today)

```
TurnStateMachine loop:
  Reason → Act → Reason → Act → ... → Complete

Each Reason iteration:
  reason.rs:772  →  llm_driver.chat_completion_stream(messages, config)
                         ↓
                 OpenResponsesProtocolLlmDriver
                         ↓
                 HTTP POST /v1/responses (stream=true)
                         ↓
                 SSE byte stream → eventsource_stream → LlmStreamEvent
```

### Key types involved

- `LlmDriver` trait — `crates/core/src/llm_driver_registry.rs:100`
- `LlmCallConfig` — `crates/core/src/llm_driver_registry.rs:374`
- `LlmCompletionMetadata` — `crates/core/src/llm_driver_registry.rs:77` (has `response_id` field, currently unused)
- `LlmGenerationMetadata` — `crates/core/src/events.rs:878` (has `response_id: Option<String>`)
- `OpenResponsesProtocolLlmDriver` — `crates/core/src/openresponses_protocol.rs:73`
- Turn loop — `crates/core/src/in_memory_loop.rs:597`
- Reason atom LLM call — `crates/core/src/atoms/reason.rs:772`

### What already exists

1. `response_id` field in `LlmCompletionMetadata` and `LlmGenerationMetadata` — always `None` today
2. `previous_response_id` field in `CompactRequest` — always `None` today
3. Turn state machine loops Reason→Act→Reason, threading `ReasonResult` between iterations
4. Metadata dict on `LlmCallConfig` carries `session_id`, `turn_id`, `exec_id` per call

## Design

### Approach: Transport-Layer Optimization

WebSocket support is an **internal transport optimization** within `OpenResponsesProtocolLlmDriver`. The `LlmDriver` trait interface changes minimally — only adding `previous_response_id` to the call config so the driver knows when to use incremental input.

The caller (reason atom / turn loop) threads `response_id` from the previous call's metadata back into the next call's config. The driver decides whether to use HTTP or WebSocket based on availability and config.

```
                    ┌──────────────────────────────────┐
                    │   LlmDriver trait (unchanged)    │
                    │   chat_completion_stream(         │
                    │     messages, config              │
                    │   ) → LlmResponseStream           │
                    └──────────────┬───────────────────┘
                                   │
                    ┌──────────────▼───────────────────┐
                    │ OpenResponsesProtocolLlmDriver   │
                    │                                   │
                    │  Has previous_response_id in cfg? │
                    │         ╱            ╲            │
                    │       Yes             No          │
                    │        │               │          │
                    │   ┌────▼────┐    ┌────▼────┐     │
                    │   │WebSocket│    │  HTTP   │     │
                    │   │transport│    │transport│     │
                    │   └────┬────┘    └────┬────┘     │
                    │        │               │          │
                    │    Same LlmStreamEvent output     │
                    └──────────────────────────────────┘
```

### Why not a separate driver or new trait?

- WebSocket mode uses identical request/response schemas — only the transport differs
- The optimization is specific to multi-turn tool-calling within a single turn execution
- Keeping it inside the driver avoids leaking transport details into the trait
- Other providers (Anthropic, Gemini) don't support this — no need for a generic abstraction

## Implementation Phases

### Phase 1: Response ID Plumbing

**Goal:** Extract and thread `response_id` through the turn loop. No WebSocket yet — but this is required infrastructure and independently useful for OTel tracing.

**Changes:**

1. **Extract `response_id` from OpenAI response events** (`openresponses_protocol.rs`)
   - Parse `response.completed` event for `id` field (the response ID)
   - Set it on `LlmCompletionMetadata.response_id` (field exists, currently `None`)

2. **Add `previous_response_id` to `LlmCallConfig`** (`llm_driver_registry.rs`)
   ```rust
   pub struct LlmCallConfig {
       // ... existing fields ...
       /// Previous response ID for stateful continuation (OpenAI WebSocket mode)
       /// When set, driver may send only incremental input for efficiency.
       pub previous_response_id: Option<String>,
   }
   ```

3. **Thread response_id through turn loop**
   - `ReasonResult` already flows from reason atom to caller
   - Add `response_id: Option<String>` to `ReasonResult`
   - In `in_memory_loop.rs` / durable loop: capture `response_id` from `ReasonResult`, pass into next `ReasonInput`
   - In `reason.rs`: read `previous_response_id` from `ReasonInput`, set it on `LlmCallConfig`

4. **Populate OTel `response_id`** in `LlmGenerationMetadata` (currently `None`)

**Files touched:**
- `crates/core/src/openresponses_protocol.rs` — extract response_id from events
- `crates/core/src/llm_driver_registry.rs` — add field to `LlmCallConfig`
- `crates/core/src/atoms/reason.rs` — thread response_id in/out
- `crates/core/src/in_memory_loop.rs` — pass response_id between iterations
- `crates/core/src/turn.rs` — optionally store response_id on state machine
- `crates/core/src/events.rs` — populate response_id on generation metadata
- Durable turn executor (worker) — same threading as in_memory_loop

**Tests:**
- Unit: verify response_id extracted from mock SSE stream
- Unit: verify LlmCallConfig carries previous_response_id through iterations
- Integration: full turn loop with mock driver verifying response_id threading

### Phase 2: WebSocket Transport

**Goal:** Add WebSocket connection support inside `OpenResponsesProtocolLlmDriver`. Use it when `previous_response_id` is set and we're hitting the OpenAI API (not custom endpoints).

**New dependency:**
```toml
# crates/core/Cargo.toml
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }
```

**Changes:**

1. **`WebSocketConnection` struct** — new file `crates/core/src/openresponses_ws.rs`
   ```rust
   pub struct WebSocketConnection {
       sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
       stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
       created_at: Instant,
       api_key: String,
   }

   impl WebSocketConnection {
       /// Connect to wss://api.openai.com/v1/responses
       pub async fn connect(api_url: &str, api_key: &str) -> Result<Self>;

       /// Send response.create and return event stream
       pub async fn send_request(
           &mut self,
           body: serde_json::Value,
       ) -> Result<LlmResponseStream>;

       /// Check if connection is still valid (< 60 min)
       pub fn is_valid(&self) -> bool;
   }
   ```

2. **Connection management in driver** (`openresponses_protocol.rs`)
   ```rust
   pub struct OpenResponsesProtocolLlmDriver {
       client: Client,
       api_key: String,
       api_url: String,
       retry_config: LlmRetryConfig,
       /// Active WebSocket connections keyed by turn_id
       ws_connections: Arc<Mutex<HashMap<String, WebSocketConnection>>>,
   }
   ```

3. **Transport selection in `chat_completion_stream`**
   - If `config.previous_response_id.is_some()` AND `api_url` is OpenAI's endpoint:
     - Look up or create WebSocket connection (keyed by `turn_id` from metadata)
     - Send `response.create` with `previous_response_id` + incremental input
     - Return event stream from WebSocket
   - Otherwise: use existing HTTP path (no behavior change)

4. **Event parsing** — WebSocket messages are JSON frames containing the same event types as SSE. Reuse existing `StreamingEvent` parsing from `openresponses_types.rs`. The only difference is framing: SSE `event: type\ndata: json` vs WebSocket text frames with `{"type": "...", ...}`.

5. **Connection cleanup**
   - Remove connection from map when turn completes (Done event or error)
   - Background task to prune expired connections (>55 min)
   - Drop connections on `websocket_connection_limit_reached` error

**Files touched:**
- NEW: `crates/core/src/openresponses_ws.rs` — WebSocket connection + event parsing
- `crates/core/src/openresponses_protocol.rs` — transport selection, connection map
- `crates/core/Cargo.toml` — add `tokio-tungstenite`
- `crates/core/src/lib.rs` — export new module

**Tests:**
- Unit: WebSocket message → LlmStreamEvent parsing
- Unit: transport selection logic (when to use WS vs HTTP)
- Integration: mock WebSocket server, verify full request/response cycle
- Integration: connection reuse across two reason iterations

### Phase 3: Incremental Input

**Goal:** When using WebSocket with `previous_response_id`, send only new input items instead of the full conversation history.

**Changes:**

1. **Track message count from previous call** in driver or config
   - Add `previous_input_count: Option<usize>` to `LlmCallConfig` or track internally
   - On subsequent calls: only include messages after `previous_input_count`

2. **Optimize `build_input()`** — existing method converts `Vec<LlmMessage>` to API input format
   - When `previous_response_id` is set: skip messages already sent, include only new tool results and user messages
   - The turn loop between reason iterations adds tool results — these are the "new" messages

3. **Validate incremental correctness**
   - If WebSocket connection lost, fall back to HTTP with full context
   - If `previous_response_not_found` error (400): retry with full HTTP request

**Files touched:**
- `crates/core/src/openresponses_protocol.rs` — incremental input building
- `crates/core/src/openresponses_ws.rs` — error handling for `previous_response_not_found`
- `crates/core/src/atoms/reason.rs` — pass message offset information

**Tests:**
- Unit: incremental input building (only new messages included)
- Unit: fallback on `previous_response_not_found`
- Integration: multi-iteration turn with incremental input verification

### Phase 4: Advanced Features

**Goal:** Production hardening. Can be done incrementally.

1. **Warmup (`generate: false`)**
   - Send first request with `generate: false` to pre-load context
   - Useful for long system prompts — start WebSocket early while building input
   - Add `warmup()` method to `WebSocketConnection`

2. **Reconnection**
   - On `websocket_connection_limit_reached` (60 min): auto-reconnect
   - Fall back to HTTP for current request, establish new WebSocket for next
   - Exponential backoff on connection failures

3. **Connection pooling (if needed)**
   - Multiple concurrent turns (different sessions) could share a pool
   - Probably unnecessary initially — one connection per turn is fine

4. **Metrics & observability**
   - Track WebSocket vs HTTP usage in `LlmCompletionMetadata`
   - Add `transport: Option<String>` ("websocket" | "http") to metadata
   - Log connection lifecycle events (connect, disconnect, reconnect)
   - OTel spans for WebSocket connection establishment

5. **Feature flag**
   - `EVERRUNS_OPENAI_WEBSOCKET=true` env var to enable (default off initially)
   - Allows gradual rollout and easy rollback

## Error Handling

| Error | Source | Handling |
|---|---|---|
| WebSocket connect failure | Phase 2 | Fall back to HTTP, log warning |
| `previous_response_not_found` (400) | Phase 3 | Retry with full HTTP request |
| `websocket_connection_limit_reached` (400) | Phase 4 | Reconnect, use new connection |
| WebSocket frame error | Phase 2 | Close connection, fall back to HTTP |
| Rate limit (429) | Phase 2 | Existing retry logic applies to HTTP fallback |

All errors fall back to HTTP — WebSocket is a pure optimization, never a hard requirement.

## Configuration

```rust
/// WebSocket mode configuration
pub struct WebSocketConfig {
    /// Enable WebSocket transport for OpenAI Responses API
    /// Default: false (behind feature flag initially)
    pub enabled: bool,

    /// Maximum connection age before proactive reconnection
    /// Default: 55 minutes (5 min buffer before 60 min server limit)
    pub max_connection_age: Duration,

    /// Whether to use warmup (generate=false) on first request
    /// Default: false
    pub warmup_enabled: bool,
}
```

## When WebSocket Mode Helps (and When It Doesn't)

**High value (use WebSocket):**
- Agent turns with 5+ tool call iterations (reason→act→reason cycles)
- Large context windows where incremental input saves bandwidth
- Latency-sensitive real-time interactions

**Low value (stick with HTTP):**
- Single-shot completions (no tool calls)
- Non-OpenAI providers (Anthropic, Gemini, custom endpoints)
- Short conversations with small context

The driver makes this decision automatically based on whether `previous_response_id` is available.

## Migration / Rollout

1. Phase 1 ships first — pure plumbing, zero risk, independently valuable for OTel
2. Phase 2 behind feature flag (`EVERRUNS_OPENAI_WEBSOCKET=false` default)
3. Phase 3 only activates when Phase 2 is enabled
4. Flip flag to `true` after validation in staging
5. Phase 4 items are post-GA hardening

## Open Questions

1. **Custom OpenAI-compatible endpoints** — Should WebSocket be attempted for non-`api.openai.com` URLs? Probably not initially; only enable for known OpenAI endpoints.
2. **Durable execution recovery** — If a worker crashes mid-turn and another picks up, the WebSocket connection is lost. HTTP fallback handles this naturally, but we lose the latency benefit for that turn.
3. **Compact + WebSocket interaction** — When compaction fires mid-turn (context too large), should we close the WebSocket and fall back to HTTP for that compacted request? Yes — compaction changes the input shape, so `previous_response_id` may not apply.
