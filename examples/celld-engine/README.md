# The engine inside a celld cell

The everruns engine compiled to `wasm32-unknown-unknown` and run inside a
[celld](https://github.com/denoland/celld) Durable Object. Each cell
(`/cells/<name>/...`) is one agent session: its event log is the cell's
SQLite, and its turns run in the cell, one engine step per commit.

Compare [`examples/serve/celld`](../serve/celld), which runs a whole `serve`
app in a container next to the cell and replays whole requests after a loss.
Here a lost node costs at most the step that was running: one model call or
one tool batch. Steps that already committed never run again.

## Run it

```sh
rustup target add wasm32-unknown-unknown
cargo install worker-build
worker-build --release

echo "OPENAI_API_KEY=sk-..." > .dev.vars
celld dev
```

```sh
curl -X POST localhost:8080/cells/demo/messages \
  -H content-type:application/json -d '{"text": "What is celld?"}'
curl localhost:8080/cells/demo/state     # turn position, inbox, step counters
curl localhost:8080/cells/demo/events    # the canonical event log
```

Any Chat Completions endpoint works: set `OPENAI_BASE_URL` and `MODEL` in
`.dev.vars` or `wrangler.jsonc`.

## How a turn runs

- `POST /messages` queues the message in the cell's inbox and arms the
  alarm. The response is `202`; the turn runs in the alarm, not the request.
- The alarm opens a turn for the oldest queued message, then runs engine
  steps: Reason (`ReasonAtom`, one model call), then Act (`ActAtom`, the tool
  calls it asked for), until the model answers without tools.
- Each step writes its events uncommitted and commits them, together with
  the next turn position, in one `transactionSync`.
- Before each step the alarm is re-armed one lease (`LEASE_MS`) ahead. If the
  node dies, that alarm fires on the node that owns the cell next. The cell
  discards the uncommitted events and re-runs only the interrupted step.

The step machine (`src/cell.rs`) and the driver (`src/openai.rs`) are
portable; only `src/durable.rs` needs the isolate.

## Tests

```sh
cargo test                       # the step machine, incl. losing a step mid-flight
worker-build --release
node test/e2e.mjs                # a real celld node, killed in the middle of a tool call
```

The e2e test uses a scripted model server in the test process, so it needs
no key. It kills the celld node while the second turn's tool call runs,
restarts it, and sends nothing. The alarm finishes the turn. The test then
checks that the model was called four times for two turns, which means the
committed call was not repeated.

## Limits

- A tool with side effects outside the cell can run twice if the node dies
  after it ran and before its step committed.
- Model calls are non-streaming Chat Completions through `fetch`.
- The example's one tool is pure; tools that need a filesystem or processes
  have no isolate equivalent.
