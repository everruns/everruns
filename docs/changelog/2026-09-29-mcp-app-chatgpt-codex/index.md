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

![Everruns and OpenAI logos side by side, announcing the MCP App in ChatGPT and Codex](./integration@2x.png)

The Everruns MCP server at `/mcp` is now an MCP App. Supported hosts show a
session view with status, messages and a reply box. Questions appear as a form,
and approvals refuse stale clicks. An Everruns panel opens from the ChatGPT
sidebar. Hosts without app rendering retain text tools.

See [MCP: In ChatGPT and Codex](/features/mcp/#in-chatgpt-and-codex).

## Update 2026-11-02

Agent cards use MCP Apps templates instead of exposing raw HTML, with a text
summary for clients without app rendering.

## Update 2026-11-03

Agent, Harness, Capability and Knowledge cards now reuse the product UI. Home
and Session open the full workspaces, including Session Trace. See
[the shared workspace update](/changelog/2026-11-03-mcp-shared-workspaces/).
