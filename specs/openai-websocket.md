# OpenAI WebSocket Mode Support

## Status: Phase 1 implemented

## Overview

OpenAI's Responses API supports a WebSocket transport (`wss://api.openai.com/v1/responses`) that eliminates per-request HTTP overhead and enables incremental input across multi-turn tool-calling workflows.

- [OpenAI WebSocket Mode Guide](https://developers.openai.com/api/docs/guides/websocket-mode)

| Property | Value |
|---|---|
| Endpoint | `wss://api.openai.com/v1/responses` |
| Auth | `Authorization: Bearer {key}` on upgrade |
| Client→Server | `{"type": "response.create", "response": {...}}` |
| Server→Client | Same events as Responses API SSE |
| Connection limit | 60 min, then `websocket_connection_limit_reached` |
| Concurrency | One in-flight response per connection |
| Continuation | `previous_response_id` + only new input items |
| `store=false` | Only most-recent response in connection-local cache |

## Phase 1: Response ID Plumbing (done)

Threads `response_id` from OpenAI's `response.completed` event through the turn loop so subsequent reason calls send `previous_response_id`. Works cross-worker via HTTP (`store=true`).

**Data flow:**
```
ResponseCompleted.response.id
  → LlmCompletionMetadata.response_id
  → ReasonResult.response_id
  → [in_memory_loop / durable_runner carry forward]
  → ReasonInput.previous_response_id
  → LlmCallConfig.previous_response_id
  → ResponsesRequest.previous_response_id (sent to OpenAI)
  → CompactRequest.previous_response_id (used in compaction)
  → LlmGenerationMetadata.response_id (OTel + Braintrust)
```

## Horizontal Scaling

Workers are stateless (SKIP LOCKED, no affinity). Activities within a turn can land on different workers.

- **`store=true` (default):** `previous_response_id` works cross-worker via HTTP — OpenAI hydrates from disk.
- **`store=false`:** Only works within same WebSocket connection.

Phase 1 benefits all workers. WebSocket (Phase 2+) is a same-worker latency bonus.

## Phase 2: WebSocket Transport (future)

Add `wss://` transport inside `OpenResponsesProtocolLlmDriver` behind `EVERRUNS_OPENAI_WEBSOCKET` flag. Use when `previous_response_id` is set + same worker has a live connection. HTTP fallback on any error.

- New file: `crates/core/src/openresponses_ws.rs`
- Dep: `tokio-tungstenite`
- Connections keyed by `turn_id`, tokio::sync::Mutex map
- Cleanup: prune >55 min connections

## Phase 3: Incremental Input (future)

When using WebSocket + `previous_response_id`, send only new input items. Track sent count per turn.

## Phase 4: Hardening (future)

Warmup (`generate: false`), reconnection, transport metrics, connection pooling.

## Error Handling

All errors fall back to HTTP. WebSocket is never a hard requirement.

| Error | Handling |
|---|---|
| WS connect failure | HTTP fallback |
| `previous_response_not_found` | HTTP retry with full context |
| `websocket_connection_limit_reached` | Reconnect + HTTP fallback |
| WS frame error | Close + HTTP fallback |
