---
title: OpenRouter Server Tools
description: Enable OpenRouter's provider-executed server tools, web search, web fetch, datetime, image generation, and more. OpenRouter runs them server-side and returns the final answer; non-OpenRouter providers ignore the setting.
sidebar:
  order: 95
---

| | |
|---|---|
| **ID** | `openrouter_server_tools` |
| **Category** | Tools |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | High (grants provider-executed web reach) |

Enables [OpenRouter's provider-executed "server tools"](https://openrouter.ai/docs/guides/features/server-tools)
(beta). Unlike normal [function tools](/features/capabilities/), these run
**server-side by OpenRouter**: it loops internally and returns the final answer,
so the agent loop never dispatches them. This capability contributes *request
intent*, not executable tools, the selected tools are compiled into the
OpenRouter request's `tools` array as provider-executed entries.
The concrete implementation lives in the focused
`everruns-integrations-openrouter-workspace` crate; core carries only the
provider-neutral routing contract.

This is the OpenRouter counterpart to client-executed web access like
[Web Fetch](/capabilities/web-fetch/): the difference is *who runs the tool*.
With server tools, OpenRouter performs the search or fetch and folds the results
into the same generation, no extra round-trip through Everruns. Use it when your
agents run on the [OpenRouter provider](/providers/openrouter/) and you want
built-in web reach without wiring up a separate search [integration](/integrations/).

## Tools

None, this capability configures the OpenRouter request, it does not provide
client-side tools. The model invokes server tools during the generation and
OpenRouter executes them; the only client-visible artifact is the final answer.

## Available server tools

OpenRouter exposes these server tools. Enable any subset:

| Tool | Name | What it does |
|---|---|---|
| Web Search | `web_search` | Searches the web and grounds the answer in results. Accepts an optional `max_results` cap. |
| Web Fetch | `web_fetch` | Fetches and reads a URL the model chooses. |
| Date & Time | `datetime` | Gives the model the current date and time. |
| Image Generation | `image_generation` | Generates images inline. |
| Apply Patch | `apply_patch` | Applies code patches. |
| Fusion | `fusion` | OpenRouter's Fusion tool. |
| Advisor | `advisor` | OpenRouter's Advisor tool. |
| Subagent | `subagent` | Delegates to an OpenRouter-run subagent. |

`web_search` is the only server tool that takes parameters today
(`web_search_max_results`). Availability of each tool depends on the upstream
model and OpenRouter's beta rollout, see
[OpenRouter's server-tools docs](https://openrouter.ai/docs/guides/features/server-tools)
for the current list.

## How it works

1. **Capability config → request intent**: the tools you enable are compiled
   into the OpenRouter routing config and serialized by the OpenRouter driver
   into the request's `tools` array as `{"type":"openrouter:…"}` entries.
2. **OpenRouter executes server-side**: when the model decides to call a server
   tool, OpenRouter runs it, loops internally, and returns the final answer. The
   agent loop never sees an intermediate tool call.
3. **No-op off OpenRouter**: non-OpenRouter providers ignore the routing config
   entirely. Enabling this capability on a non-OpenRouter agent is a harmless
   no-op, so it is safe to leave on for agents that may switch providers.

## Configuration

### Enable web search

```json
{
  "capabilities": [
    {
      "capability_ref": "openrouter_server_tools",
      "config": { "tools": ["web_search"] }
    }
  ]
}
```

### Enable several tools and cap web-search results

```json
{
  "capabilities": [
    {
      "capability_ref": "openrouter_server_tools",
      "config": {
        "tools": ["web_search", "web_fetch", "datetime"],
        "web_search_max_results": 5
      }
    }
  ]
}
```

Config rules:

- `tools`, array of server-tool names from the table above. Unknown names are
  rejected on write. Duplicates are de-duplicated.
- `web_search_max_results`, positive integer; only decorates `web_search`. It is
  ignored for every other tool and rejected when `< 1`.

## Security

Enabling server tools grants the model **provider-executed web reach**
(`web_search` / `web_fetch`). OpenRouter performs these requests, so Everruns'
own egress controls do not apply, the same data-exfiltration class as client-side
[Web Fetch](/capabilities/web-fetch/). The capability is therefore rated **High
risk** and gated behind the same admin-only trust check as other outbound-web
capabilities. Grant it only to agents you trust with outbound web access.

## Limitations

- **OpenRouter only**: this is an OpenRouter request extension. Other providers
  ignore it (no error, no behavior change).
- **Beta**: server tools are an OpenRouter beta; tool availability varies by
  upstream model and may change.
- **Provider-side execution**: because OpenRouter runs the tools, their activity
  does not appear as Everruns tool calls. Inspect them in OpenRouter's
  [dashboard logs](/providers/openrouter/#logs-traces-and-observability) instead.

## See Also

- [OpenRouter provider](/providers/openrouter/), configure the provider these
  tools run on, plus OAuth, actual-cost reporting, and logs
- [OpenRouter server-tools docs](https://openrouter.ai/docs/guides/features/server-tools), official OpenRouter guide
- [Web Fetch](/capabilities/web-fetch/), the client-executed equivalent
- [Integrations overview](/integrations/), search and web integrations as an alternative
- [Capabilities Overview](/capabilities/)
