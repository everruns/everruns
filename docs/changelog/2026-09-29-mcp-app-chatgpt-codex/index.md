---
title: Everruns MCP server renders as an MCP App in ChatGPT and Codex
description: MCP App hosts such as ChatGPT and Codex show a live session view, answer agent questions in a form, approve pending actions and open an Everruns panel.
date: 2026-09-29
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [agent]
release: "0.33.0"
links:
  docs: [/features/mcp/]
  prs: [everruns/everruns#3927]
---

The Everruns MCP server at `/mcp` is now an MCP App. In hosts that render MCP
Apps, such as ChatGPT and Codex, running an agent shows a session view with
status, messages and a reply box. Agent questions appear as a form, and pending
approvals as Approve and Decline buttons that refuse stale clicks. An Everruns
panel opens from the ChatGPT sidebar.

Hosts that do not render apps get the same tools as plain text, and the
existing tool shapes are unchanged.

See [MCP: In ChatGPT and Codex](/features/mcp/#in-chatgpt-and-codex).
PR [#3927](https://github.com/everruns/everruns/pull/3927).
