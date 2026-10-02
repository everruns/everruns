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
| `off` | Nothing |

A tool cannot avoid the gate by declaring itself read-only and outward-facing
at the same time.

## Configuration

```json
{
  "ref": "tool_approval",
  "config": { "mode": "normal", "timeout_seconds": 900 }
}
```

| Field | Default | Description |
|---|---|---|
| `mode` | `normal` | `off`, `normal`, or `protective` |
| `timeout_seconds` | `900` | How long a request waits for an answer before it counts as rejected (60 to 86400) |

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
timeout on every gated call and then carry on without it. Use `off`, or leave
the capability off, for agents nobody is watching.
