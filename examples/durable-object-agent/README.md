# An ordinary Rust agent inside a Durable Object

This source-only example compiles the shared Everruns host, actor recovery driver,
capability API and tool loop to Wasm. JavaScript supplies authority storage, alarms
and model fetch; it contains no agent planning. Cloudflare Workers and celld use
the same module. The older `celld-engine` example demonstrates a smaller custom
step machine; this example exercises the ordinary runtime composition.

## Run with a real model

Requires Rust 1.96, Node 24, the wasm32 target and wasm-bindgen-cli 0.2.129:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cd crates/durable-object/worker && npm ci && cd ../../..
bash examples/durable-object-agent/build.sh
cd examples/durable-object-agent
umask 077
printf 'AUTHORITY_TOKEN=%s\n' "$(openssl rand -hex 32)" > .dev.vars
# Add OPENAI_API_KEY to .dev.vars without committing it.
../../crates/durable-object/worker/node_modules/.bin/wrangler dev
```

The default provider is OpenAI with `gpt-6-luna`. `MODEL` and `OPENAI_BASE_URL`
are deployment settings. Authenticate and authorize users in your application;
the authority token is a privileged server credential. Bind a typed session to
the agent/build in generated `worker/build.json`, then admit an `input.message`
through the authority API described by the adapter source. Retain that message
identity when retrying an uncertain acknowledgement.

The `bump_counter` tool updates session KV. Its at-most-once recovery policy
interrupts an operation whose effect may already have happened; it never blindly
repeats the increment. The same policy is used on the native worker. The example
uses the shared Chat Completions driver, including SSE parsing, options, retries
and usage metadata. Its injected transport streams bounded response bytes and
cancels fetch when the driver drops the body.
Native filesystem/process tools require an explicitly supported isolate backend.

The build ID hashes source, manifests, locks and toolchain using relative paths.
Retain the matching immutable Worker build while sessions depend on it. Redeploying
an incompatible build suspends execution instead of silently changing agents.

## Qualify a backend

The test-only scripted provider and fault injection live under
`crates/durable-object/worker/test`, separate from this real-model example.
The test commits a KV effect, loses its owner before tool completion, makes no
application requests during recovery, and requires a completed turn on its first
read. It also checks admission deduplication, immutable builds and stale writes.

Cloudflare qualification uses `test/qualify-cloudflare.sh` from the adapter Worker.
It deploys a uniquely named temporary Worker with a fresh random credential,
runs only synthetic data, and removes its Worker and namespace afterward. It
requires explicit deployment authorization and a configured Cloudflare account.
Run `npm run test:isolate` and `npm run test:celld` in the adapter Worker
folder after building. Celld qualification requires `celld` 0.6.2 and `s3s-fs`
0.14.1 (`cargo install s3s-fs --version 0.14.1 --features binary --locked`).
The harness starts three nodes, uses replicated fleet durability, kills the
owner with SIGKILL and stops all its processes. Its S3 filesystem emulator is
serialized to make conditional writes atomic; no cloud credentials are used.
The orphan-alarm scan is set to one second, so the test's quiet interval bounds
recovery independently of application requests. Logs remain at the printed path.

Cloudflare service qualification and the local celld fleet qualify their tested failure modes;
production bucket consistency, resource limits and account policy remain deployment
responsibilities.
