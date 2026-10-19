# An ordinary Rust agent inside a Durable Object

This shared Agent module imports `everruns` and uses ordinary `Agent`, tools and
`Engine` session APIs on native and Wasm. `ObjectHosting` supplies coherent
session authority and activation. Rust owns lifecycle, planning and recovery;
JavaScript supplies storage, alarms and provider fetch. The
[celld bootstrap](../celld-engine) imports this same application.

## Run with a real model

Requires Rust 1.96, Node 24, the wasm32 target and wasm-bindgen-cli 0.2.129:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cd crates/durable-object/worker && npm ci && cd ../../..
bash examples/durable-object-agent/build.sh
cd examples/durable-object-agent
umask 077
printf 'AUTHORITY_TOKEN=%s\nAPI_TOKEN=%s\n' "$(openssl rand -hex 32)" "$(openssl rand -hex 32)" > .dev.vars
# Add OPENAI_API_KEY to .dev.vars without committing it.
../../crates/durable-object/worker/node_modules/.bin/wrangler dev
```

The default provider is OpenAI with `gpt-6-luna`. `MODEL` and `OPENAI_BASE_URL`
are deployment settings. `API_TOKEN` authenticates `/v1`; `AUTHORITY_TOKEN` is a
separate privileged server credential. Use `/v1/sessions` with a stable `id`,
then `/v1/sessions/{id}/messages` with a stable `message_id` and ordinary message
content. Retain identities across uncertain replies. Read `/events`, `/sse` and
`/admissions/{message_id}` through the same Engine path. No application sends
raw canonical `input.message` events or constructs runtime backends.

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
