---
title: "Claude Fable 5.2 and Mythos 5.2 models added for Anthropic providers"
description: "claude-fable-5-2 is seeded as an enabled favorite and becomes the research model on Anthropic and Bedrock; claude-mythos-5-2 is recognized for verified organizations."
date: 2026-11-09
kind: feature
significance: minor
products: [platform, cloud, framework]
surfaces: [api, ui]
links:
  docs: [/providers/anthropic/]
---

Anthropic providers now offer `claude-fable-5-2`, the same day Anthropic
released it, and its `[1m]` long-context twin. Both are seeded as enabled
favorites, and Fable 5.2 replaces Fable 5.1 as the recommended research model
for Anthropic and Bedrock providers. Fable 5.1 stays enabled for agents that
already use it.

Fable 5.2 costs less than Fable 5.1: $8 per million input tokens and $40 per
million output tokens, with cache reads at $0.20, so the same agent run on the
top Claude tier now reports a 20% lower cost. Its profile has 128K output
tokens, always-on adaptive thinking with high effort by default, preserved
thinking, mid-conversation system messages, tool search and the computer
toolset.

`claude-mythos-5-2` is the same model for organizations in Anthropic's
verification programs. It is not seeded; an organization with access adds it to
its Anthropic provider, and it gets the same profile, pricing and request
handling as Fable 5.2.

See [Anthropic](/providers/anthropic/).
