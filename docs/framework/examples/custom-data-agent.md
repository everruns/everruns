---
title: Custom Data Agent Worker
description: Package a Framework agent with private database tools and connect it to an Everruns control plane while keeping the ordinary chat and tracing surfaces.
---

This example turns an ordinary Framework agent into an experimental
customer-hosted worker. Its `sales_summary` function reads its own SQLite
database. An OpenAI `gpt-6.1-sol` model chooses the tool arguments and composes the
answer. Users talk to the Everruns control plane, not to the worker directly.

Follow [Run a custom agent worker](/how-to/custom-agent-workers/) for complete
provisioning, TLS, credential rotation and smoke-test instructions. The
[source and README](https://github.com/everruns/everruns/tree/main/examples/custom-data-agent)
contain the executable and container packaging.

## Follow the code

| Source | What it owns |
| --- | --- |
| `src/main.rs` | Worker lease loop, CP journal reads, Framework agent/tool composition, canonical event forwarding and cancellation. |
| `src/transport.rs` | Application-owned OpenAI transport with a fixed HTTPS origin, redirect refusal and deployment proxy support. |
| `smoke.py` | End-to-end verification against a real CP, built-in worker and model, using either a process or container. |
| `Dockerfile` | Source-built release image. |
| `Dockerfile.runtime` | Package an already-built Linux binary against a compatible runtime for smoke/CI use. |

The worker opens the database with SQLite's read-only flag. Model input supplies
only the region, bound as a query parameter. It cannot choose a file path or send
arbitrary SQL. For a real data service, apply your application's own authorization
before querying and return only fields that session readers may see.

## State and events

Each lease reads the canonical CP event log and selects the next unconsumed user
input. The example reconstructs conversation context in a fresh in-memory Engine.
The CP remains the durable record; a second private Engine checkpoint is not
claimed. Completed final answers can finish after a crash without rerunning the
model. Other interrupted reads can replay under the same CP turn, with a bounded
attempt count.

The adapter subscribes before sending to the Framework. It forwards generations,
tool calls, streaming output and terminal events, replaces local session/input
correlation with CP correlation and reconstructs streamed text prefixes omitted
by the facade. A lost observer event is an error, not a successful acknowledgment.
The server scopes trace ids to the deployment/session/input and sends events
through the normal EventService, so existing SSE, work and trace consumers apply.

Every exit path cancels local execution. Renewal happens during execution and
cancellation is checked against the input it targets. The worker acknowledges
only the completed input; input arriving later remains queued.

## Run and verify

After provisioning the CP binding and private database:

```bash
cargo run -p everruns-custom-data-agent
```

Credentials and endpoints come from the deployment environment, as documented in
the how-to. The [full smoke test](/how-to/custom-agent-workers/#5-run-the-full-smoke-test)
checks three real-model turns, live SSE, process/container replacement, changed
private data, credential revocation and rotation. Offline tests cover the SQL
boundary, streaming projection and fixed-origin transport. Neither fixture-only
tests nor a successful compilation establish an end-to-end deployment.

This is a deployment example for advanced hosts. It uses the internal protocol
and canonical event contracts in addition to the public application facade.
Pin the server and worker to the same revision. It is not native durable Engine
hosting, a general-purpose SQL agent, a sandbox, or a Cloud image-hosting service.
