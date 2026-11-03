---
title: "OpenAI prompt caching supports twelve-hour retention"
description: "Keep reusable OpenAI Responses prefixes warm across longer pauses with an opt-in twelve-hour lifetime and estimates that include the higher write rate."
date: 2026-11-03
kind: improvement
significance: minor
products: [platform, cloud, framework]
surfaces: [agent, code]
links:
  docs: [/capabilities/prompt-caching/]
---

[Prompt Caching](/capabilities/prompt-caching/) accepts `retention: "12h"` for
direct OpenAI Responses calls on supporting models. Defaults stay unchanged.
Choose `explicit` to cache just the stable instructions prefix:

```json
{
  "ref": "prompt_caching",
  "config": {
    "retention": "12h",
    "strategy": "explicit"
  }
}
```

Twelve-hour writes cost 2.5 times uncached input. Estimates use the provider's
reported lifetime split and served service tier. Anthropic snaps this request
to one hour; Azure and custom Responses endpoints retain thirty minutes.
This does not enable twelve-hour caching on Chat Completions or native batches.

In Framework code, with `everruns-contracts`:

```rust
use everruns_contracts::driver_registry::{PromptCacheConfig, PromptCacheRetention};
let cache = PromptCacheConfig {
    enabled: true,
    retention: PromptCacheRetention::TwelveHours,
    ..Default::default()
};
```
