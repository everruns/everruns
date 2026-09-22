---
type: Specification
title: "Correlation IDs"
description: "Correlation IDs."
tags:
  - everruns
  - operations
---
# Correlation IDs

Everruns attaches three correlation identifiers to every request and propagates them through async execution and observability systems.

## Identifiers

### Request ID (`request_id`)

A per-HTTP-request opaque identifier.

- **Header**: `X-Request-ID` (de facto standard; used by nginx, AWS API Gateway, GCP, Heroku)
- **Format**: Opaque client-supplied printable ASCII value when `X-Request-ID` is provided (max 256 chars); otherwise a server-generated UUID v4 (e.g. `550e8400-e29b-41d4-a716-446655440000`). Clients are recommended to send UUID v4 values for interoperability.
- **Source**: Extracted verbatim from the incoming `X-Request-ID` header if present and valid; generated as UUID v4 otherwise.
- **Response**: Always echoed back in the `X-Request-ID` response header.
- **Scope**: Single HTTP request → persists into any async durable run triggered by that request.

Why `X-Request-ID` and not W3C `traceparent`? `traceparent` is the official distributed-tracing standard and Everruns supports it automatically via OpenTelemetry. `X-Request-ID` complements it as a human-readable, stable ID for log search and support workflows, clients can inject their own ID and find all related log lines without understanding OTel trace format.

### Session ID (`session_id`)

The persistent conversation identifier that spans multiple turns.

- **Format**: `session_{32-hex}` (see `knowledge/foundations/id-schema.md`)
- **Source**: URL path parameter in session-scoped endpoints.
- **Scope**: All HTTP handlers and background tasks operating on a session record this on their tracing span.

### Durable Workflow Correlation

The session ID doubles as the durable workflow ID (see `knowledge/operations/durable-execution-engine.md`). This means a single `session_id` or `request_id` search in logs covers both the HTTP layer and the async worker execution.

---

## Where IDs Appear

### Tracing Spans / Logs

All HTTP request spans include:

| Field | Source |
|---|---|
| `request_id` | `X-Request-ID` header or generated UUID |
| `session_id` | Set by session-scoped handlers via `Span::current().record(...)` |

The `RequestIdLayer` Tower middleware (see `crates/server/src/middleware/request_id.rs`) generates or extracts the request ID and stores it in request extensions. The `TraceLayer` reads it from extensions when creating the per-request span so it is present on every log line emitted within the request.

In addition, the `http_access_log_layer` middleware (see `crates/server/src/middleware/access_log.rs`) emits one tracing event per HTTP request with `method`, `route` (matched-path template, low-cardinality), `status`, `latency_ms`, and `request_id`. The level is `INFO` for normal responses, `DEBUG` for noise endpoints (`/health`, `/metrics`), and `WARN` for 5xx. This guarantees a single grep on `request_id=<x>` returns the wire-side line plus every child span emitted under the request span. The middleware is applied as a `route_layer` so axum's `MatchedPath` extractor is populated; unmatched (404) requests are not logged via this middleware (the surrounding `TraceLayer` span still emits its own record for them).

### Durable Execution

The `request_id` is stored in `DurableTurnInput` and propagated across all durable tasks (reason, act, tool execution) so worker logs carry the originating request ID even when executed asynchronously in a separate worker process.

### OpenTelemetry

`request_id` and `session_id` are span fields on all HTTP spans. OTel exporters include them as span attributes, making them queryable in Jaeger, Honeycomb, Grafana Tempo, and similar backends.

The W3C `traceparent` / `tracestate` headers are handled automatically by the OTel SDK (see `knowledge/operations/observability.md`) and provide a parallel, standard-format correlation path.

---

## Middleware Stack Position

```
RequestIdLayer            ← outermost: extracts/generates request_id, stores in extensions, echoes in response
  prometheus http_metrics_layer  ← records HTTP request duration histogram with matched-path labels
  http_access_log_layer   ← emits one INFO/WARN/DEBUG event per request with method, route, status, latency_ms, request_id
    TraceLayer            ← reads request_id from extensions, creates span with request_id + session_id fields
      Security headers
      CORS
        Handlers          ← record session_id on span; extract RequestId extension for CreateMessageContext
```

`RequestIdLayer` must sit outside everything that reads the ID: `TraceLayer`,
which needs it when the span is created, and `http_access_log_layer`, which
reads the extension directly. The access log and prometheus middleware are
applied with `route_layer` so axum's `MatchedPath` extractor is populated before
they run; completely unmatched paths (404 outside any route) are therefore not
logged via the access-log middleware, though the `TraceLayer` span still records
them. The access-log event is emitted outside that span rather than under it,
which is why the middleware mirrors `request_id` as its own field instead of
inheriting it.

**The ordering trap.** Each `.layer()` / `.route_layer()` call wraps *outside*
what came before, so the stack above is built bottom-up and the outermost
middleware is the one applied **last**. Adding a new middleware that reads
`RequestId` at the end of the chain puts it outside `RequestIdLayer`, where the
extension does not exist yet and `unwrap_or_default()` silently yields `""`.
That is EVE-1075: every production access-log line carried `request_id=""` for
as long as the access log was applied after `RequestIdLayer`. Nothing caught it
because the middleware's own tests injected the extension by hand and mounted
one layer. `the_logged_request_id_is_the_one_the_response_echoes` in
`crates/server/src/middleware/access_log.rs` now mounts both and pins the order.

---

## Implementation

| Component | File |
|---|---|
| `RequestIdLayer` Tower middleware | `crates/server/src/middleware/request_id.rs` |
| `http_access_log_layer` middleware | `crates/server/src/middleware/access_log.rs` |
| Middleware wiring + custom `TraceLayer` span | `crates/server/src/app_builder.rs` |
| `request_id` in `CreateMessageContext` | `crates/server/src/domains/messages/service.rs` |
| `request_id` in `AgentRunner::start_run` | `crates/worker/src/runner.rs` |
| `request_id` in `DurableTurnInput` | `crates/worker/src/durable_runner.rs` |
| `session_id` span recording | `crates/server/src/api/messages.rs` |

---

## Reverse Proxy Requirements

For `X-Request-ID` to work end-to-end, the reverse proxy must forward it unchanged. See `knowledge/operations/production-deployment.md` for the full header-forwarding contract.

If the proxy strips or rewrites the header, clients lose the ability to inject their own IDs and the echoed ID in responses will be a server-generated UUID rather than the client-supplied value.
