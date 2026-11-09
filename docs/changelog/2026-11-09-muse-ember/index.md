---
title: Muse Ember, Meta's cheapest Muse model, is available in Everruns
description: Muse Ember runs sub-agents, tool calls and routing at $0.10/$0.50 per million tokens, and becomes the Meta small model; Muse Spark 1.1, Spark Open and Glimmer 30B join the catalog.
date: 2026-11-09
kind: feature
significance: minor
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/providers/meta/]
---

Meta released **Muse Ember** (`muse-ember-1.0`) today, and it works in Everruns right away. Ember is
the Muse model for the work an agent repeats all day: sub-agent tasks, tool calls, routing, and
running one job across thousands of files or records. It costs $0.10 input, $0.004 cached and $0.50
output per million tokens, about a twelfth of Muse Spark 1.3, and still reads the full 1,048,576-token
context plus images, audio, video and PDFs. It is now the Meta provider's recommended small model.

Ember thinks hard by default (Meta runs it at Max effort unless told otherwise), so drop the effort
for quick routing calls:

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

Requests over 200K input tokens bill entirely at $0.30 / $0.012 / $1.50, and costs in Everruns
follow that switch.

The rest of Meta's lineup is in the catalog too: the original `muse-spark-1.1` on Meta Model API,
and the open-weight Muse Spark Open and Muse Glimmer 30B through OpenRouter. OpenRouter's
version-less `meta/muse-ember` and `meta/muse-wonder` IDs now pick up the right profile.

See [Meta](/providers/meta/).
