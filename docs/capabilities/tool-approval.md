---
title: Tool Approval
description: Hold back risky tool calls until a person approves them. The call does not run until someone says yes.
sidebar:
  order: 95
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `tool_approval` |
| **Category** | Safety |
| **Tools** | None |
| **Dependencies** | None |

A hard approval gate. Before a risky tool runs, the turn pauses and a person
decides. Unlike [soft approval](/capabilities/ask-user/#not-a-consent-gate),
which asks the model to pause, this gate does not depend on the model: a call
nobody approved does not run.

## What gets gated

Each tool declares what it does, and the gate reads those declarations rather
than guessing from names:

| Mode | Asks before |
|---|---|
| `normal` (default) | Tools that declare themselves destructive or outward-facing (they reach outside the session), and tools that declare they require approval |
| `protective` | Anything not declared read-only. A tool with no declarations counts as one that changes state |
| `auto` | The same calls as `protective`, but a reviewer model answers first and asks a person only when it will not decide. See [Auto mode](#auto-mode) |
| `off` | Nothing |

A tool cannot avoid the gate by declaring itself read-only and outward-facing
at the same time.

## Configuration

```json
{
  "ref": "tool_approval",
  "config": {
    "mode": "auto",
    "timeout_seconds": 900,
    "rules": [
      { "tool": "github_merge_*", "action": "ask" },
      { "tool": "send_email", "action": "never" }
    ],
    "auto": { "notes": ["github.com/acme is our organization; pushing branches there is routine."] }
  }
}
```

| Field | Default | Description |
|---|---|---|
| `mode` | `normal` | `off`, `auto`, `normal`, or `protective` |
| `timeout_seconds` | `900` | How long a request waits for an answer before it counts as rejected (60 to 86400) |
| `rules` | none | Tool name patterns (`*` matches anything) with `allow` (run without asking), `ask` (always a person) or `never` (blocked). The strictest match wins, in every mode. At most 50 |
| `auto.notes` | none | Plain sentences the auto reviewer reads: what you trust, what counts as production. At most 20 |

## Auto mode

Long tasks stall when every routine call waits for a person. In `auto` mode a
reviewer model looks at each call the gate would hold and answers:

- **Allow**: the call runs.
- **Block**: the call does not run and the agent is told which rule it broke.
  The agent can ask you in chat; once you say yes, its retry is allowed.
- **Ask a person**: the call waits for you as in any other mode.

The reviewer sees what people wrote in the session, the agent's tool calls
since the last message, the call itself and your trust notes. It never sees
the agent's own text, tool output or tool descriptions, so instructions hidden
in a web page or a file cannot talk it into anything. It blocks by default
anything that sends data out, deletes what already existed, contacts people,
spends money, changes access or production, acts in bulk, or goes beyond what
a person asked for.

When the reviewer is unavailable or unsure, a person decides: it never lets a
call through by default. After 3 blocks in a row, or 20 in a session, it stops
reviewing and every call goes to a person until someone approves one. Tools
that always require approval, and tools with an `ask` rule, always go to a
person.

Each verdict is recorded as a `tool.reviewed` session event and in the
organization audit log as `agent.approval.auto_reviewed`.

## Who picks the mode

The agent's config sets the mode. A person can change it for one session, and
set their own default for all their sessions:

```bash
everruns sessions approval-mode set --session-id $SESSION_ID --mode auto
curl -X PUT "$EVERRUNS_API/v1/user/preferences/tool_approval" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": {"default_mode": "auto", "rules": [{"tool": "send_*", "action": "ask"}]}}'
```

Personal rules can only add caution (`ask` or `never`). A person cannot turn
the gate off for an agent whose config turned it on: `off` runs as `auto`.

Organization admins can limit the modes and add rules and trust notes for
every agent with `tool_approval_policy` on `PATCH /v1/orgs/{org}`:

```json
{
  "tool_approval_policy": {
    "allowed_modes": ["auto", "normal", "protective"],
    "rules": [{ "tool": "*_delete_*", "action": "ask" }],
    "trust_notes": ["Only the prod-* clusters are production."]
  }
}
```

A mode the organization does not allow runs as the next more careful one.

## Answering a request

When a call is held back, the session waits with status
`waiting_for_tool_results` and emits a `tool.call_requested` event containing
one `approve_tool_call` call per held-back call. Its arguments show the tool,
the arguments it would run with, why it was gated, and the deadline. The
session UI renders these as approval cards.

Answer them with:

```bash
curl -X POST "$EVERRUNS_API/v1/sessions/$SESSION_ID/tool-approvals" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"decisions": [{"tool_call_id": "tool_approval_toolu_01...", "decision": "allow"}]}'
```

| Decision | Effect |
|---|---|
| `allow` | This exact call may run, once |
| `allow_always` | Every call of this tool in this session may run |
| `reject` | This call does not run |
| `reject_always` | No call of this tool runs in this session |

All requests from one pause are answered together. A request left out of the
submission is not approved. Answering requires permission to manage the
session.

After an answer the turn resumes and the agent calls the tool again. An
`allow` covers only the call with exactly the arguments that were shown;
different arguments are asked about again.

If nobody answers before the deadline, the requests count as rejected and the
turn resumes. Nothing is remembered, so the agent can ask again later.

## When to enable it

Enable it for agents that can send messages, spend money, change production
systems, or browse with [computer use](/capabilities/computer-use/): anything
where one wrong call is expensive.

An agent that runs unattended on a schedule or trigger will wait for the full
timeout on every gated call and then carry on without it. `auto` cuts most of
those waits; calls the reviewer hands to a person still wait. Use `off`, or
leave the capability off, for agents nobody is watching.
