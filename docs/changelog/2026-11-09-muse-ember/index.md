---
title: Muse Ember, Meta's cheapest Muse model, is available in Everruns
description: Muse Ember runs sub-agents, tool calls and routing at $0.10/$0.50 per million tokens and becomes the Meta small model, alongside more Muse models.
date: 2026-11-09
kind: feature
significance: minor
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/providers/meta/]
---

Meta released **Muse Ember** (`muse-ember-1.0`) today, and it works in Everruns right away. Ember
handles the work an agent repeats all day: sub-agent tasks, tool calls and routing. It costs $0.10
input and $0.50 output per million tokens, about a twelfth of Muse Spark 1.3, and still reads 1M
tokens of context. It is now the Meta provider's small model. Requests over 200K input tokens bill at
3x, and Everruns costs follow.

Ember thinks at Max effort by default, so drop it for quick routing:

```rust
let model = Model::new("muse-ember-1.0", Meta::from_env()?);
let route = model
    .completion()
    .user("Route this ticket: 'I was charged twice'. Answer billing, bug or other.")
    .reasoning_effort(ReasoningEffort::Low)
    .send()
    .await?;
println!("{}", route.text); // billing
```

Also new: `muse-spark-1.1`, and the open-weight Muse Spark Open and Glimmer 30B through OpenRouter.

See [Meta](/providers/meta/).
