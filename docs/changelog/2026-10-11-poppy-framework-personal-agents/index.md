---
title: "Framework apps act as Poppy personal agents"
description: "PoppyPersonalAgent makes a Framework app a person's own agent over the Personal Agent Protocol, with the same tools Platform Chat uses and a runnable example."
date: 2026-10-11
kind: feature
significance: notable
products: [framework]
surfaces: [code, agent]
links:
  docs: [/capabilities/poppy-personal-agent/, /features/poppy/]
  prs: [everruns/everruns#4577, everruns/everruns#4566]
---

A Framework app can now be a person's own agent over the
[Personal Agent Protocol](https://personalagentprotocol.org/docs/spec)
("Poppy"). Turn on the `poppy` feature of the `everruns` crate and give the
agent `PoppyPersonalAgent`. It gets the same five tools as Platform Chat:
discover, send, read, handoff and close.

```rust
use everruns::poppy::{PoppyKey, PoppyPersonalAgent};

let poppy = PoppyPersonalAgent::new("https://agent.example.com", PoppyKey::generate()?);
let agent = Agent::builder()
    .name("personal-agent")
    .model(model)
    .capability(poppy.clone())
    .build()?;
// Serve poppy.metadata("My assistant") at /v1/poppy/agent.json
// and poppy.jwks() at /v1/poppy/jwks.json.
```

The app signs with its own key and keeps the person's User IDs and Session
Tokens in memory. The server and the Framework now share one protocol client,
so both behave the same. The
[`poppy-personal-agent` example](https://github.com/everruns/everruns/tree/main/examples/poppy-personal-agent)
runs on `gpt-6.1-sol` and is tested against
[pi-poppy](https://github.com/richardanaya/pi-poppy)'s independent company.

See [Poppy Personal Agent](/capabilities/poppy-personal-agent/) and
[Personal Agent Protocol (Poppy)](/features/poppy/), and
[#4577](https://github.com/everruns/everruns/pull/4577),
[#4566](https://github.com/everruns/everruns/pull/4566).
