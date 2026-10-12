---
title: "Everruns agents act as Poppy personal agents"
description: "The poppy_client capability lets an Everruns agent find a company's agent through the Personal Agent Protocol, talk to it and hand off to a person there."
date: 2026-10-10
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/poppy-personal-agent/]
  prs: [everruns/everruns#4479, everruns/everruns#4549, everruns/everruns#4558]
---

The [Personal Agent Protocol](https://personalagentprotocol.org) ("Poppy") lets
a person's own agent do business with a company: find it, start a conversation
with its agent, and hand off to a person. Add the `poppy_client` capability and
an Everruns agent becomes that personal agent for any company that publishes
`/.well-known/poppy.json`, with nothing to register. Platform Chat has it.

```json
{ "ref": "poppy_client", "config": {} }
```

The deployment holds one personal agent identity with pairwise user IDs, and
keys and session tokens stay in the control plane. A Poppy channel type serves
the company side of the protocol.

See [Poppy personal agent](/capabilities/poppy-personal-agent/) and [#4479](https://github.com/everruns/everruns/pull/4479), [#4549](https://github.com/everruns/everruns/pull/4549), [#4558](https://github.com/everruns/everruns/pull/4558).
