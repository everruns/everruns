---
title: "Sessions recover from provider limits and follow the agent's model"
description: "A session retries after a short provider rate limit, explains OpenRouter credit shortfalls, and follows the agent's default model unless one was set."
date: 2026-10-12
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, api]
release: "0.48.0"
links:
  docs: [/how-to/handle-errors-and-cancellation/]
---

When a provider refuses a turn and asks for a wait of up to 10 minutes, such as
OpenRouter's in-flight spending limit, the session schedules one retry after
that wait and says so in chat, at most 3 times an hour. An OpenRouter 402 "can
only afford Y tokens" now reports `provider_quota_exhausted` with the token
counts and tells you to add credits or lower max output tokens. See
[handling errors](/how-to/handle-errors-and-cancellation/).

A new session no longer copies the agent's default model, so changing the
agent's model reaches its existing sessions. To pin one session:

```bash
everruns sessions update session_01h9 --model-id model_01933b5a00007000800000000000001 --reason "Provider limit on the current model"
```
