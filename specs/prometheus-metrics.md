# Prometheus Metrics Endpoint

Exposes application metrics in Prometheus exposition format at `GET /metrics`.

## Crates

`metrics` 0.24 + `metrics-exporter-prometheus` 0.16 (workspace deps in `Cargo.toml`).

## Endpoint

- **Path:** `/metrics` (root-level, outside API prefix, alongside `/health`)
- **Auth:** None (standard practice; restrict via network policy or `METRICS_ADDR`)
- **Content-Type:** `text/plain; version=0.0.4`
- **Implementation:** `crates/server/src/api/prometheus.rs`

## Serving Modes

### Dev (default): main server

When `METRICS_ADDR` is **unset**, `/metrics` is mounted on the main API server.
Convenient for local development — `curl localhost:9301/metrics` just works.

### Production: dedicated internal server

When `METRICS_ADDR` is **set** (e.g. `127.0.0.1:9090`), a separate lightweight
HTTP server is spawned that **only** serves `/metrics`. This keeps the metrics
endpoint off the public interface entirely.

Scrapers inside the cluster reach it via pod-local networking, sidecar, or
service mesh. External traffic never sees the port.

```yaml
# Kubernetes deployment example
env:
  - name: METRICS_ADDR
    value: "127.0.0.1:9090"
```

```yaml
# VictoriaMetrics / Prometheus scrape config
scrape_configs:
  - job_name: everruns
    scrape_interval: 15s
    static_configs:
      - targets: ["localhost:9090"]  # pod-local sidecar access
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `METRICS_ENABLED` | `true` | Enable/disable metrics collection and endpoint |
| `METRICS_ADDR` | *(unset)* | Dedicated bind address for internal-only metrics server. When set, `/metrics` is NOT on the main API server. Recommended for production. |

## Horizontal Scaling Model

Metrics are designed for correct behavior with multiple API server replicas:

| Category | Source | Multi-replica behavior |
|----------|--------|----------------------|
| **Gauges** | DB via MetricsCollector | All replicas emit identical values (same DB). Prometheus deduplicates via `instance` label. |
| **Counters** | Local events (EventListener, HTTP middleware) | Each replica counts only its own work. `sum()` across instances gives true total. |
| **Histograms** | Local observations | Each replica records its own latencies. Prometheus merges across instances. |

Task/workflow DB totals are emitted as **gauges** (not counters) because they
represent global state from the shared database. For rate-like queries, use
`increase(everruns_tasks_completed[5m])` — Prometheus `increase()` works on
gauges too.

## Metrics

All prefixed `everruns_`.

### Gauges (from DB via MetricsCollector bridge, 10s refresh)

| Metric | Description |
|--------|-------------|
| `everruns_workflows_running` | Running workflow count |
| `everruns_workflows_pending` | Pending workflow count |
| `everruns_tasks_pending` | Pending task count |
| `everruns_tasks_claimed` | Claimed task count |
| `everruns_workers_active` | Active worker count |
| `everruns_load_ratio` | System load ratio (0.0-1.0) |
| `everruns_dlq_size` | Dead letter queue size |
| `everruns_tasks_completed` | DB cumulative completed tasks |
| `everruns_tasks_failed` | DB cumulative failed tasks |
| `everruns_tasks_started` | DB cumulative started tasks |
| `everruns_workflows_completed` | DB cumulative completed workflows |
| `everruns_workflows_failed` | DB cumulative failed workflows |
| `everruns_workflows_started` | DB cumulative started workflows |

### Counters (per-instance, from local events)

| Metric | Labels | Source |
|--------|--------|--------|
| `everruns_http_requests_total` | method, path, status | HTTP middleware |
| `everruns_llm_requests_total` | provider, model | EventListener (llm.generation) |
| `everruns_tool_executions_total` | tool | EventListener (tool.completed) |

### Histograms (per-instance, from local observations)

| Metric | Labels | Source |
|--------|--------|--------|
| `everruns_http_request_duration_seconds` | method, path, status | Axum route_layer middleware |
| `everruns_llm_request_duration_seconds` | provider, model | EventListener (llm.generation) |
| `everruns_tool_execution_duration_seconds` | tool | EventListener (tool.completed) |

### Scheduled Tasks (future, wire when scheduler lands)

`everruns_schedule_triggers_total{status}`, `everruns_schedule_executions_total{status}`,
`everruns_schedules_active`, `everruns_schedule_trigger_latency_seconds`,
`everruns_schedule_execution_duration_seconds{activity_type}`

## Architecture

1. **Recorder:** `PrometheusBuilder::new().install_recorder()` installs the global
   `metrics` recorder early in `ServerAppBuilder::run()`.
2. **Gauge bridge:** Background task reads latest `MetricsCollector` snapshot every
   10s and emits Prometheus gauges (aligned with existing sampler). All values are
   global DB state — safe to duplicate across replicas.
3. **HTTP middleware:** `http_metrics_layer` (Axum `route_layer` middleware) records
   per-request counter + duration histogram. Uses `MatchedPath` for low-cardinality
   path labels; unmatched routes labeled `"unmatched"`.
4. **Event listener:** `PrometheusMetricsListener` implements `EventListener` for
   `llm.generation` and `tool.completed` events, recording per-instance counters
   and duration histograms.
5. **Render:** `GET /metrics` calls `PrometheusHandle::render()`.

## Non-Goals

- No push-based metrics export (use OTLP exporter separately)
- No Grafana dashboards in this change
