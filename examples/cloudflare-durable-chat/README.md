# Durable chat on Cloudflare Durable Objects

A standalone Cloudflare Worker application that runs the Everruns WASM engine
**inside a native Cloudflare Durable Object**. Each shared room uses the DO's
SQLite database, durable alarms and hibernatable WebSockets. No external
runtime, container, daemon or sibling example is required.

This recreates [Earendil's multiplayer chat tutorial](https://earendil.com/posts/pi-durable-on-cloudflare/)
using Everruns. The [walkthrough](../../docs/framework/examples/durable-chat.md)
explains conversations, forks, receipts, scheduled work and the recoverable
history-search child.

## Run

From this directory, with Rust and Node.js 22 or newer installed:

```sh
rustup target add wasm32-unknown-unknown
cargo install worker-build --version 0.8.7 --locked
npm ci
npm run build
cp .dev.vars.example .dev.vars
npm run dev
```

Add your OpenRouter API key to `.dev.vars` before starting. The default model is
`meta/muse-spark-1.3-contributor`; live runs incur provider charges.
`OPENROUTER_BASE_URL` and `MODEL`
select another Chat Completions endpoint and model. Requests cap output at
4,096 tokens unless the engine supplies an explicit limit.

Open the printed local URL, choose a display name and create a conversation.
Copy the room link into a second browser. Messages are queued durably, and
both participants see the same committed transcript. Ask for a reminder in
30 seconds, close the browsers, and reopen after the deadline. In another
conversation, ask about a fact recorded earlier to exercise the search child.

## Hosting boundary

- `src/worker.rs` implements the native Cloudflare Worker entrypoint and
  `ChatRoom` Durable Object with the official `worker` Rust SDK.
- `src/engine.rs` adapts the portable `everruns-core` Reason/Act kernel to
  conversation checkpoints. It is part of this package.
- `src/chat.rs` defines room state, command receipts, forks and tools.
- `src/openai.rs` calls the provider through native Worker `fetch`.
- `src/chat.html` is the browser client.

The Rust manifest depends on Everruns crates under `../../crates` and registry
packages. It does not depend on another example or on celld. Wrangler is the
only direct npm dependency. `npm run check:hosting` audits the resolved Rust
and npm dependencies, the current DO binding, and the built Worker exports.

The deployment migration history keeps the existing `ChatRoom` namespace and
retires an unused minimal-agent class from the initial deployment. The
application binds only `ROOMS` to `ChatRoom`.

## Verify

```sh
cargo test
npm run build
npm test
npx wrangler deploy --dry-run
```

Native tests reopen persisted state after interrupting a step. The end-to-end
test uses real workerd, SQLite, WebSockets and alarms, kills the runtime during
a search, and restarts it without reconnecting a client. A separate check
restarts before a scheduled deadline and confirms one reminder delivery.
Scripted model responses appear only in tests. Tests use temporary
configuration and persistence directories and never edit `.dev.vars`.

## Deploy

```sh
npx wrangler login
npm run deploy
npx wrangler secret put CHAT_ACCESS_TOKEN
npx wrangler secret put OPENROUTER_API_KEY
```

The optional `CHAT_ACCESS_TOKEN` secret requires a shared access key on every
route. Open `/?access=YOUR_KEY`; the page sets an HttpOnly cookie for APIs and
WebSockets. Invitation links include this key. API clients may use it as a
Bearer credential. Without this secret the demo is open.

## Limits

This is an experimental advanced host example, not a stable Framework storage
adapter. Room state is a JSON document in one SQLite row, read and rewritten
on each mutation. There is no automatic compaction, production quota system,
individual identity or weather tool. Anyone with a room invitation can read
and steer the room using the configured model credential.

Interrupted steps may repeat external side effects; any additional tools need
their own idempotency or reconciliation. Recurring ticks missed during downtime
collapse into one follow-up. A fork copies its committed history prefix.
