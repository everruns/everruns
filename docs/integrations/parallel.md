---
title: Parallel
description: Use Parallel's hosted MCP server for free web search and URL fetching, with optional API-key authentication and OAuth-compatible endpoint selection.
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M7 4v16M12 4v16M17 4v16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>

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

## Paid machine payments

Operators can separately enable Parallel's paid search, extraction, and task tools with
`FEATURE_MACHINE_PAYMENTS=true`. This deployment flag is off by default in every environment.
When it is off, Everruns does not expose Settings > Payments or the payment account, policy, and
attempt APIs, so the deployment does not ask organization owners to entrust wallet keys for a
capability that cannot spend.
