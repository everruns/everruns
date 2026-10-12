---
title: "Prompt caching gets a retention setting and a prompt-only mode"
description: "Ask for one hour Anthropic cache entries with retention 1h, and use the explicit strategy on Claude to cache tools and instructions but not the conversation."
date: 2026-10-12
kind: improvement
significance: minor
products: [platform, cloud, framework]
surfaces: [agent, code]
links:
  docs: [/capabilities/prompt-caching/]
  prs: [everruns/everruns#4593]
---

The Prompt Caching capability takes a `retention` setting: `default`, `5m`,
`30m` or `1h`. On Claude models, `1h` keeps cache entries for an hour, which
suits agents that pause between turns longer than the five minute default.
One hour writes cost twice the input rate, and cost estimates now include that.

```json
{ "ref": "prompt_caching", "config": { "retention": "1h" } }
```

The `explicit` strategy now works on Claude too: it caches the tool definitions
and system prompt but not the conversation, for agents whose history changes
from call to call.
