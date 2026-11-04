---
title: Turns no longer fail when a provider asks to wait too long
description: A provider rate limit whose Retry-After exceeds the retry budget now falls back to backoff and retries, instead of failing the turn at once.
date: 2026-10-18
kind: fix
significance: notable
products: [platform, cloud, framework]
release: "0.49.0"
surfaces: [ui, code]
---

When a model provider answered a rate limit or outage with a `Retry-After`
longer than the 30-second retry budget, for example OpenRouter asking for 45
seconds, the turn failed within milliseconds without a single retry. The
request now falls back to exponential backoff and retries inside the budget.

When the budget does run out, the error reports the time actually spent next
to the budget, rather than only the configured budget:

```text
Before: retry time budget exhausted after 0 retries over 30.0s
After:  retry time budget exhausted after 1 retries in 12.4s of a 30.0s budget
```

This applies to every HTTP driver: OpenAI Responses, OpenAI Chat Completions,
Anthropic and Gemini.
