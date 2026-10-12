---
title: "Session Trace names prompt cache resets and their cause"
description: "When a model call loses the prompt cache the previous call built, the trace now says so and names the likely cause, such as changed tools or an idle gap."
date: 2026-10-12
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/capabilities/prompt-caching/]
---

**In the UI:** the cache bar on a model call in Session Trace now warns when the
call reused less than half of what the previous call left cached, with how many
tokens were lost and the likely cause: the model, tool definitions or system
prompt changed, earlier messages were rewritten (for example by compaction), or
the session sat idle past the cache retention. A long prompt sent with caching
off gets a hint to turn on Prompt Caching.

**In code:** the trace request usage carries `cache_reset` (`reason`,
`lost_tokens`, `previous_cached_tokens`, `idle_seconds`) and
`caching_would_help`.
