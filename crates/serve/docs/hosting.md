# Manifest and hosting

> Experimental. See the [README](../README.md) for status.

The idea taken from eve is that the build declares what it needs, and the host
provides it. The user never writes infrastructure config.

## The manifest

`cargo run -- manifest` prints it. The shape is defined in
[`src/manifest.rs`](../src/manifest.rs), and it contains:

- **agents**: each agent's model string, tools, whether it is the default or a
  subagent, and its source file;
- **tools**: each tool's JSON Schema and approval mode (`never`, `always` or
  `conditional`);
- **skills**, **channels** (with their webhook routes), **schedules** (cron)
  and **connections** (MCP URL or type);
- **secrets**: every `Secret::named` that a connection or channel declares,
  plus `serve.toml [secrets] required`;
- **sandbox** kind, **models** for the gateway, **evals**, and **routes**;
- **build_id**: a hash of all of the above plus the embedded `agent/**`
  files, so editing a prompt produces a new build.

The binary produces the manifest, rather than `build.rs`, because only the
linked binary knows what the macros registered.

## What a host does with it

`cargo run -- deploy` prints this plan. This PoC does not execute it.

1. Build the binary and an OCI image, and publish `manifest.json` next to it.
2. Create one cron entry per schedule and one webhook route per channel.
3. Ask the owner for any missing secrets before the first deploy.
4. Provision `DATABASE_URL`, `NATS_URL` and the sandbox adapter, and pass them
   in as environment variables.
5. Point `SERVE_GATEWAY_URL` and `SERVE_GATEWAY_KEY` at the model gateway, so
   the app holds no provider keys.
6. Give each branch a preview URL, and run `eval --against <preview>` before
   promoting it.

## Build pinning, and why restarts are free

Agent behavior (closures, tools, hooks) is code, and code cannot be
serialized. With a plain `everruns::Engine`, the application rebuilds that
behavior after a restart and calls `Engine::attach`. serve does the same thing
automatically: the binary is the behavior. After a restart, the first request
for a session rebuilds its agent from the same registrations, attaches it to
the session's persisted history, and carries on.

Every session records the `build_id` it started on. Across deploys:

- A hosted router sends each session to the build it started on. The old build
  keeps running until its in-flight sessions finish, so a session running
  during a deploy finishes on its original version.
- `start` refuses a session from another build with `409` and
  `x-serve-build: <build_id>`, which is exactly what a router needs.
- `dev` resumes the session anyway (the console notes the build change),
  because in development every edit is a new build.

## Self-hosting

It is the same binary. Set `PORT`, and either `SERVE_DATA_DIR` or
`DATABASE_URL=sqlite:///path`. In the PoC, the everruns local store (the
conversation and the durable event log the wire API serves) and serve's
session catalog live in SQLite under that path; Postgres and NATS adapters are
future work. Run it with `start`.

## Hosting targets

A platform with its own container contract gets a hosting target: a crate of
its own that boots the app through `serve::Server` (the same boot as `start`),
merges `Server::router()` (the `/v1` wire API) with the platform's routes, and
hands every other command back to `serve::start`. serve itself does not change
per platform.

`Server::builder(app, mode)` adds what a target may supply: `.data_dir(dir)`
(nothing persists without one) and `.microvm(f)`, the adapter
`[sandbox] kind = "microvm"` uses when each session already runs in its own
machine. Without an adapter, `microvm` falls back to bashkit, so the same
`serve.toml` is safe under `dev`.

| Target | Crate | Contract |
|---|---|---|
| Amazon Bedrock AgentCore Runtime | [`everruns-serve-agentcore`](../../serve-agentcore) | `GET /ping` (`Healthy`, or `HealthyBusy` while a turn runs), `POST /invocations` (AG-UI `RunAgentInput` or `{"prompt": ...}`, streamed as AG-UI SSE), port 8080, arm64 image. Boots on the first non-ping request, on session storage (`/mnt/workspace`) when mounted; `microvm` is a host shell in the session workspace. See [`examples/serve/agentcore`](../../../examples/serve/agentcore) and [`examples/serve/agentcore-workspace`](../../../examples/serve/agentcore-workspace). |
| celld (self-hosted Durable Objects) | [`everruns-serve-celld`](../../serve-celld) | `GET /celld/state` (boot id, busy), `GET /celld/snapshot` (tar of the data directory, `409` while busy), `PUT /celld/restore` (only before boot), port 8080. Boots on the first other request so the supervising Durable Object can restore first; that object (JavaScript, [`examples/serve/celld/worker`](../../../examples/serve/celld/worker)) keeps the snapshot and a request journal and replays it into a fresh container. `microvm` is a host shell in a workspace inside the data directory. |
