# Durable Object hosting reference

Source-only integration example (`publish = false`), resolved in its own Cargo
workspace. This package is not a supported Cloudflare product API or a Framework
dependency. It demonstrates implementing `SessionHosting` using only the public
`everruns::hosting::spi`, with the agent runtime retained inside Framework.

> Coherent session hosting for ordinary Everruns Agents on native executors and Wasm Durable Objects.

Part of the [Everruns](https://everruns.com) ecosystem.

An experimental native and Wasm execution profile: a Durable Object owns admission, the
canonical session log, leases and alarms. Ordinary Everruns Rust agents run on an
external executor or inside an object’s Wasm isolate using the same actor driver
as the platform worker. No planner
or tool loop is reimplemented in JavaScript.

## What It Provides

- One per-session authority for binding, admission, commands, canonical events and wakeups.
- Epoch-fenced runtime ports and exact-turn approval/client-result commands.
- The same Framework Engine and shared actor driver on native and Wasm placements.

## Quick Example

Enable `everruns/openai` in the application. Configure the Worker and its
independent Engine executor as described below, then admit an ordinary Agent input:

```rust,no_run
use everruns::{Agent, Engine, MessageId, OpenAI, SessionId, hosting::AgentBuild};
use everruns_durable_object::{AuthorityClient, ObjectHosting};

# #[cfg(feature = "native")]
# async fn submit() -> Result<(), Box<dyn std::error::Error>> {
let client = AuthorityClient::new(
    "https://authority.example.com", "tenant", "replace-with-a-long-random-service-secret",
)?;
let agent = Agent::builder()
    .instructions("Answer concisely.")
    .provider(OpenAI::from_env()?)
    .model("gpt-6.1-sol")
    .build()?;
let build = AgentBuild::new("assistant", "immutable-build-v1");
let engine = Engine::builder()
    .hosting(ObjectHosting::new(client))
    .register(build.clone(), agent)
    .build();
let session = engine.create_registered(SessionId::new(), &build).await?;
let receipt = session.send_with_id(MessageId::new(), "Hello.").await?;
println!("admitted {}", receipt.message_id);
# Ok(())
# }
# fn main() {}
```

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
cargo run --manifest-path crates/durable-object/Cargo.toml --example native
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

From the repository root:

```sh
python3 scripts/lib/check-session-hosting-boundary.py
CARGO_TARGET_DIR="$PWD/target" cargo test --locked --manifest-path crates/durable-object/Cargo.toml
```

The independent resolution prevents product-workspace feature unification from
hiding missing SPI dependencies. The workerd harness also runs the same public
conformance scenarios as the built-in authorities, then qualifies actual process
loss and independent wakeup delivery.

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
See the [hosting guide](https://everruns.com/docs/framework/durable-objects/).

## Extension ports

`AuthorityTransport` supplies tenant/session-scoped authority calls; the native
HTTP implementation and Wasm `ObjectTransport` implement the same protocol.
`ObjectHosting` implements the Framework `SessionHosting` and `SessionEpoch`
ports. Only a claimed epoch supplies canonical write access; frontend bundles
are read-only. Native and Wasm placements invoke the shared `SessionActorDriver`
through Engine. No application host factory assembles a core runtime.

## Documentation

- [Durable Object hosting](https://everruns.com/docs/framework/durable-objects/)
- [Public hosting SPI](https://everruns.com/docs/framework/hosting/)
- [Framework architecture](https://everruns.com/docs/framework/architecture/)

## License

MIT. See [LICENSE](https://github.com/everruns/everruns/blob/main/LICENSE).
