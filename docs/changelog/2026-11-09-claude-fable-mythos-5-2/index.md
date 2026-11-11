---
title: "Claude Fable 5.2 and Mythos 5.2 models added for Anthropic providers"
description: "claude-fable-5-2 is seeded as a favorite and becomes the Anthropic research model at 20% below Fable 5.1; claude-mythos-5-2 works for verified orgs."
date: 2026-11-09
kind: feature
significance: minor
products: [platform, cloud, framework]
release: "0.52.0"
surfaces: [api, ui]
links:
  docs: [/providers/anthropic/]
---

Anthropic providers now offer `claude-fable-5-2`, released the same day, and
its `[1m]` long-context twin. Both are seeded as enabled favorites, and Fable
5.2 replaces Fable 5.1 as the recommended research model on Anthropic and
Bedrock. Fable 5.1 stays enabled for agents that use it.

It costs $8/$40 per million input/output tokens, with $0.20 cache reads, so the
same run on the top Claude tier reports 20% less than on Fable 5.1. Thinking is
always on, with high effort by default. Pick it in an agent's model picker, or
call it straight from the Framework:

```rust
let model = Model::new("claude-fable-5-2", Anthropic::from_env()?);
let answer = model.complete("Is 2^61 - 1 prime?").await?;
```

`claude-mythos-5-2`, the same model for organizations in Anthropic's
verification programs, is not seeded but gets the same profile once added.

See [Anthropic](/providers/anthropic/).
