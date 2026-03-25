# Prometheus Metrics Endpoint

Exposes application metrics in Prometheus exposition format at `GET /metrics`.

## Crates

`metrics` 0.24 + `metrics-exporter-prometheus` 0.16 (workspace deps in `Cargo.toml`).

## Endpoint

- **Path:** `/metrics` (root-level, outside API prefix, alongside `/health`)
- **Auth:** None (standard practice; restrict via network policy in production)
- **Content-Type:** `text/plain; version=0.0.4`
- **Implementation:** `crates/server/src/api/prometheus.rs`

## Deployment-Grade Gating

| Grade | Default `METRICS_ENABLED` | Rationale |
|-------|--------------------------|-----------|
| Dev / Poc / Preview | `true` | Always available for development |
| Prod | `false` | SaaS: not exposed externally; opt-in behind network policy |

Operators set `METRICS_ENABLED=true` in production when the `/metrics` port is
restricted to internal scrapers (VPC, service mesh, network policy).

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `METRICS_ENABLED` | grade-dependent | Enable `/metrics` endpoint |
| `METRICS_PREFIX` | `everruns` | Metric name prefix |

## Metrics

All prefixed `everruns_`.

### Gauges (from MetricsCollector bridge, 10s refresh)

| Metric | Description |
|--------|-------------|
| `everruns_workflows_running` | Running workflow count |
| `everruns_workflows_pending` | Pending workflow count |
| `everruns_tasks_pending` | Pending task count |
| `everruns_tasks_claimed` | Claimed task count |
| `everruns_workers_active` | Active worker count |
| `everruns_load_ratio` | System load ratio (0.0-1.0) |
| `everruns_dlq_size` | Dead letter queue size |
| `everruns_tasks_total{status}` | Cumulative task count by status (completed/failed/started) |
| `everruns_workflows_total{status}` | Cumulative workflow count by status |

### Histograms

| Metric | Labels | Source |
|--------|--------|--------|
| `everruns_http_request_duration_seconds` | method, path, status | Axum middleware |
| `everruns_llm_request_duration_seconds` | provider, model | `PrometheusMetricsListener` (llm.generation events) |
| `everruns_tool_execution_duration_seconds` | tool | `PrometheusMetricsListener` (tool.completed events) |

### Scheduled Tasks (future, wire when scheduler lands)

`everruns_schedule_triggers_total{status}`, `everruns_schedule_executions_total{status}`,
`everruns_schedules_active`, `everruns_schedule_trigger_latency_seconds`,
`everruns_schedule_execution_duration_seconds{activity_type}`

## Architecture

1. **Recorder:** `PrometheusBuilder::new().install_recorder()` installs the global
   `metrics` recorder early in `ServerAppBuilder::run()`.
2. **Gauge bridge:** Background task reads latest `MetricsCollector` snapshot every
   10s and sets Prometheus gauges (aligned with existing sampler cadence).
3. **HTTP middleware:** `http_metrics_layer` (Axum `from_fn` middleware) records
   request duration histogram with method/path/status labels.
4. **Event listener:** `PrometheusMetricsListener` implements `EventListener` for
   `llm.generation` and `tool.completed` events, recording duration histograms.
5. **Render:** `GET /metrics` calls `PrometheusHandle::render()`.

## Scrape Config (VictoriaMetrics / Prometheus)

```yaml
scrape_configs:
  - job_name: everruns
    scrape_interval: 15s
    static_configs:
      - targets: ["everruns-server:9301"]
```

## Non-Goals

- No push-based metrics export (use OTLP exporter separately)
- No Grafana dashboards in this change
