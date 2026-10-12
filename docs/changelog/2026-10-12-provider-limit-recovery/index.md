---
title: "Sessions recover from short provider limits and follow the agent's model"
description: "A session retries on its own after a short provider rate limit, explains OpenRouter credit shortfalls, and follows the agent's default model unless one was chosen for it."
date: 2026-10-12
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, api]
links:
  docs: [/how-to/handle-errors-and-cancellation/]
---

**Short provider limits:** when a provider refuses a turn and asks for a wait of
up to 10 minutes, such as OpenRouter's in-flight spending limit
(`Retry-After: 120`), the session now schedules one retry after that wait and
says so in chat. It retries at most 3 times an hour, then waits for you as
before. A wait the provider asks for during a turn is also honored when it fits
the turn's own retry budget.

**Clearer errors:** the in-flight spending limit gets its own message instead of
a generic "Rate limited". An OpenRouter 402 "requires more credits ... can only
afford Y" now reports `provider_quota_exhausted` with `requested_max_tokens` and
`affordable_tokens`, and tells you to add credits or lower max output tokens,
instead of a generic processing error.

**Models:** a new session no longer copies the agent's default model. Changing
the agent's model now reaches its existing sessions on their next turn. A model
chosen for the session still wins, and `PATCH /v1/sessions/{id}` (or
`everruns sessions update --model-id`) sets it; `"model_id": null` clears it.
