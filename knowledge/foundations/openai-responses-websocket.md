---
type: Specification
title: "OpenAI Responses WebSocket Transport"
description: "Opt-in WebSocket transport for the OpenAI Responses driver: the wire contract it implements, when a call uses it, and how it falls back to SSE."
tags:
  - everruns
  - foundations
  - providers
  - openai
---
# OpenAI Responses WebSocket Transport

## Abstract

OpenAI's Responses API can be driven over one persistent WebSocket to
`/v1/responses` instead of one HTTP request per turn. A tool loop then
continues from connection-local state instead of reloading the previous
response, which OpenAI reports as up to roughly 40% faster end to end on
rollouts with 20+ tool calls, and recommends for `service_tier: "ultrafast"`.
Everruns offers it as an opt-in transport of the Open Responses driver. SSE
stays the default and the fallback.

Source: [`websocket.rs`](../../crates/provider/src/openresponses_protocol/websocket.rs)
(policy), [`websocket_transport.rs`](../../crates/provider/src/openresponses_protocol/websocket_transport.rs)
(transport, behind the `responses-websocket` feature of `everruns-provider`),
and [`tests_websocket.rs`](../../crates/provider/src/openresponses_protocol/tests_websocket.rs)
(mock-server tests).

## Wire contract

Pinned from OpenAI's [WebSocket mode guide](https://developers.openai.com/api/docs/guides/websocket-mode),
the [WebSocket events reference](https://developers.openai.com/api/reference/resources/responses/websocket-events)
and the [Ultrafast mode guide](https://developers.openai.com/api/docs/guides/ultrafast-mode),
read 2026-10-02.

- **Endpoint and handshake.** The HTTP Responses URL with its scheme mapped
  (`https` to `wss`): `wss://api.openai.com/v1/responses`, authenticated with
  the same headers as HTTP (`Authorization: Bearer …`). *Inferred:* the guide
  names only "a persistent connection to /v1/responses"; the URL derivation
  and header-based auth are taken from the official `openai-python` SDK's
  `responses.connect()` (`_prepare_url` and `_connect_ws`), not from prose.
- **Client event.** One JSON text frame per turn,
  `{"type": "response.create", …}`, carrying the same top-level fields as
  `POST /v1/responses`. `stream` is implicit and is not sent; `background` is
  not supported over the socket.
- **Server events.** The same JSON payloads as HTTP streaming, one per text
  frame, without SSE framing: `response.created`, deltas, item events, then a
  terminal `response.completed`, `response.incomplete` or `response.failed`.
- **Errors.** A WebSocket-only `error` event,
  `{"type": "error", "status": 400, "error": {"type", "code", "message", "param"}}`.
  Unlike the SSE `error` event it carries a `status` and no `sequence_number`.
- **Continuation.** The next turn sends `previous_response_id` and only the
  new input items. The service keeps the latest response of each lane in a
  connection-local cache; an uncached id hydrates from storage when
  `store=true` and fails with `previous_response_not_found` when it is not.
- **Lanes.** `stream_id` multiplexes ordered lanes over one socket (up to 16
  in-flight responses, 32 named lanes). Everruns uses only the implicit default
  lane and one request per socket at a time, so it sends no `stream_id`.
- **Limits.** A connection lasts at most 60 minutes
  (`websocket_connection_limit_reached`).

Not documented, so not relied on: the server's idle timeout and ping cadence,
and whether closing a socket cancels a response still generating on it.

## When a call uses it

Three conditions, all required:

1. The `everruns-provider` crate is built with `responses-websocket`
   (`everruns-openai` enables it).
2. The driver declares support. The OpenAI driver does so for
   `api.openai.com` only; Azure OpenAI, OpenRouter and custom gateways stay on
   SSE, since only OpenAI documents the mode.
3. The call opts in: the `openai/websocket` driver option set to `true`, or
   the provider built with `OpenAIChatDriver::with_websocket_transport(true)`
   and the call not opting out with `false`.

A [background-mode](llm-drivers.md#background-mode-openai-responses) call
never uses the socket, because the WebSocket API rejects `background`. The
default, including for `ultrafast`, is unchanged: SSE.

## Fallback and commit point

The transport sends `response.create` and waits up to 20 seconds for the first
server event. Until that event arrives nothing is committed, and any failure
sends the same request over SSE instead: a failed dial or handshake, a socket
that closes or stalls, and an `error` event (a rate limit, a rejected request,
an unknown previous response). Reusing HTTP for these keeps one copy of the
status classification, the 429 retry policy and the stateless recovery for a
rejected continuation. The cost is one extra round trip on a rejected request.

Once the first event is forwarded the stream is committed. A later drop is
reported as a stream error, like a mid-stream SSE failure, and never re-sent:
the consumer may already have acted on what it received. *Residual:* a socket
that dies after OpenAI accepted the request but before its first event arrives
falls back to SSE, which can bill the generation twice; the 20-second window
and `response.created` arriving before generation keep that window small.

## Connection reuse

A socket whose response completed is parked in a process-wide pool keyed by
the connection identity and that response's id. The next call that continues
from that id (the next turn of the tool loop) takes the socket back and
sends only the new items, which is the case the connection-local cache
speeds up. Any other call dials its own socket, so two sessions never share
one and a checkout is exclusive.

- **Identity** hashes the socket URL and every handshake header, credentials
  included, so a socket is never reused under another key. The pool is
  process-wide because drivers are rebuilt on every step.
- **Idle sockets** are held by a task that answers pings until checkout, a
  server close, or 120 seconds of idleness, then sends a close frame. A
  socket the server closed while parked is replaced by a fresh dial on the
  next turn, and older than 55 minutes is not reused.
- **A dropped stream** (turn cancellation, a stall timeout) closes its socket
  with a close frame rather than parking it. A failed or rejected response
  closes its socket too, since nothing continues from it.

## Security

The base URL is org-configurable, so the dial applies the SSRF guard of the
shared HTTP clients ([TM-LLM-045](../security/threat-model.md)): the host is
resolved, refused when any address is private or internal, and the socket
connects to the resolved addresses. A handshake that does not answer `101` is a
failed dial, so redirects are never followed. Handshake headers are marked
sensitive.

## Follow-ups

- Measure time to first token and end-to-end tool-loop latency, SSE against
  WebSocket, on `gpt-6-astra` with `ultrafast` before considering any default.
- Pre-warming with `generate: false` and mid-turn steering
  (`response.steer`) are not used.
