# Owned WebSocket steering prototype

An **opt-in standalone experiment**, not a production worker transport. It runs
GPT-6 Astra on one owned connection while other local processes submit durable
user updates. Existing Everruns HTTP providers and default models are unchanged.
The [architecture contract](../../knowledge/execution/openai-steering-prototype.md)
explains ownership, recovery tradeoffs, and the bar for distributed integration.

## Start with the Everruns Framework

For an application using real `Agent`, `Engine`, and `Session` APIs, run the
[Framework live-session example](../../crates/everruns/examples/live_session.rs):

```sh
# From the repository root. Offline and free; echoes the correction.
cargo run -p everruns --example live_session

# Real OpenAI model; requires OPENAI_API_KEY and API credits.
cargo run -p everruns --features openai --example live_session -- --live \
  'Plan a three-day trip from Paris to Amsterdam.' \
  'Prefer trains and keep the budget under EUR 500.'
```

The Framework example uses `session.send()` for both the initial request and a
correction, waits on the returned receipts, and prints the answer and retained
user messages. Replace its second `send()` with your UI's message-submit handler.
If the turn is still active, the correction joins it; if it finished, a follow-up
turn starts. Acceptance is distinct from completion; wait on the latest receipt
for the result that includes the correction.

**These are two different mechanisms.** Framework steering currently queues user
input for the next reasoning/tool boundary over ordinary HTTP. This Python
experiment sends `response.steer` on an active OpenAI WebSocket. It is a prototype
because the connection owner, recovery journal, and status projection are not
integrated with Framework sessions or distributed workers. The Framework example
does not claim that in-flight WebSocket capability or the Python journal's
recovery guarantees; its session lives in memory.

## Python WebSocket experiment

Requires Python 3.11+ on macOS/Linux. SQLite and the ownership lock require a local
filesystem; do not put the journal on NFS or share it between hosts.

## Run without API credentials

From this directory:

```sh
python3 -m venv /tmp/everruns-steering-venv
/tmp/everruns-steering-venv/bin/pip install -r requirements.txt
/tmp/everruns-steering-venv/bin/python -m unittest -v
```

The integration test starts a real loopback WebSocket server, launches the CLI in
a separate process to submit a user update, acknowledges it, requests a tool
result, and verifies the successor output and combined usage. Other tests cover
normal and interrupted parents, FIFO updates, rejection, approvals, duplicate
receipt, ownership exclusion, and recovery with verified history. No billable API
requests are made by the test suite. The path-scoped OpenAI Steering Prototype CI
workflow runs this suite on Python 3.11 and 3.14 for PRs and main.

## Run against OpenAI

Set `OPENAI_API_KEY` in the environment. The account needs GPT-6 Astra access and
credits. Use a fresh journal path; `init` refuses to overwrite an existing file.

```sh
PY=/tmp/everruns-steering-venv/bin/python
"$PY" steering.py --db /tmp/steering-session.db init 'Draft a detailed project plan for a task-tracking app.'
"$PY" steering.py --db /tmp/steering-session.db run
```

From another terminal, send an actual user correction while it runs:

```sh
PY=/tmp/everruns-steering-venv/bin/python
"$PY" steering.py --db /tmp/steering-session.db update 'Keep scope to two weeks for one developer.' --id user-message-1
"$PY" steering.py --db /tmp/steering-session.db status
```

Retry delivery with the same `--id` and identical text. Reusing an ID with different
text is rejected. IDs are local; the wire steering event contains only the three
fields allowed by OpenAI. Each journal is one isolated session, and updates sent
after completion become ordinary follow-up Responses on the same connection.

`run` stays available for further user updates until Ctrl+C or its connection
budget expires. It prints response/intent state changes. `status` includes final
response outputs, required inputs, errors, durable inbox contents, and aggregate
usage across all terminal responses. The journal and owner-lock file are created
with mode 0600; journals contain prompts, results, and provider events. Keep them
outside the repository.

For a finite, **billable** check that queues a correction for the first live
response and verifies the successor's marker:

```sh
"$PY" live_smoke.py --db /tmp/steering-live-check.db
```

This fails explicitly if steering does not reach a completed successor. It never
reports acceptance alone as a successful smoke test.

## Tools and approvals

Pass `--settings settings.json` to `init` to provide synchronous function/custom
tools, MCP tools, instructions, reasoning, or an output-token limit. For example:

```json
{
  "instructions": "Read the current project status with get_project_status before planning.",
  "tools": [{
    "type": "function",
    "name": "get_project_status",
    "description": "Read current project status",
    "parameters": {"type": "object", "properties": {}, "required": [], "additionalProperties": false},
    "strict": true
  }]
}
```

The owner does **not** execute tools or decide approvals. A tool worker reads the
call from `status`, performs it once, and persists its actual result using the
original call ID. For example, save this as `/tmp/steering-result.json`, replacing
`ACTUAL_CALL_ID` and the output with the real call's result:

```json
{"type":"function_call_output","call_id":"ACTUAL_CALL_ID","output":"Design complete; implementation has not started."}
```

```sh
"$PY" steering.py --db /tmp/steering-session.db result /tmp/steering-result.json
```

An approval result uses `type: "mcp_approval_response"`, the actual
`approval_request_id`, and an explicit boolean `approve` from the normal approval
flow. Missing decisions and unrelated IDs are rejected. Save results before
retrying delivery; conflicting results for the same call are rejected. Accepted
steering never cancels a tool already running, and the owner never reruns it.

The owner waits for all required results and sends one continuation with saved
settings. The accepted correction is not repeated: OpenAI prepends it. This
prototype accepts string function/custom tool outputs and MCP approval decisions;
other required input types remain visible and blocked. Native async tools and
configuration updates are outside this experiment.

## Recover

Stop the old owner before recovering. The exclusive lock enforces this, including
when an old process is paused rather than dead.

```sh
"$PY" steering.py --db /tmp/steering-session.db recover
"$PY" steering.py --db /tmp/steering-session.db resume
```

Recovery retrieves known nonterminal responses over HTTP. If the known response
is still running, retry recovery after it finishes. It cannot be steered from a
new connection. Completed responses contribute usage once, and saved tool results
can continue from their parent without reexecuting the tool.

A sent or accepted correction without an observed successor becomes `uncertain`.
It is retained and **never automatically replayed**. OpenAI has no documented
successor-discovery endpoint or steering idempotency key. If an operator obtains
the successor ID from provider history, supply it:

```sh
"$PY" steering.py --db /tmp/steering-session.db recover --successor-id resp_ACTUAL_SUCCESSOR
"$PY" steering.py --db /tmp/steering-session.db resume
```

The owner fetches that response and paginated input history, checks session
metadata and parent, and requires exactly the expected additional user input and
saved results before committing local state. A guessed ID, absent input, or
duplicate input fails reconciliation. If the successor cannot be identified, the
journal remains blocked for investigation; absence from the parent alone cannot
prove that replay is safe. This prototype does not silently restart the prompt or
claim exactly-once automatic recovery across that ambiguity.

## Validation recorded on 2026-09-05

The local suite exercises the protocol end to end through a real WebSocket and
the producer CLI. A live GPT-6 Astra request reached `response.created`,
`response.in_progress`, and `response.steer.accepted`, then returned
`credit_balance_exhausted`. The unresolved input was preserved without replay.
Live successor completion remains unverified; rerun the optional smoke check with
an account that has credits. No UI was changed.
