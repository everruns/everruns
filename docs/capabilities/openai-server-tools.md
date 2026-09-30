---
title: OpenAI Server Tools
description: Enable OpenAI's hosted tools, starting with web search, on agents that run on the OpenAI or Azure OpenAI Responses API. OpenAI runs them inside the response.
sidebar:
  order: 94
---

| | |
|---|---|
| **ID** | `openai_server_tools` |
| **Category** | Tools |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | High (grants provider-executed web reach) |

Enables [OpenAI's hosted tools](https://platform.openai.com/docs/guides/tools-web-search)
on agents whose model runs on the [OpenAI](/providers/openai/) or Azure OpenAI
provider. OpenAI runs a hosted tool inside the same response: the model decides to
search, OpenAI performs the search, and the answer comes back grounded in the
results with its sources cited. The agent loop never dispatches the call, so there
is no extra round-trip through Everruns.

Web search is available today. Code interpreter, file search, and remote MCP are
planned for this capability.

## Tools

None. This capability configures the OpenAI request, it does not provide
client-side tools. The only client-visible artifact is the answer, which cites the
pages it used as links.

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

Config rules:

- `tools`, array of hosted tool names. Only `web_search` is accepted today;
  other names are rejected on write.
- `web_search_context_size`, `low`, `medium`, or `high`: how much retrieved
  context the model may use per search. OpenAI defaults to `medium`.
- `web_search_allowed_domains`, bare domains (no scheme or path) that limit where
  results may come from.
- `web_search_user_location`, optional `country` (two-letter ISO code), `region`,
  `city`, and `timezone` (IANA name) used to localize results.

The web search options only take effect when `web_search` is in `tools`.

## Provider support

Hosted tools run only on the OpenAI and Azure OpenAI Responses API. On any other
provider, including OpenAI-compatible gateways, the turn fails with a message
that names the provider, instead of answering without the tools. Switch the
agent's model or remove the capability. For OpenRouter models, use
[OpenRouter Server Tools](/capabilities/openrouter-server-tools/).

Each model decides which hosted tools it supports; OpenAI rejects a request that
asks a model for a tool it does not offer, and that error is shown on the turn.

## Security

Web search sends conversation content to OpenAI-side tools and gives the model
**provider-executed web reach**. OpenAI performs the requests, so Everruns' own
egress controls do not apply, the same data-exfiltration class as client-side
[Web Fetch](/capabilities/web-fetch/). The capability is rated **High risk** and
uses the admin-only assignment gate. Grant it only to agents you trust with
outbound web access.

## Limitations

- **Hosted calls are not Everruns tool calls**: a search does not appear in the
  session as a tool call yet; the answer's citations show what was used.
- **Billing**: OpenAI bills hosted tool calls per call on top of tokens. Everruns
  logs the per-call counts but does not yet price them in session usage.

## See Also

- [OpenAI provider](/providers/openai/)
- [OpenRouter Server Tools](/capabilities/openrouter-server-tools/), the OpenRouter counterpart
- [Web Fetch](/capabilities/web-fetch/), the client-executed equivalent
- [Capabilities Overview](/capabilities/)
