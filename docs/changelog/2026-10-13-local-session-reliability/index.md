---
title: "More reliable Framework agent sessions"
description: "Local background tasks recover after owner loss, MCP discovery pauses failed authentication, and provider tool calls preserve optional arguments."
date: "2026-10-13"
products: [framework]
kind: fix
significance: notable
surfaces: [code, agent]
---

Framework hosts built with `LocalRuntimeBuilder` now settle background tasks whose executor is known lost.
Recovery checks ownership and the current attempt atomically, preserves live and
unknown owners, and never replays interrupted commands. Losing an owner process
reports an unverified execution outcome rather than confirming cancellation.
Terminal recovery notifications persist across restarts. See [local host deployment](/framework/deployment/#advanced-local-hosts).

Background execution checks cancellation before starting a tool. Task lists keep
diagnostic summaries compact, with full details available through `get_task`.

MCP discovery pauses authentication failures until reload or changed configuration;
transient failures retry after a short cooldown. Rejected tool calls restore their
full schemas on the next step. Strict provider tool calls remove only optional null
placeholders before recording or approval. OpenRouter daily limits fail without
short retries and preserve the supplied reset time.
