---
title: Durable multiplayer chat on Cloudflare
description: Build a shared chat room with the Everruns engine, SQLite-backed Durable Objects, forks, scheduled work, and a recoverable history-search subagent.
appliesTo: [framework]
---

This walkthrough recreates the behavior of Earendil's
[Pi Durable on Cloudflare tutorial](https://earendil.com/posts/pi-durable-on-cloudflare/)
with the Everruns execution kernel. A room contains several conversations,
every participant sees the same committed messages, and work continues after a
browser disconnects or the Worker restarts.

The implementation is in
[`examples/cloudflare-durable-chat`](https://github.com/everruns/everruns/tree/main/examples/cloudflare-durable-chat).
The Everruns WASM engine runs directly inside a Cloudflare Durable Object,
using its native SQLite, alarms and hibernatable WebSockets. The application
is an independently built package with no celld runtime or
sibling-example dependency. Wrangler runs the same Worker locally and deploys
it to Cloudflare.

**Experimental advanced host example.** This uses `everruns-core`'s portable
Reason/Act kernel directly. The `everruns` application facade and its native
`LocalConfig` storage are not compiled into the Worker. For native applications,
start with [Framework sessions](/framework/sessions/).

## Run the chat

Clone the whole Everruns repository: the example uses local workspace crates.
Install Rust, Node.js 22 or newer, and the build tool:

```sh
git clone https://github.com/everruns/everruns.git
cd everruns/examples/cloudflare-durable-chat
rustup target add wasm32-unknown-unknown
cargo install worker-build --version 0.8.7 --locked
npm ci
worker-build --release
cp .dev.vars.example .dev.vars
```

Put your OpenRouter API key in `.dev.vars`, then start Wrangler:

```sh
npm run dev
```

Open the printed local URL. Choose a display name and create a conversation
with the **+** button. **Copy room link** gives you a URL to open in another
browser. Both participants can send messages, and the assistant answers queued
messages in order within each conversation. Other conversations can run at the
same time.

The default is [Muse Spark 1.3 Contributor](https://openrouter.ai/meta/muse-spark-1.3-contributor)
through OpenRouter (`meta/muse-spark-1.3-contributor`). Set `MODEL` or
`OPENROUTER_BASE_URL` in `.dev.vars` to use another Chat Completions model.
Requests cap output at 4,096 tokens unless the engine supplies an explicit limit.
Provider charges apply; the Contributor model uses submitted data for training.

## One room, one Durable Object

The Worker routes `/rooms/<name>/...` to the `ROOMS` binding using
`id_from_name`. The browser creates an opaque room ID and puts it in the shared
URL. Different names address separate SQLite databases.

| Original tutorial | Everruns example |
| --- | --- |
| Pi harness per shared session | `ChatRoom` Durable Object, with a `Room` host |
| Pi conversation | One `ConversationEngine` with its own Everruns session ID, events, turn and inbox |
| Model and tool tasks | Everruns `ReasonAtom` and `ActAtom` steps |
| Task-owned search conversation | Child `ConversationEngine` owned by the parent's stable tool execution key |
| Pi task deadlines | Persisted schedules, projected onto the Durable Object alarm |
| PartySocket and React | Native hibernatable WebSockets and a dependency-free browser client |

The main files are:

- `src/engine.rs`: the shared engine step machine and canonical event projection.
- `src/chat.rs`: conversations, receipts, forks, schedules and tools.
- `src/worker.rs`: SQLite, WebSocket handlers, routing and alarms.
- `src/chat.html`: the browser interface and reconnect queue.

The small host keeps room state in a JSON document in one SQLite row. A
synchronous read/modify/write never crosses an `await`, and a single SQLite
statement is its atomic commit. That makes the durability boundaries easy to
follow, at the cost of rewriting the room document on each mutation. It is a
learning implementation for small rooms, not a scalable storage adapter.

## Commit messages and retry commands

Every create, fork, send or schedule-cancel command has a `request_id`.
`Room::command` saves the command's receipt in the same write as its mutation.
A retry with the same ID and input gets that receipt back; reusing the ID for
other input is rejected.

The browser saves unacknowledged commands in `sessionStorage`. A reconnect or
reload resends them with their original IDs. It replaces its transcript from a
fresh committed snapshot, so missing a WebSocket update loses no history.
Different tabs have separate pending queues.

Messages first enter a conversation's durable inbox. When the conversation is
ready, `ConversationEngine::step` opens the next turn, records its input, and consumes the
inbox entry together. Messages posted during a turn wait behind it.

## Fork from an earlier message

Select **Fork here** beside a committed user message or final assistant reply.
The fork copies the committed event prefix through that message, assigns a new
session ID, and starts with no active turn, queued messages or schedules. Later
changes to either conversation do not change the other.

This sample copies history rather than sharing it structurally. It refuses a
fork at an assistant tool-call message, which would otherwise leave an
unfinished tool exchange in the new context.

## Survive a lost Worker

A turn alternates between Reason and Act:

1. Reason calls the model and records its completed response.
2. If it requested tools, Act executes the tool batch and records results.
3. The next Reason reads those results, until a response ends the turn.

A step stages canonical events as uncommitted. It commits those events and the
next turn position together. If the Worker stops before the commit, recovery
discards only that step's uncommitted events and repeats the step. A committed
model call is not repeated when a later tool step is lost.

Before starting a step, the host persists a recovery alarm. The alarm can
restart work even with no connected browsers. `running` prevents overlapping
alarm handlers within the same isolate; persisted turns and inboxes determine
what a fresh isolate resumes.

WebSockets use `accept_web_socket`, allowing the object to hibernate between
events. Display names and selected conversations live in socket attachments.
They survive hibernation and are used to rebuild presence. The host sends
committed snapshots and change notifications; token deltas are not streamed.

## Wake for scheduled work

Ask: **“In 30 seconds, remind us to review the launch.”** Close both browsers,
wait, and reopen the room. The follow-up appears without a client submitting
another message. A recurring request can say **“Remind us every minute.”**
Use **Cancel** in the sidebar to stop future occurrences.

The `schedule` tool stores an instruction, deadline and optional interval. Its
key includes the session and the assistant tool-call message, so replaying an
interrupted Act step reuses the existing schedule. A later model request may
reuse a provider tool-call ID without colliding with the old schedule.

A Durable Object has one alarm. The host chooses the earliest schedule deadline
or the recovery deadline for active work. When an occurrence is due, it queues
the instruction and advances or deactivates the schedule in one commit.
Duplicate alarm deliveries therefore cannot queue that occurrence twice.
Recurring ticks missed during downtime collapse into one follow-up.

## Search through a durable child conversation

Try this sequence:

1. Tell the assistant: **“We chose PostgreSQL for the event store.”**
2. Create a second conversation.
3. Ask: **“What did we decide about the event store?”**
4. Select **History search** in the sidebar to inspect the child's search and answer.

The parent has `search_history`; the child has only `search_conversations`.
The parent tool creates the child once under its stable execution key. If that
tool is replayed, it finds the child, resumes its unfinished engine step, and
returns its final answer. Raw matches stay in the child's transcript.

Search uses case-insensitive substring matching across this room's top-level
user and assistant messages. It excludes search children and assistant replies
produced by previous history searches, so generated search answers do not
become evidence for later searches. The child is asked to cite conversation IDs
and message sequence numbers and to treat transcripts as untrusted evidence.

## Verify recovery

From the example directory:

```sh
cargo test
npm run build
npm test
# This includes a hosting-boundary audit of dependencies and built exports.
```

The native tests drop a step's future mid-flight and reopen persisted state.
The end-to-end test runs actual workerd, SQLite, WebSockets and alarms. It kills
the runtime during a child model request and before a scheduled deadline,
restarts it, and waits for the test model to receive requests before any client
reconnects. Scripted model responses are used only in tests.

## Deploy and limits

```sh
npx wrangler login
npm run deploy
npx wrangler secret put CHAT_ACCESS_TOKEN
npx wrangler secret put OPENROUTER_API_KEY
```

The optional `CHAT_ACCESS_TOKEN` secret protects every route, including WebSocket upgrades. Open `/?access=YOUR_KEY`: the page sets an HttpOnly cookie
for API and WebSocket requests. The copied invitation URL includes the key,
so a second browser can join. API clients may instead send the key as a Bearer
credential. Keep invitation URLs private.

Without that secret the demo is open. For a production service, replace the
shared key with individual authentication, room authorization and rate limits.
Anyone with a room invitation can read and steer that room using your provider
key. Display names are labels, not verified identities.

The browser client is intentionally small. There is no automatic compaction,
full-text index, retention policy or production quota system. Commands have
small admission limits to keep the demo manageable. A fork copies its prefix;
the room document is read and rewritten in full. External tools may run again
if their Act step was interrupted, so any added side effects need their own
idempotency or reconciliation. Weather lookup is not included; use a reminder
for the scheduling exercise instead of expecting current weather data.
