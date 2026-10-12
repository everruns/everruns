---
title: Dry Run
description: Try an agent without its side effects. Read-only tools run for real; every other tool call returns a simulated result and changes nothing.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `dry_run` |
| **Category** | Safety |
| **Tools** | None |
| **Features** | None |
| **Dependencies** | None |

Dry run lets you see what an agent would do before it does it. A session with
this capability runs tools that only read, and answers every other tool call
with a simulated result. Nothing is sent, written, or deleted.

The **Test run** button on an agent's page starts a session with dry run on.

## What runs for real

A tool runs for real when it declares itself read-only and nothing marks it
destructive or requiring approval. MCP tools declare this through their own
annotations or through a
[risk label](/reference/cli/mcp-servers/) set on the
connection. Every tool that does not declare itself read-only is simulated: a
dry run errs on the side of not acting.

## What a simulated call returns

The call is recorded as a successful result with a `dry_run` object that names
the tool and its arguments:

```json
{
  "dry_run": {
    "simulated": true,
    "tool": "visti_send",
    "arguments": { "text": "main CI is red" },
    "note": "Dry run: `visti_send` was not executed. Nothing was changed."
  }
}
```

The agent is told it is in a dry run. It carries on as if simulated calls
worked, so the rest of the run is exercised, and its final answer lists the
actions it would have taken. The session trace marks simulated calls.

## Configuration

Dry run has no settings. Add it to a session's capabilities:

```json
{ "capabilities": [{ "ref": "dry_run" }] }
```
