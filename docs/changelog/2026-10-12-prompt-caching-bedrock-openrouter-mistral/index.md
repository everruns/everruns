---
title: "Prompt caching on Bedrock, OpenRouter Claude and Mistral"
description: "Prompt caching now writes cache points on Bedrock Claude and Nova, turns on caching for Claude through OpenRouter, and routes Mistral by cache key."
date: 2026-10-12
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [agent, code]
links:
  docs: [/capabilities/prompt-caching/]
---

The Prompt Caching capability now covers three more providers. Before, these
requests went out without caching and paid the full input rate on every call.

- **Bedrock**: Claude and Nova models get `cachePoint` blocks after the system
  prompt, the tools (Claude only) and the latest messages. Cache reads and
  writes now show up in usage and cost.
- **OpenRouter**: Claude models get Anthropic's automatic caching, with the
  `retention` setting honored.
- **Mistral**: requests carry a session-scoped `prompt_cache_key`, so calls that
  share a prefix reach the same cache.
