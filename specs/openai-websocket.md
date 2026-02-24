# OpenAI WebSocket Mode Support

## Status: Draft

## Overview

OpenAI's Responses API supports a WebSocket transport (`wss://api.openai.com/v1/responses`) that eliminates per-request HTTP overhead and enables incremental input across multi-turn tool-calling workflows. The server maintains the previous response in connection-local memory, enabling sub-second continuations without disk I/O.

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
| Auth | `Authorization: Bearer {key}` header on WebSocket upgrade |
| Client→Server message | `{"type": "response.create", "response": {...}}` |
| Server→Client events | Same event types as Responses API SSE stream |
| Connection limit | 60 minutes, then `websocket_connection_limit_reached` |
| Concurrency | Sequential — one in-flight response per connection |
| Multiplexing | Not supported; use multiple connections for parallel runs |
| Continuation | `previous_response_id` + only new input items |
| Warmup | `generate: false` prepares state, returns response ID, no output |
| State with `store=false` | Only most-recent response kept in connection-local cache |

## Current Architecture

### Call path (today)

```
TurnStateMachine loop:
  Reason → Act → Reason → Act → ... → Complete

Each Reason iteration:
  reason.rs:771  →  llm_driver.chat_completion_stream(messages, config)
                         ↓
                 OpenResponsesProtocolLlmDriver
                         ↓
                 HTTP POST /v1/responses (stream=true)
                         ↓
                 SSE byte stream → eventsource_stream → LlmStreamEvent
```

### Key types (verified line numbers)

- `LlmDriver` trait — `llm_driver_registry.rs:100`
- `LlmStreamEvent` enum — `llm_driver_registry.rs:37` (TextDelta, ThinkingDelta, ThinkingSignature, ToolCalls, Done, Error)
- `LlmCallConfig` — `llm_driver_registry.rs:374` (model, temperature, max_tokens, tools, reasoning_effort, metadata)
- `LlmCompletionMetadata` — `llm_driver_registry.rs:77` (tokens, model, finish_reason, retry_metadata — **no response_id**)
- `LlmGenerationMetadata` — `events.rs:843` (has `response_id: Option<String>`, always `None` today)
- `OpenResponsesProtocolLlmDriver` — `openresponses_protocol.rs:73` (client, api_key, api_url, retry_config)
- `CreateResponseBody` — `openresponses_types.rs:544` (has `previous_response_id: Option<String>`)
- `ResponseResource` — `openresponses_types.rs:734` (has `id: String` — the response ID)
- `StreamingEvent::ResponseCompleted` — `openresponses_types.rs:851` (contains full `ResponseResource`)
- `ReasonInput` — `reason.rs:97` (context, harness_id, agent_id, org_id, mcp_tool_definitions)
- `ReasonResult` — `reason.rs:117` (success, text, tool_calls, has_tool_calls, tool_definitions, max_iterations, error, usage)
- `CompactRequest` — `openresponses_protocol.rs:1069` (has `previous_response_id: Option<String>`, always `None`)
- Turn loop — `in_memory_loop.rs:582` (`run_turn` method)
- Reason atom LLM call — `reason.rs:771`

### What already exists

1. `ResponseResource.id` parsed from `response.completed` events — but **discarded** (not captured into `LlmCompletionMetadata`)
2. `LlmGenerationMetadata.response_id` field — always `None` (hardcoded at `reason.rs:1148`)
3. `CreateResponseBody.previous_response_id` field — exists in the type, but **never set** by the driver
4. `CompactRequest.previous_response_id` field — always `None` (`reason.rs:801`)
5. Turn state machine loops Reason→Act→Reason, threading `ReasonResult` between iterations
6. `LlmCallConfig.metadata` HashMap carries `session_id`, `turn_id`, `exec_id` per call

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

**Goal:** Extract and thread `response_id` through the turn loop. No WebSocket yet — required infrastructure, independently valuable for OTel tracing + compaction chaining.

**Changes:**

1. **Add `response_id` to `LlmCompletionMetadata`** (`llm_driver_registry.rs`)
   ```rust
   pub struct LlmCompletionMetadata {
       // ... existing fields ...
       /// Provider's response ID (e.g., OpenAI response ID from response.completed)
       pub response_id: Option<String>,
   }
   ```

2. **Extract `response_id` from `response.completed` event** (`openresponses_protocol.rs:1004`)
   - `ResponseCompleted` already carries `ResponseResource` with `id: String`
   - Set `response_id: Some(response.id)` on `LlmCompletionMetadata` in the `Done` event
   - Currently discarded — one-line fix

3. **Add `previous_response_id` to `LlmCallConfig`** (`llm_driver_registry.rs`)
   ```rust
   pub struct LlmCallConfig {
       // ... existing fields ...
       /// Previous response ID for stateful continuation (OpenAI WebSocket mode).
       /// When set, driver may send only incremental input.
       pub previous_response_id: Option<String>,
   }
   ```

4. **Thread response_id through turn loop**
   - Add `response_id: Option<String>` to `ReasonResult`
   - In reason atom (`reason.rs:1148`): populate from `LlmCompletionMetadata.response_id` instead of `None`
   - In `in_memory_loop.rs:619`: capture `response_id` from `ReasonResult`, add `previous_response_id` field to `ReasonInput`
   - In `reason.rs`: read `previous_response_id` from `ReasonInput`, set on `LlmCallConfig`

5. **Populate OTel `response_id`** in `LlmGenerationMetadata` (`reason.rs:1148`)
   - Pass `completion_metadata.response_id` instead of `None`

6. **Use for compaction** — set `CompactRequest.previous_response_id` from config (`reason.rs:801`)

**Files touched:**
- `crates/core/src/llm_driver_registry.rs` — add field to `LlmCompletionMetadata` and `LlmCallConfig`
- `crates/core/src/openresponses_protocol.rs:1028` — capture `response.id` from `ResponseCompleted`
- `crates/core/src/atoms/reason.rs` — thread response_id in `ReasonInput`/`ReasonResult`, populate on events
- `crates/core/src/in_memory_loop.rs` — pass response_id between Reason iterations
- `crates/worker/src/durable_runner.rs` — same threading as in_memory_loop

**Tests:**
- Unit: verify `response_id` extracted from mock SSE stream containing `response.completed`
- Unit: verify `LlmCallConfig` carries `previous_response_id` through 2+ iterations
- Integration: full turn loop with mock driver verifying response_id → previous_response_id threading

### Phase 2: WebSocket Transport

**Goal:** Add WebSocket connection support inside `OpenResponsesProtocolLlmDriver`. Use it when `previous_response_id` is set and target is `api.openai.com`.

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
   }

   impl WebSocketConnection {
       /// Connect with Authorization header on upgrade
       pub async fn connect(api_url: &str, api_key: &str) -> Result<Self>;

       /// Send {"type": "response.create", "response": {body}} and return event stream
       pub async fn send_request(
           &mut self,
           body: CreateResponseBody,
       ) -> Result<LlmResponseStream>;

       /// Check if connection is still valid (< 55 min)
       pub fn is_valid(&self) -> bool;
   }
   ```

2. **WebSocket event stream adapter** — WebSocket text frames contain the same JSON events as SSE, but without SSE framing (`event: type\ndata: json`). Each text frame is a JSON object with `"type"` field. Reuse existing `StreamingEvent` serde parsing.

3. **Connection management in driver** (`openresponses_protocol.rs`)
   ```rust
   pub struct OpenResponsesProtocolLlmDriver {
       client: Client,
       api_key: String,
       api_url: String,
       retry_config: LlmRetryConfig,
       ws_config: WebSocketConfig,
       /// Active WebSocket connections keyed by turn_id
       ws_connections: Arc<tokio::sync::Mutex<HashMap<String, WebSocketConnection>>>,
   }
   ```
   Use `tokio::sync::Mutex` (not `std::sync::Mutex`) since connection operations are async.

4. **Transport selection in `chat_completion_stream`**
   - If `ws_config.enabled` AND `config.previous_response_id.is_some()` AND `api_url` starts with `https://api.openai.com`:
     - Look up existing WebSocket by `turn_id` from `config.metadata`
     - If none or expired: establish new connection to `wss://api.openai.com/v1/responses`
     - Send `{"type": "response.create", "response": {body_with_previous_response_id}}` with only new input
     - Return event stream from WebSocket text frames
   - Otherwise: existing HTTP path (no behavior change)
   - On **any** WebSocket error: close connection, fall back to HTTP with full context

5. **Connection lifecycle**
   - Create on first WebSocket-eligible call in a turn
   - Reuse across Reason iterations within same turn (keyed by `turn_id`)
   - Remove when turn completes (Done event) or on error
   - Background cleanup task: prune connections older than 55 min

**Files touched:**
- NEW: `crates/core/src/openresponses_ws.rs` — connection, event adapter
- `crates/core/src/openresponses_protocol.rs` — transport selection, connection map
- `crates/core/Cargo.toml` — add `tokio-tungstenite`
- `crates/core/src/lib.rs` — export module

**Tests:**
- Unit: WebSocket JSON frame → `StreamingEvent` → `LlmStreamEvent` parsing
- Unit: transport selection logic (WS vs HTTP conditions)
- Integration: mock WebSocket server, full request/response cycle
- Integration: connection reuse across two Reason iterations in one turn

### Phase 3: Incremental Input

**Goal:** When using WebSocket with `previous_response_id`, send only new input items instead of full conversation history. The server maintains the previous response in connection-local memory — sending `previous_response_id` + only new items avoids retransmitting context.

**Changes:**

1. **Track sent message count** — after each WebSocket call, record how many input items were sent. Store in `LlmCallConfig` or driver-internal state keyed by turn_id.

2. **Compute incremental input in driver** — when `previous_response_id` is set:
   - Convert full message list to input items (existing `convert_message` logic)
   - Skip items up to `previous_input_count`
   - Send only new items (typically: assistant tool_call output items from Act phase)
   - Always include `previous_response_id` in request body

3. **Correctness safeguards**
   - If WebSocket connection lost mid-turn: HTTP fallback sends full context (no `previous_response_id`)
   - If server returns `previous_response_not_found` (400): discard connection, retry via HTTP with full context
   - With `store=false`: only the most recent response is in server cache. If we chain correctly (always using last response_id), this is fine. If we skip one, fall back to HTTP.

**Files touched:**
- `crates/core/src/openresponses_ws.rs` — track sent count, incremental input logic
- `crates/core/src/openresponses_protocol.rs` — pass-through for incremental mode
- `crates/core/src/atoms/reason.rs` — pass total message count for offset tracking

**Tests:**
- Unit: incremental input building (N messages → only last K sent)
- Unit: fallback on `previous_response_not_found` error
- Integration: 3-iteration turn verifying each iteration sends only new items

### Phase 4: Production Hardening

**Goal:** Polish. Ship incrementally.

1. **Warmup (`generate: false`)**
   - On first Reason call of a turn (no `previous_response_id`): open WebSocket, send first request with `generate: false`
   - Returns a response_id immediately (pre-loads system prompt + tools on server)
   - Next Reason call uses this response_id + WebSocket for sub-second continuation
   - Trade-off: extra roundtrip upfront, but amortized over 5+ iterations

2. **Reconnection**
   - On `websocket_connection_limit_reached`: close, reconnect, HTTP fallback for current request
   - Exponential backoff on connection failures (reuse `LlmRetryConfig`)

3. **Metrics & observability**
   - Add `transport: Option<String>` ("websocket" | "http") to `LlmCompletionMetadata`
   - Track in `LlmGenerationMetadata` for OTel: `gen_ai.transport`
   - Log connection lifecycle: connect, reuse, reconnect, close

4. **Feature flag**
   - `EVERRUNS_OPENAI_WEBSOCKET=true` env var (default `false`)
   - Gradual rollout, easy rollback

## Error Handling

| Error | Phase | Handling |
|---|---|---|
| WebSocket connect failure | 2 | Fall back to HTTP, log warning |
| WebSocket frame error | 2 | Close connection, HTTP fallback with full context |
| `previous_response_not_found` (400) | 3 | Discard connection, HTTP retry with full context |
| `websocket_connection_limit_reached` (400) | 4 | Reconnect, HTTP fallback for current request |
| Rate limit (429) | 2 | HTTP fallback triggers existing retry logic |

**Principle:** All errors fall back to HTTP. WebSocket is a pure optimization, never a hard requirement.

## Configuration

```rust
/// WebSocket mode configuration
pub struct WebSocketConfig {
    /// Enable WebSocket transport for OpenAI Responses API
    /// Default: false (env: EVERRUNS_OPENAI_WEBSOCKET)
    pub enabled: bool,

    /// Maximum connection age before proactive reconnection
    /// Default: 55 minutes (5 min buffer before 60 min server limit)
    pub max_connection_age: Duration,

    /// Use warmup (generate=false) on first request to pre-load context
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

Decision is automatic: first call in a turn always uses HTTP (no `previous_response_id`). Subsequent iterations in the same turn use WebSocket when enabled.

## Migration / Rollout

1. **Phase 1** ships first — pure plumbing, zero risk, independently valuable for OTel + compaction
2. **Phase 2** behind feature flag (`EVERRUNS_OPENAI_WEBSOCKET=false` default)
3. **Phase 3** only activates when Phase 2 is enabled
4. Flip flag to `true` after validation in staging
5. **Phase 4** items are post-GA hardening

## Open Questions

1. **Custom OpenAI-compatible endpoints** — Should WebSocket be attempted for non-`api.openai.com` URLs? Probably not initially; only enable for known OpenAI endpoints.
2. **Durable execution recovery** — If a worker crashes mid-turn and another picks up, the WebSocket connection is lost. HTTP fallback handles this naturally, but we lose the latency benefit for remaining iterations.
3. **Compact + WebSocket interaction** — When compaction fires mid-turn (context too large), we must close the WebSocket and fall back to HTTP. Compaction changes the input shape, invalidating `previous_response_id`.
4. **`store=false` state limit** — With `store=false`, only the most recent response exists in connection-local cache. Older response IDs become unrecoverable. Our sequential Reason→Act→Reason loop naturally uses only the last response_id, so this is fine.
