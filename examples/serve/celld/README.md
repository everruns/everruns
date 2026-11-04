# serve with Engine-owned celld sessions

This source-only reference uses the public hosting SPI. The celld and DO adapters
are not published product dependencies; build this example from the repository.

An ordinary [serve](../../../crates/serve) app with
[`serve_celld::start`](../../../crates/serve-celld). A per-session Durable Object
owns canonical state and alarms. A replaceable native container runs the same
Engine used by serve's `/v1` API. The Worker forwards protocol requests and
activation; Rust owns execution, recovery and tool policy.

To run without a container engine, configure matching private variables in the
Worker's ignored `.dev.vars` and the native process environment:

- `TENANT_ID=example` and `AUTHORITY_URL=http://127.0.0.1:9876` (use your celld port).
- Distinct random `AUTHORITY_TOKEN`, `EXECUTOR_TOKEN`, `API_TOKEN` of at least 32 bytes.
- `SERVE_API_KEYS` on the native process matching `API_TOKEN`.
- `OPENAI_API_KEY` for the real model. Use `CONTAINER_ENV=OPENAI_API_KEY` to pass it
  into a deployed container.

```sh
cargo run -p serve-example-celld -- celld --port 8080
# Another terminal, with the matching .dev.vars:
cd examples/serve/celld/worker
npm ci
celld dev wrangler.external.jsonc
```

Use `/v1/sessions` and `/v1/sessions/{id}/messages` with a user bearer token.
Retain a creation `id` and `message_id` across uncertain replies. Canonical
history and outcomes remain in the session authority when the native process
restarts; the DO alarm retries activation without resubmitting input.

For containers, build from the repository root:

```sh
docker build -f examples/serve/celld/Dockerfile -t serve-celld-example .
```

Run `celld dev` from `worker/`. Set `AUTHORITY_URL` to the reachable authority
Worker origin. For a fleet, push an immutable image and pin it in the deployment
configuration. Retain matching builds for active sessions.

`SERVE_WORKSPACE_DIR` provides a separate directory per session; preserve it with
your chosen workspace service. OS tools additionally require an explicit isolated
`Options::microvm` adapter. `Options::environment` can reopen remote workspace and
compute services. The adapter does not enable an uncontained shell in the Engine
process: its authority credentials must remain inaccessible to commands. Journal
durability does not preserve container files. The
local serve catalog is a projection. Channels and schedules require their own
qualified host services; the new profile migration is still being qualified.

The old app-level cell, canonical snapshot and HTTP replay protocol are retired.
The isolate placement is [celld-engine](../../celld-engine), which imports the
same Framework Agent module as Cloudflare. See the
[shared qualification harness](../../../crates/durable-object/worker/test) for
workerd and replicated celld owner-loss proofs.
