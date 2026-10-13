---
title: "Tool calls cut off by a crash are rerun when it is safe"
description: "After a worker crash, calls that never started, read-only and idempotent calls run again, and MCP servers get a stable idempotency key."
date: 2026-10-13
kind: improvement
significance: minor
products: [platform, cloud, framework]
surfaces: [api]
links:
  docs: [/explanation/durable-execution/]
  prs: [everruns/everruns#4694, everruns/everruns#4698, everruns/everruns#4700]
---

When a worker stops in the middle of a turn, the turn resumes on another worker, and fewer tool
calls now end as "interrupted":

- A call that had not started yet runs, even one that must run at most once, such as sending an
  email: such a call starts only after its start is recorded.
- A call to a tool marked read-only or idempotent runs again. This includes MCP tools that
  declare `readOnlyHint` or `idempotentHint`.
- Every MCP tool call carries an idempotency key that stays the same when the call runs again,
  so a server that honors it can return the first result instead of acting twice:

```json
{
  "method": "tools/call",
  "params": {
    "name": "send_email",
    "arguments": {},
    "_meta": {
      "io.everruns/idempotency-key": "9b6c0f3e-5d1a-5e4b-8f2a-0c7d4e6a1b23"
    }
  }
}
```

A short outage of the control plane also no longer stops a running turn, as long as it ends
within the lease time.

See [Durable Execution](/explanation/durable-execution/).
