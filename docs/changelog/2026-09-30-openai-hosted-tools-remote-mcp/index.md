---
title: OpenAI hosted code interpreter, shell, file search and remote MCP
description: The openai_server_tools capability adds code interpreter, hosted shell, file search and approval-gated remote MCP next to web search on OpenAI Responses agents.
date: 2026-09-30
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [api, agent]
release: "0.33.0"
links:
  docs: [/capabilities/openai-server-tools/]
  prs: [everruns/everruns#3918, everruns/everruns#3923, everruns/everruns#3928, everruns/everruns#3930]
---

Agents on the OpenAI or Azure OpenAI Responses API can let OpenAI run tools
inside the response. The `openai_server_tools` capability accepts
`web_search`, `code_interpreter`, `shell`, `file_search` and `mcp`:

```json
{ "ref": "openai_server_tools",
  "config": { "tools": ["code_interpreter", "shell", "file_search"],
              "file_search_vector_store_ids": ["vs_abc123"] } }
```

Hosted calls show in the work log and are priced in usage. Each remote MCP
call pauses on an approval card by default. A server that needs credentials is
named by its registered Everruns MCP server, and its URL and token are
resolved per call, never stored in agent config or events.

See [OpenAI server tools](/capabilities/openai-server-tools/). PRs
[#3918](https://github.com/everruns/everruns/pull/3918), [#3923](https://github.com/everruns/everruns/pull/3923), [#3928](https://github.com/everruns/everruns/pull/3928), [#3930](https://github.com/everruns/everruns/pull/3930).
