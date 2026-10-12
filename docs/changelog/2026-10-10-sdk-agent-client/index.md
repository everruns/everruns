---
title: "Everruns SDKs add AgentClient to call one agent with a key"
description: "The Python, TypeScript and Rust SDKs gain AgentClient, which calls a single agent through the Agent Execution API with an agent key, including SSE streaming."
date: 2026-10-10
kind: feature
significance: major
products: [sdk]
surfaces: [code]
release: "0.3.0"
links:
  docs: ["/how-to/call-your-agent-from-code/", "/features/sdk/"]
  prs: [everruns/sdk#116]
---

`AgentClient` in the Python, TypeScript and Rust SDKs calls one agent through the
Agent Execution API with an agent key (`evr_ak_...`). It reads
`EVERRUNS_AGENT_URL` and `EVERRUNS_AGENT_KEY` and covers `run()`, sessions,
messages, events with SSE streaming, cancel, questions, tool approvals, runtime
tokens, `for_end_user` and idempotency keys.

```python
from everruns_sdk import AgentClient

agent = AgentClient()
print(await agent.run("What is the status of order 42?"))
```

Calling an agent through the management client's `sessions.create` and
`messages.create` still works but is deprecated: Python warns once, and
TypeScript marks the methods `@deprecated`.

See [Call your agent from code](/how-to/call-your-agent-from-code/), [SDKs](/features/sdk/) and [#116](https://github.com/everruns/sdk/pull/116).
