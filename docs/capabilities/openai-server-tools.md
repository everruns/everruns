---
title: OpenAI Server Tools
description: Enable OpenAI's hosted tools (web search, code interpreter, hosted shell, file search, remote MCP) on agents that run on the OpenAI or Azure OpenAI Responses API. OpenAI runs them inside the response.
sidebar:
  order: 94
---

| | |
|---|---|
| **ID** | `openai_server_tools` |
| **Category** | Tools |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | High (grants provider-executed web reach and code execution) |

Enables [OpenAI's hosted tools](https://platform.openai.com/docs/guides/tools)
on agents whose model runs on the [OpenAI](/providers/openai/) or Azure OpenAI
provider. OpenAI runs a hosted tool inside the same response: the model decides to
search, OpenAI performs the search, and the answer comes back grounded in the
results with its sources cited. The agent loop never dispatches the call, so there
is no extra round-trip through Everruns.

| Tool | What OpenAI runs |
|---|---|
| `web_search` | Searches the web and cites the pages it used |
| `code_interpreter` | Python in an OpenAI-managed container |
| `shell` | Shell commands in an OpenAI-managed container |
| `file_search` | Retrieval over your OpenAI vector stores |
| `mcp` | Calls to remote MCP servers you list, after approval |

## Tools

None. This capability configures the OpenAI request, it does not provide
client-side tools. Hosted calls appear in the session's activity (see
[Activity and cost](#activity-and-cost)); their results stay inside OpenAI's
response and reach the conversation only through the answer.

Code interpreter and shell run in OpenAI's container, not in the session's
Everruns sandbox: files they create are not in the session file system. Use
Everruns' own execution capabilities when the agent's work must land there.

## Configuration

### Enable web search

```json
{
  "capabilities": [
    {
      "capability_ref": "openai_server_tools",
      "config": { "tools": ["web_search"] }
    }
  ]
}
```

### Tune web search

```json
{
  "capabilities": [
    {
      "capability_ref": "openai_server_tools",
      "config": {
        "tools": ["web_search"],
        "web_search_context_size": "high",
        "web_search_allowed_domains": ["openai.com", "docs.everruns.com"],
        "web_search_user_location": { "country": "US", "city": "Chicago", "timezone": "America/Chicago" }
      }
    }
  ]
}
```

### Code, shell, and file search

```json
{
  "capabilities": [
    {
      "capability_ref": "openai_server_tools",
      "config": {
        "tools": ["code_interpreter", "shell", "file_search"],
        "container_memory_limit": "4g",
        "file_search_vector_store_ids": ["vs_abc123"],
        "file_search_max_results": 8
      }
    }
  ]
}
```

### Remote MCP

```json
{
  "capabilities": [
    {
      "capability_ref": "openai_server_tools",
      "config": {
        "tools": ["mcp"],
        "mcp_servers": [
          { "server_label": "deepwiki", "server_url": "https://mcp.deepwiki.com/mcp" },
          {
            "server_label": "docs",
            "server_url": "https://docs.example.com/mcp",
            "allowed_tools": ["search"],
            "require_approval": "never"
          }
        ]
      }
    }
  ]
}
```

OpenAI connects to each server and asks before every call. The turn pauses
with an approval card in the chat that shows the server, the tool, and the
exact arguments; **Approve** runs the call and **Deny** lets the model answer
without it. API clients see the request as a `tool.call_requested` event with
an `openai_mcp_approval` call and answer it through
`POST /v1/sessions/{id}/tool-results` with `{"approve": true}` or
`{"approve": false}`. Any other result, an error included, denies.

A server can skip approval with `"require_approval": "never"`, but only for an
explicit `allowed_tools` list, so every unattended call is a tool you named.

A `server_url` must be `https` and must not carry a username or password, so
it suits public servers. A server that needs an API key or an OAuth
connection is named by `mcp_server`, the name of an MCP server registered in
Everruns, instead of a URL:

```json
{ "server_label": "linear", "mcp_server": "linear" }
```

Its URL and credentials come from the MCP server registration and the
session's connections on every call, exactly as for the agent's own MCP tools,
and never appear in agent config, events, or logs. If the connection is
missing, the turn fails with a message naming where to connect it. Servers
that bind secrets to tool parameters cannot be used this way, because OpenAI
calls them directly.

Config rules:

- `tools`, array of hosted tool names: `web_search`, `code_interpreter`,
  `shell`, `file_search`, `mcp`. Other names are rejected on write.
- `web_search_context_size`, `low`, `medium`, or `high`: how much retrieved
  context the model may use per search. OpenAI defaults to `medium`.
- `web_search_allowed_domains`, bare domains (no scheme or path) that limit where
  results may come from.
- `web_search_user_location`, optional `country` (two-letter ISO code), `region`,
  `city`, and `timezone` (IANA name) used to localize results.
- `container_memory_limit`, `1g`, `4g`, `16g`, or `64g`: memory for the
  container that runs `code_interpreter` and `shell`. OpenAI defaults to `1g`;
  larger containers cost more.
- `file_search_vector_store_ids`, OpenAI vector store ids (`vs_...`) in the same
  OpenAI account as the provider's API key. Required when `file_search` is on.
- `file_search_max_results`, 1 to 50 results per search.
- `mcp_servers`, at least one server when `mcp` is on. Each has a unique
  `server_label` (letters, digits, `-`, `_`), either an `https` `server_url` or
  a registered `mcp_server` name, optional
  `allowed_tools`, and `require_approval` (`always`, the default, or `never`).

Each option only takes effect when its tool is in `tools`.

## Provider support

Hosted tools run only on the OpenAI and Azure OpenAI Responses API. On any other
provider, including OpenAI-compatible gateways, the turn fails with a message
that names the provider, instead of answering without the tools. Switch the
agent's model or remove the capability. For OpenRouter models, use
[OpenRouter Server Tools](/capabilities/openrouter-server-tools/).

Each model decides which hosted tools it supports; OpenAI rejects a request that
asks a model for a tool it does not offer, and that error is shown on the turn.

## Security

Hosted tools send conversation content to OpenAI-side tools. Web search gives
the model **provider-executed web reach**, code interpreter and shell give it
**code execution in an OpenAI container**, and file search reads whatever the
listed vector stores hold. Remote MCP sends the model's tool arguments to the
servers you list, which are third parties to both OpenAI and Everruns; approval
shows each call before it happens. OpenAI performs the requests, so Everruns' own
egress controls do not apply, the same data-exfiltration class as client-side
[Web Fetch](/capabilities/web-fetch/). The capability is rated **High risk** and
uses the admin-only assignment gate. Grant it only to agents you trust with
outbound web access and code execution, list only vector stores whose
contents every user of the agent may see, and list only MCP servers you trust
with the conversation.

## Activity and cost

Each hosted call shows up in the session's activity as it happens, through the
`tool.hosted_call` event, with a short detail: the search query, the first line
of code, the shell commands, the file search query, or the MCP server and tool. It is not an Everruns
tool call: nothing is dispatched and no tool result is stored.

OpenAI bills hosted tool calls per call on top of tokens. Session usage adds them
to the estimated cost at OpenAI's list price: $10 per 1,000 web searches on GPT-5
and newer reasoning models, $25 per 1,000 on GPT-4o and GPT-4.1, and $2.50 per
1,000 file searches. Code interpreter and shell are billed by OpenAI per
container session rather than per call, so they are not in the estimate; check
OpenAI's usage dashboard for them. File search also bills vector store storage.

## See Also

- [OpenAI provider](/providers/openai/)
- [OpenRouter Server Tools](/capabilities/openrouter-server-tools/), the OpenRouter counterpart
- [Web Fetch](/capabilities/web-fetch/), the client-executed equivalent
- [Capabilities Overview](/capabilities/)
