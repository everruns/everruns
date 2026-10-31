---
title: "Sessions with tool calls keep the prompt cache again"
description: "Workers gave each loaded tool result its load time, so every model call rewrote earlier time facts and lost the prompt cache after the first tool call."
date: 2026-10-30
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/prompt-caching/]
---

**For agents:** a worker that loads session history from the server gave each
tool result the time it was loaded, not the time it was stored. The
`current_time` fact placed after each tool result was then rewritten on every
model call, so the provider's prompt cache stopped at the first tool call of
the session. Tool results now keep their stored time, so each call only
appends to the previous one.

On models that receive the current tool list, a tool revealed by tool search
now goes at the end of the list instead of where it sorts, and OpenAI-style
`prompt_cache_key` values no longer change when tools or instructions change
(see [Prompt Caching](/capabilities/prompt-caching/)).

Old tool outputs are now replaced by summaries eight at a time, not one per
call, so a busy session rewrites its history once per batch.
