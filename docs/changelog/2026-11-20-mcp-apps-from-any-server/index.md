---
title: "MCP servers can draw their results in the chat"
description: "When a connected MCP server ships an interactive view with a tool, the chart or table shows under the tool call, sandboxed, and in Slack as a picture."
date: 2026-11-20
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, agent]
visual: "Session chat with a connected server's chart view open under its tool call, server label, Expand and Show data visible; crop to the tool row and the view"
links:
  docs: [/features/mcp/#interactive-views-mcp-apps]
---

Ask an agent with Sentry connected how many errors hit checkout today, and the
answer comes back as Sentry's own chart, right under the tool call, instead of
a wall of numbers. Any MCP server that ships an
[MCP Apps](https://github.com/modelcontextprotocol/ext-apps) view works the
same way: **Expand** opens it full screen, **Show data** shows what it was drawn
from.

Views run sandboxed and can only ask: read-only tool calls run, anything else
waits for your OK, and a message a view suggests lands in the chat box as a
draft. In Slack you get a picture of the view and an **Open interactive view**
button. Turn it off, or keep it to the console, per server under
**Interactive views**. See
[Interactive views](/features/mcp/#interactive-views-mcp-apps).
