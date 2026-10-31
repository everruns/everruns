# Durable multiplayer chat on Cloudflare

A standalone Cloudflare Worker that runs ordinary Everruns `Agent` and `Engine`
sessions **inside native Durable Objects**. It recreates
[Earendil's multiplayer tutorial](https://earendil.com/posts/pi-durable-on-cloudflare/)
with shared conversations, forks, reminders and a recoverable history-search child.

## Run

Requires Rust 1.96, Node.js 24, the wasm32 target and wasm-bindgen-cli 0.2.129.
From this directory in the full Everruns checkout:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
npm ci
npm run build
cp .dev.vars.example .dev.vars
npm run dev
```

Set `OPENROUTER_API_KEY` before starting. The default model is
`meta/muse-spark-1.3-contributor` through OpenRouter. Provider charges apply, and
the Contributor model uses submitted data for training. `MODEL` and
`OPENROUTER_BASE_URL` are trusted deployment configuration.

Open the local URL, choose a name and create a conversation. Open its room link
in another browser. Record a decision in one conversation and ask about it in
another; the assistant can create a search child. Ask for a reminder, close all
browsers, and return after its deadline. Fork a committed message to explore an
alternative conversation.

## What belongs to Framework

`src/agent.rs` defines ordinary Framework Agents and idempotent tools.
`src/lib.rs` registers them on `Engine::builder().hosting(ObjectHosting::new(...))`.
Framework owns admission, canonical events, turn planning, iteration limits,
tool recovery, waits, cancellation, history and completion metadata.

The application uses the shared `OpenRouterChatDriver` with `with_transport`.
`src/fetch.rs` adapts host response bytes and cancellation; it contains no provider
JSON or SSE parser. `worker/provider.js` supplies endpoint and credential policy.

`ChatSession` uses the shared authority, activation and isolate bridge from
`crates/durable-object`. Each conversation and search child has its own SQLite
journal, Framework binding, execution epoch and durable alarm. There is no custom
turn state machine, celld runtime, container or sibling-example dependency.

The source-only DO adapter implements Framework's public hosting SPI. One Engine
Wasm module also exports the shared command reducer used inside the authority's
storage transaction. The Worker imports `service.js` and installs those exports;
it does not upload a second copy of the reducer. The application does not prepare
trusted command effects.
Each session retains its model at creation. Changing the host's `MODEL` default
affects new sessions; recovery of existing sessions uses their original model.
Pre-pinning memberships use `LEGACY_MODEL`, defaulting to the original Muse
model. Set it to the historical model before upgrading an older custom
deployment; `MODEL` selects the default for new sessions.

## What belongs to the application

`ChatRoom` owns room membership, command receipts, forks, search ownership,
WebSocket presence and reminder delivery. These are application services, separate
from session execution. Room metadata occupies one SQLite row.

Accepted commands commit a receipt and durable outbox intent after arming the
room alarm. Delivery uses Framework `Session::send_with_id` with a stable message
identity. A crash between admission and outbox acknowledgement retries the same
admission. Timers advance together with their outbox entry; missed recurring
ticks collapse into one delivery.

A search child is keyed by the Framework tool execution identity, including the
assistant tool-call message. It is created and messaged through Engine, and owns
an independent activation alarm. A parent replay finds the same child and admission.
Search children and search-generated answers are excluded from future evidence.

A fork imports only the committed user/assistant conversation prefix through
Framework's validated empty-history seed port. It does not copy pending tools,
execution state or timers. Rendering canonical events for the UI is application
code; Framework constructs model context.

## Verify

```sh
npm run build
npm test
npm run check:bundle
```

Tests use actual workerd, SQLite, hibernatable WebSockets and alarms. They kill
workerd while a search is in flight, restart it without reconnecting clients, and
check recovery. A separate restart before a timer deadline checks one reminder
admission. Synthetic model responses exist only in tests. The hosting audit checks
resolved dependencies and requires Framework Engine, ObjectHosting and the shared
driver while refusing a custom runner or sibling-example dependency.

To qualify an Engine upgrade, save the previous build's generated `pkg/` outside
the checkout before building the new revision. Run
`EVERRUNS_CHAT_UPGRADE_FROM=/absolute/path/to/previous-pkg npm test`.
The suite creates journals with that package and switches to the current package
when it restarts the interrupted search. It checks retained history, models,
registrations, child recovery and subsequent reminders. Run this mode separately
from bundle checks because it temporarily swaps generated package files and
restores them on exit.

## Deploy

Check the complete bundle with `npm run check:bundle`. Cloudflare's
[current size limits](https://developers.cloudflare.com/workers/platform/limits/#worker-size)
allow 64 MiB uncompressed on Free and Paid, with no compressed-size cap. Startup,
CPU, memory, request and storage limits remain separate constraints.

```sh
npx wrangler login
npm run deploy
npx wrangler secret put OPENROUTER_API_KEY
```

Internal room/session calls use private Cloudflare Durable Object bindings and
need no service token. The public gateway forwards only the explicit chat routes;
internal tool and session authority routes return 404. Do not expose these DO
fetch handlers through another public gateway. The shared HTTP authority adapter
retains its authentication for deployments that expose an authority service.

The demo has no browser access gate: anyone with its URL can open rooms and send
messages. Room links need no access key or cookie. For an existing deployment,
remove the old gate secret with `npx wrangler secret delete CHAT_ACCESS_TOKEN`.

## Existing demo data and limits

The existing ChatRoom namespace is retained. On first access, the previous room
payload is backed up in `chat_migration_backup`, settled text history is imported
through Framework, and queued messages retain their original IDs. An unfinished
legacy tool call is not replayed under the new runtime. Existing timer metadata
and command receipts are retained. The old namespace must not be deleted.

Build IDs now identify the immutable Agent policy, independently of compatible
Engine/adapter upgrades. `retained-builds.json` names the prior live and review
builds that used the same factory. The build checks that factory's full digest;
do not retarget these keys to changed instructions or tools. A future behavior
revision must retain its previous factory and host tool contracts separately.

Engine selects the persisted registration and validates the runtime format.
Unsupported code or formats suspend execution while keeping history. The room
snapshot exposes compatibility and suspension diagnostics, and the browser shows
a paused state. Restoring code and explicitly calling Framework's checked
`Engine::retry_session` can resume a compatibility suspension; the demo has no
browser retry control or automatic binding-rewrite migration.

This is experimental hosting for small demo rooms. There is no automatic
compaction, individual identity or production quota system. Shared access permits
use of the configured model budget. External tool effects require idempotency or
reconciliation. The chat client displays committed snapshots rather than token
streams. See the [walkthrough](../../docs/framework/examples/durable-chat.md).
