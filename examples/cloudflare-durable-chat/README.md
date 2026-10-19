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

Set `OPENROUTER_API_KEY` and replace `AUTHORITY_TOKEN` with an independent random
secret of at least 32 characters before starting. The default model is
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

The source-only DO adapter implements Framework's public hosting SPI. The build
produces both its authority Wasm module and the chat Engine module. Admission,
history seeding and other session commands run shared Rust rules inside the DO's
storage transaction; the application does not prepare trusted command effects.
Each session retains its model at creation. Changing the host's `MODEL` default
affects new sessions; recovery of existing sessions uses their original model.

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
npx wrangler deploy --dry-run
```

Tests use actual workerd, SQLite, hibernatable WebSockets and alarms. They kill
workerd while a search is in flight, restart it without reconnecting clients, and
check recovery. A separate restart before a timer deadline checks one reminder
admission. Synthetic model responses exist only in tests. The hosting audit checks
resolved dependencies and requires Framework Engine, ObjectHosting and the shared
driver while refusing a custom runner or sibling-example dependency.

## Deploy

The current Wasm bundle exceeds Workers Free's 3 MiB compressed script limit;
use a Cloudflare Workers Paid plan.

```sh
npx wrangler login
npm run deploy
npx wrangler secret put AUTHORITY_TOKEN
npx wrangler secret put OPENROUTER_API_KEY
npx wrangler secret put CHAT_ACCESS_TOKEN
```

Use independent random authority and optional shared-access secrets. The Worker
exposes only the room gateway; privileged authority operations are object-local
or authenticated internal calls. They must never be exposed to browsers.
With `CHAT_ACCESS_TOKEN`, open `/?access=YOUR_KEY` to establish an HttpOnly,
Secure, SameSite cookie. Invitation links include that shared key.

## Existing demo data and limits

The existing ChatRoom namespace is retained. On first access, the previous room
payload is backed up in `chat_migration_backup`, settled text history is imported
through Framework, and queued messages retain their original IDs. An unfinished
legacy tool call is not replayed under the new runtime. Existing timer metadata
and command receipts are retained. The old namespace must not be deleted.

Build IDs hash application and shared runtime sources, manifests and locks. Keep
matching immutable deployments available while sessions retain those bindings;
a code-incompatible redeployment suspends existing Framework sessions rather
than silently changing their behavior.

The example registers only its current build and has no retained-build router
or automatic Framework binding migration. A changed build cannot continue old
sessions; retaining a Cloudflare version alone does not route those sessions.
Test incompatible revisions in a separate Worker and namespaces. A long-lived
deployment needs retained-build routing or an explicit migration before upgrade.

This is experimental hosting for small demo rooms. There is no automatic
compaction, individual identity or production quota system. Shared access permits
use of the configured model budget. External tool effects require idempotency or
reconciliation. The chat client displays committed snapshots rather than token
streams. See the [walkthrough](../../docs/framework/examples/durable-chat.md).
