---
title: GitHub's MCP server works for repository tools
description: Agents send the Mcp-Param headers that MCP servers ask for, so GitHub's pull request, issue and Actions tools stop failing with a header mismatch.
date: 2026-11-19
kind: fix
significance: minor
products: [platform, framework]
surfaces: [agent]
links:
  docs: [/features/mcp/]
---

Every [MCP](/features/mcp/) tool on GitHub's official server that takes an `owner` and a
`repo`, such as reading a pull request, listing Actions runs or merging, failed with
`400 header mismatch: missing Mcp-Param-owner header`. Only the search tools worked.

The server marks those parameters with `x-mcp-header`, a 2026-07-28 addition that
asks clients to repeat the argument as an `Mcp-Param-owner` request header. Agents
now read these marks when they list a server's tools and send the headers on every
call. A value that is not plain ASCII travels Base64-encoded, so a tool argument
can never add a header of its own. When a server still rejects a call, the agent
reloads the tool list and retries once.

A tool whose marks break the specification's rules is left out of the tool list
with a warning, and the server's other tools stay available.
