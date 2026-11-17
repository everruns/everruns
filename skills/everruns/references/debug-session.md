# Debug an Everruns session

Commands below are the `everruns` command language (see the [cli.md](cli.md)).
Everything here is read-only, so run it with the CLI or MCP `query`.

## 1. Get the session id

From a session link (`<origin>/sessions/<session_id>/…`), or list recent
ones: `everruns sessions list --limit 20`. If the id belongs to another
organization the user is in, `everruns orgs resolve <session_id>` names it.

## 2. Read the state

```bash
everruns sessions get <session_id> | jq '{status, agent_id, harness_id, model_id, title}'
everruns sessions events summary events --session-id <session_id>
```

The summary gives counts by event type, the time span, turns and errors. Status
meanings: `active` is working, `idle` is waiting for a user message,
`waiting_for_tool_results` is blocked on a client-side tool or approval,
`paused` was stopped on purpose.

## 3. Read the events around the problem

```bash
everruns sessions events list --session-id <session_id> --limit 50 --order-desc
everruns sessions events list --session-id <session_id> --types turn.failed,tool.completed
everruns sessions events list --session-id <session_id> --tool-name web_fetch
```

`--types` takes a comma-separated list; the summary shows which types this
session has. Other filters: see `--help`
(`--turn-id`, `--q`, `--around`, `--since-id`). Look for, in order:

1. **Errors**: model provider errors, tool errors, `connection_required`
   (the user must connect an account: MCP `connect`), budget stops.
   A `turn.failed` with a generic `processing_error` hides its cause from the
   session; on the person's own install the worker log has it:
   `everruns docker logs worker-1 --no-follow` or
   `everruns k8s logs worker --no-follow`.
2. **A tool that kept failing or was called in a loop**: usually missing
   credentials, a wrong argument shape, or instructions that do not say when
   to stop.
3. **A tool the agent never had**: compare the tools it tried with what
   `everruns agents preview` resolves for its configuration.
4. **A question or approval nobody answered**: the session waits on the user.
   Over MCP, `everruns_home` shows what is pending.

## 4. Check whether the agent changed

```bash
everruns history list <agent_id> --limit 10
everruns history diff --help
```

A recent change with a `reason` often explains a new behavior. `history
restore` can roll back, but that is a change: confirm with the user first.

## 5. Fix it and keep it fixed

If the user wants the fix applied, change the agent with [build-agent.md](build-agent.md)
and a `--reason` that names the session. Then make the failure a test: add the
user's message as a case in the agent's eval, with a scorer for what went
wrong ([evaluate-agent.md](evaluate-agent.md)), and run it.

## 6. Report

Lead with the cause in one or two sentences, with the event or change that
shows it, then the fix (instructions, capability, credential, model) and
whether you applied it. Then say how to catch it next time: offer the eval case
from step 5, or an observer that scores live sessions for the same failure
([evaluate-agent.md](evaluate-agent.md)). Link the session with the `ui_link` a read returns.

For agents with recurring problems, `everruns health-issues list` and
`everruns agents health-checks list --agent-id <id>` show what Everruns has
already flagged.
