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

Why `X-Request-ID` and not W3C `traceparent`? `traceparent` is the official distributed-tracing standard and Everruns supports it automatically via OpenTelemetry. `X-Request-ID` complements it as a human-readable, stable ID for log search and support workflows — clients can inject their own ID and find all related log lines without understanding OTel trace format.

### Session ID (`session_id`)

The persistent conversation identifier that spans multiple turns.

- **Format**: `sess_{32-hex}` (see `specs/id-schema.md`)
- **Source**: URL path parameter in session-scoped endpoints.
- **Scope**: All HTTP handlers and background tasks operating on a session record this on their tracing span.

### Durable Workflow Correlation

The session ID doubles as the durable workflow ID (see `specs/durable-execution-engine.md`). This means a single `session_id` or `request_id` search in logs covers both the HTTP layer and the async worker execution.

---

## Where IDs Appear

### Tracing Spans / Logs

All HTTP request spans include:

| Field | Source |
|---|---|
| `request_id` | `X-Request-ID` header or generated UUID |
| `session_id` | Set by session-scoped handlers via `Span::current().record(...)` |

The `RequestIdLayer` Tower middleware (see `crates/server/src/middleware/request_id.rs`) generates or extracts the request ID and stores it in request extensions. The `TraceLayer` reads it from extensions when creating the per-request span so it is present on every log line emitted within the request.

### Durable Execution

The `request_id` is stored in `DurableTurnInput` and propagated across all durable tasks (reason, act, tool execution) so worker logs carry the originating request ID even when executed asynchronously in a separate worker process.

### OpenTelemetry

`request_id` and `session_id` are span fields on all HTTP spans. OTel exporters include them as span attributes, making them queryable in Jaeger, Honeycomb, Grafana Tempo, and similar backends.

The W3C `traceparent` / `tracestate` headers are handled automatically by the OTel SDK (see `specs/observability.md`) and provide a parallel, standard-format correlation path.

---

## Middleware Stack Position

```
RequestIdLayer      ← outermost: extracts/generates request_id, stores in extensions, echoes in response
  TraceLayer        ← reads request_id from extensions, creates span with request_id + session_id fields
    CORS
    Security headers
      Handlers      ← record session_id on span; extract RequestId extension for CreateMessageContext
```

The `RequestIdLayer` must sit outside `TraceLayer` so the span has the ID when it is created.

---

## Implementation

| Component | File |
|---|---|
| `RequestIdLayer` Tower middleware | `crates/server/src/middleware/request_id.rs` |
| Middleware wiring + custom `TraceLayer` span | `crates/server/src/app_builder.rs` |
| `request_id` in `CreateMessageContext` | `crates/server/src/services/message.rs` |
| `request_id` in `AgentRunner::start_run` | `crates/worker/src/runner.rs` |
| `request_id` in `DurableTurnInput` | `crates/worker/src/durable_runner.rs` |
| `session_id` span recording | `crates/server/src/api/messages.rs` |

---

## Reverse Proxy Requirements

For `X-Request-ID` to work end-to-end, the reverse proxy must forward it unchanged. See `specs/production-deployment.md` for the full header-forwarding contract.

If the proxy strips or rewrites the header, clients lose the ability to inject their own IDs and the echoed ID in responses will be a server-generated UUID rather than the client-supplied value.
