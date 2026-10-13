---
title: "Answers cut off by a provider error are retried"
description: "When a model stream breaks after some answer text, the runtime retries it instead of keeping the cut-off text as the answer."
date: 2026-10-13
kind: improvement
significance: minor
products: [platform, cloud, framework]
surfaces: [api]
links:
  docs: [/explanation/agentic-loop/]
  prs: [everruns/everruns#4702]
---

When a provider stream breaks after the model has written part of its answer, and before it has
called a tool, the runtime now retries the call with the usual backoff instead of keeping the
cut-off text as the answer. The retry streams under the same message id and first repeats
`output.message.started` for it, so a client knows to drop the text it collected:

```text
output.message.started  message_01
output.message.delta    message_01  accumulated: "Here are the"
output.message.started  message_01
output.message.delta    message_01  accumulated: "Here are the links."
```

A client that renders `accumulated` needs no change. See [The agentic loop](/explanation/agentic-loop/).
