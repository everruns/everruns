---
title: "Long Muse sessions read their history from the prompt cache"
description: "Responses requests now always carry a prompt cache key, so Muse Spark caches prompts above 100k tokens instead of re-reading every call at full price."
date: 2026-11-01
kind: fix
significance: notable
products: [platform, cloud, framework]
surfaces: [agent]
links:
  docs: [/capabilities/prompt-caching/]
---

**For agents:** requests through the OpenAI Responses protocol (OpenAI,
OpenRouter, Meta, Vercel AI Gateway) sent a `prompt_cache_key` only when the
agent used the [Prompt Caching](/capabilities/prompt-caching/) capability.
Muse Spark caches a short prompt without that key but misses every call once
the prompt passes about 100k tokens, so a long Muse session paid full price
for its whole history on each model call. The key now goes out on every
request unless caching is turned off with `enabled: false`.

Agents with Infinity Context also moved the start of their history window on
every call once a session outgrew its budget, which changed the front of the
prompt each time. The window start now advances in steps of about a quarter of
the budget, so calls in between read the history from cache.
