# Durable Object authority (experimental)

A source-only native execution profile: a Durable Object owns admission, the
canonical session log, leases and alarms. Ordinary Everruns Rust agents run on an
external executor using the same actor driver as the platform worker. No planner
or tool loop is reimplemented in JavaScript. Not published to crates.io yet.

## Run locally

Requires Node 24, Rust and an OpenAI API key. From the repository root:

```sh
cd crates/durable-object/worker
npm ci
# Create an ignored .dev.vars with distinct random service secrets:
umask 077
printf 'AUTHORITY_TOKEN=%s\nEXECUTOR_TOKEN=%s\n' "$(openssl rand -hex 32)" "$(openssl rand -hex 32)" > .dev.vars
npx wrangler dev
```

Configure `TENANT_ID` and `EXECUTOR_URL` in `wrangler.jsonc`. In another shell,
from the repository root, export the same tenant/secrets and run:

```sh
export AUTHORITY_URL=http://127.0.0.1:8787
export AGENT_BUILD_ID=assistant-v1
cargo run -p everruns-durable-object --example native
```

The example expects `OPENAI_API_KEY`. Its executor listens on loopback port 8790.
A production Worker needs a reachable HTTPS executor and separate production
secrets. Pin immutable code builds and retain the matching executors.

Embed `AuthorityClient` in a trusted authenticated application. Create a typed
session ID, call `session(id).bind(SessionBinding { agent, build_id })`, then
`send(AcceptedTurnInput)`. Keep the accepted input identity when retrying a lost
acknowledgement. The object schedules execution; clients can page `events`,
resolve client calls, cancel or resume a suspended session. This service protocol
is not the public platform `/v1` API. Never expose its credentials to browsers.

Use `Agent::execution_host` for normal Framework agents, or
`serve::Server::execution_host` for registered serve agents, in the executor's
`HostFactory`. The host receives the authority's coherent event/session/KV stores.
Default secret storage fails closed; supply an encrypted backend if needed.
Workspaces and external effects require their own durable contracts. Outer
application lifecycle/approval handlers are owned by the embedding host.

## Verify

```sh
cd crates/durable-object/worker
npm test
npm run test:recovery
```

Recovery uses actual workerd and separate native processes, kills an executor
after a tool effect, and checks alarm-driven recovery without resending input.
It needs free ports 8789 and 8790; logs are retained under the printed `/tmp` path.
Production Cloudflare, multi-node celld and Wasm placement remain unqualified.
See [the architecture](../../knowledge/framework/durable-object-native.md).
