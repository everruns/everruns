---
title: Framework sessions can stop one tool call
description: Session::cancel_tool_call stops a single running tool call; the model reads that it was cancelled and the turn carries on.
date: 2026-11-12
kind: feature
significance: minor
products: [framework]
surfaces: [code]
links:
  docs: [/framework/events-and-cancellation/#cancel-one-tool-call]
---

An embedded agent's host can now
[stop one running tool call](/framework/events-and-cancellation/#cancel-one-tool-call)
without cancelling the whole turn, for example when a user stops a long shell command
in an editor:

```rust
if session.tool_call_running("call_42") {
    session.cancel_tool_call("call_42");
}
```

The call's future is dropped, its tool context cancellation fires, and it
completes with status `cancelled` and an error the model reads, so the agent
can react and continue. Other calls in the same batch finish as usual. A host
that runs tools behind its own registry can put a `ToolCallCancels` in the
tool context extensions instead.
