---
title: "OpenAI Responses caching defaults to twelve hours"
description: "Keep reusable OpenAI Responses prefixes warm across longer pauses with a twelve-hour default lifetime and estimates that include the higher write rate."
date: 2026-11-03
kind: improvement
significance: minor
products: [platform, cloud, framework]
surfaces: [agent, code]
links:
  docs: [/capabilities/prompt-caching/]
---

Direct OpenAI Responses calls on supporting models now default to twelve-hour
prompt caching, even without a caching capability. This is limited to
`api.openai.com` and models whose profiles support cache options (GPT-5.6 and
later). Other provider defaults are unchanged.

Use [Prompt Caching](/capabilities/prompt-caching/) to keep the lower write rate:

```json
{
  "ref": "prompt_caching",
  "config": {
    "retention": "30m"
  }
}
```

Twelve-hour writes cost 2.5 times uncached input. Estimates use the provider's
reported lifetime split and served service tier.
This does not enable twelve-hour caching on Chat Completions or native batches.

Set `enabled: false` to remove Everruns-managed cache controls and routing hints;
it cannot disable the provider's implicit caching. OpenAI documents prompt
caching as Zero Data Retention compatible, with encrypted KV tensors retained
for up to 24 hours. Twelve hours is a minimum reuse lifetime, not a deletion
deadline; response storage and organization data controls are unchanged.

In Framework code, explicitly keep thirty minutes with `everruns-contracts`:

```rust
use everruns_contracts::driver_registry::{PromptCacheConfig, PromptCacheRetention};
let cache = PromptCacheConfig {
    enabled: true,
    retention: PromptCacheRetention::ThirtyMinutes,
    ..Default::default()
};
```
