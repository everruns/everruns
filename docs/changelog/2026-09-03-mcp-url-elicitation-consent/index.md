---
title: MCP URL elicitation pauses for consent in the Everruns UI
description: When an MCP tool asks a person to finish a step in the browser, the turn now pauses on a consent card and the secret never enters the model context.
date: 2026-09-03
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, agent]
release: "0.24.0"
links:
  docs: [/features/mcp-url-elicitation/, /how-to/complete-a-url-elicitation/]
  prs: [everruns/everruns#3354]
visual: "Chat turn paused on the confirm URL elicitation card in a session (/chats/<id>); crop to the card and the message above it"
---

An MCP server can now ask a person to finish something in their browser, such as
authorizing a charge, pasting an API key or clicking a consent screen, and Everruns
handles it as both MCP client and MCP server. The value never passes through the
MCP client, the model's context or the event log.

**In the UI:** a tool call that stops on a URL elicitation pauses the turn on a
confirmation card. The consent is single use and bound to the server, tool and
domain you were shown.

**For MCP clients of Everruns:** the `/mcp` endpoint gains `session_set_secret`
and `connect`, which point you at a form Everruns serves instead of taking the value.

See [MCP URL elicitation](/features/mcp-url-elicitation/) and
[Complete a URL elicitation](/how-to/complete-a-url-elicitation/). [#3354](https://github.com/everruns/everruns/pull/3354)
