---
title: Provider retries preserve the initiating user
description: Short provider retries preserve verified user and responder context, allowing interrupted personal MCP setup to continue without an extra chat message.
date: 2026-10-27
cloud: 2026-10-28
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [agent, operations]
links:
  docs: [/capabilities/user-mcp-servers/]
---

Agents managing [personal MCP connections](/capabilities/user-mcp-servers/) can
continue interrupted setup after a short provider throttle. An automatic retry
retains the original verified user and responding agent, so connection tools
continue to recognize the person who requested the work.

Each retry must match a recent platform failure event and its original
invocation. User-created schedules cannot supply that proof. The person
completes OAuth through the normal connection card.

Tool guidance also distinguishes automatic question defaults from human answers
and tells agents to report blocked or unverified requested work as incomplete.
