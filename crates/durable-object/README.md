# Durable Object authority (experimental)

An experimental native and Wasm execution profile: a Durable Object owns admission, the
canonical session log, leases and alarms. Ordinary Everruns Rust agents run on an
external executor or inside an object’s Wasm isolate using the same actor driver
as the platform worker. No planner
or tool loop is reimplemented in JavaScript.

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

Configure an Engine with `ObjectHosting::new(client)` and register ordinary
Framework Agents under immutable `AgentBuild` references. Applications use
`Engine::create_registered`, `Session::send_with_id` and reconstructible
admission handles. An authenticated `Executor` enters that same Engine for
native placement; the Wasm application calls `Engine::activate` inside the object.
The privileged authority protocol is internal and must never be exposed to browsers.

Start/completion hooks, durable approvals, client-tool waits, cancellation and
outcome finalization run in Framework under the authority epoch. The profile
supplies coherent event/session/KV storage. Workspace and other host services
require explicit supported contracts; a durable journal does not preserve them.

## Verify

```sh
cd crates/durable-object/worker
npm test
npm run test:recovery
```

Recovery uses actual workerd and separate native processes, kills an executor
after a tool effect, and checks alarm-driven recovery without resending input.
It needs free ports 8789 and 8790; logs are retained under the printed `/tmp` path.
For Wasm builds and platform qualification, see the
[Wasm application](../../examples/durable-object-agent/README.md). Local workerd
coverage does not establish production Cloudflare or cloud-bucket guarantees.
See [the architecture](../../knowledge/framework/durable-object-native.md).

## Extension ports

`AuthorityTransport` supplies tenant/session-scoped authority calls; the native
HTTP implementation and Wasm `ObjectTransport` implement the same protocol.
`ObjectHosting` implements the Framework `SessionHosting` and `SessionEpoch`
ports. Only a claimed epoch supplies canonical write access; frontend bundles
are read-only. Native and Wasm placements invoke the shared `SessionActorDriver`
through Engine. No application host factory assembles a core runtime.
