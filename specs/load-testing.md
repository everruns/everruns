# Load Testing Specification

## Abstract

End-to-end load testing framework for the Everruns API. Measures throughput, latency, and reliability under concurrent session/message workloads using the llmsim provider.

## Goals

1. Repeatable benchmarks with saved checkpoints for regression detection
2. Automatic server metadata capture (version, target, worker topology)
3. Named bench types for different test scenarios
4. Comparison against previous runs of the same bench

## Non-Goals

1. LLM provider benchmarking (llmsim isolates server performance)
2. Client-side performance (SDK overhead is negligible)
3. Infrastructure provisioning (assumes running server)

## Architecture

Single benchmark binary at `crates/server/benches/load_test.rs`. Uses `everruns-sdk` for agent operations and raw `reqwest` for session creation (SDK lacks `harness_id`) and SSE streaming.

### Turn Completion via SSE

Each session opens a single SSE connection (`GET /v1/sessions/{id}/sse`) after creation. Turn completion is detected by waiting for the `session.idled` event on this stream. No polling — one persistent connection per session replaces the previous approach of polling `GET /sessions/{id}` and `GET /sessions/{id}/messages` every 50ms.

### Flow

1. Fetch server info from `/health` and `/v1/durable/health`
2. Create a load test agent with llmsim as default model
3. Spawn N concurrent sessions (controlled by semaphore)
4. Each session:
   a. Creates session via API
   b. Opens SSE stream (`GET /v1/sessions/{id}/sse`)
   c. Sends M messages sequentially
   d. After each message: waits for `session.idled` SSE event (turn complete)
   e. Measures latency from POST /messages to `session.idled` received
5. Collect latency, throughput, and error metrics
6. Optionally save checkpoint with full metadata

## Configuration

All configuration via environment variables, overridden by CLI args where applicable.

| Variable | Default | Description |
|---|---|---|
| `API_URL` | `http://localhost:9300/api` | Server API endpoint |
| `SESSIONS` | `100` | Number of parallel sessions |
| `MESSAGES_PER_SESSION` | `50` | Messages per session |
| `MODEL_ID` | seed llmsim-latency model | Model to use (default includes TTFT + streaming delays) |
| `MAX_CONCURRENT` | `50` | Max concurrent sessions |
| `TIMEOUT_SECS` | `300` | Per-request timeout |
| `TARGET` | auto-detected | Target label (e.g., `dev`, `docker-example`) |

### CLI Arguments

| Argument | Description |
|---|---|
| `--save` | Save results to `crates/server/benches/checkpoints/` |
| `--bench-name <NAME>` | Bench name, defaults to `throughput` |
| `--moniker <NAME>` | Custom environment moniker |
| `--help` | Show help |

## Bench Names

Named benches distinguish different test scenarios. Comparisons only match runs with the same bench name.

| Name | Description | Typical Config |
|---|---|---|
| `throughput` | Baseline throughput (default) | 100 sessions, 50 messages |
| `history-depth` | Message history scaling | 1 session, 200+ messages |
| `horizontal-scale` | Worker scaling | Fixed load, vary workers |
| `burst` | Burst capacity | High concurrency, low messages |

## Justfile Profiles

Profiles set session/message/concurrency defaults. CLI args pass through.

```bash
just load-test quick              # 10 sessions, 10 messages
just load-test medium             # 100 sessions, 50 messages (default)
just load-test heavy              # 500 sessions, 100 messages
```

## Checkpoints

Saved to `crates/server/benches/checkpoints/` as JSON. Filename format: `{bench_name}_{sessions}_{moniker}_{datetime}.json`.

### Checkpoint Schema

```json
{
  "id": "uuid-v7",
  "bench_name": "throughput",
  "timestamp": "2026-02-22T18:00:00Z",
  "server": {
    "version": "0.8.0",
    "auth_mode": "None",
    "target": "docker-example",
    "workers": 3,
    "active_workers": 3,
    "total_capacity": 30
  },
  "environment": {
    "moniker": "local-M4-Pro-48GB",
    "os": "Darwin 15.5",
    "cpu_name": "Apple M4 Pro",
    "cpu_cores": 14,
    "memory_gb": 48.0,
    "hostname": "MAC-P73L96T"
  },
  "config": { "..." },
  "metrics": {
    "sessions_created": 10,
    "sessions_completed": 10,
    "messages_sent": 100,
    "messages_completed": 100,
    "throughput_msg_per_sec": 26.6,
    "latency_p50_ms": 338.0,
    "latency_p95_ms": 686.0,
    "latency_p99_ms": 794.0,
    "..."
  },
  "errors": []
}
```

### Target Auto-Detection

When `TARGET` is not set, inferred from URL and worker count:
- `localhost` / `127.0.0.1` -> `local-{N}w`
- Remote host -> `remote-{N}w`

Explicit `TARGET` overrides auto-detection.

### Comparison

When saving, the tool compares against previous runs with the **same bench name**. Warns on target mismatch between runs.

## Usage Examples

```bash
# Quick smoke test
just load-test quick

# Save results against dev mode
just load-test quick --save

# Save against docker example stack with explicit target
TARGET=docker-example just load-test medium --save

# Named bench: history depth test
SESSIONS=1 MESSAGES_PER_SESSION=200 just load-test quick --save --bench-name history-depth

# Heavy load with custom moniker
just load-test heavy --save --moniker ci-4cpu-8gb
```

## Latency Simulation

Load tests default to the `llmsim-latency` seed model, which simulates realistic LLM streaming behavior:

- **TTFT (Time To First Token)**: Sampled from `LatencyProfile::fast()` before the first token
- **TBT (Time Between Tokens)**: Sampled from `LatencyProfile::fast()` between each streamed word

This measures end-to-end server performance under conditions closer to real LLM usage, where streaming responses arrive over time rather than instantly. The llmsim driver detects the `-latency` suffix in the model name and enables latency simulation automatically.

To bypass latency simulation (e.g., for pure server overhead measurement), override the model: `MODEL_ID=model_01933b5a000070008000000000000401` (the `llmsim-default` instant model).
