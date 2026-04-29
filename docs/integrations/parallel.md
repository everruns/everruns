---
title: Parallel
description: Use Parallel's hosted MCP server for free web search and URL fetching, with optional API-key authentication and OAuth-compatible endpoint selection.
---

# Parallel

Parallel provides hosted MCP tools for web search and URL fetching.

## Setup

Add the `parallel_search` capability to an agent or harness. It works for free without any connection.

To use a Parallel API key, add a `Parallel` connection in Settings > Connections, then configure the capability with `auth: "connection"`.

To use Parallel's OAuth-compatible MCP endpoint, configure the capability with `endpoint: "oauth"`. This mode requires the `Parallel` connection because the endpoint rejects anonymous requests.

## Tools

| Tool | Purpose |
|---|---|
| `mcp_parallel__web_search` | Search the web and return ranked URLs with excerpts. |
| `mcp_parallel__web_fetch` | Fetch and extract focused content from known URLs. |

Agents should reuse one stable `session_id` across Parallel tool calls in the same conversation.
