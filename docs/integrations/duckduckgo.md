---
title: DuckDuckGo
description: Give agents free instant answers, definitions, and topic summaries via DuckDuckGo. No API key required, lightweight search for general knowledge queries.
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" fill="currentColor" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M12 0C5.37 0 0 5.37 0 12s5.37 12 12 12 12-5.37 12-12S18.63 0 12 0zm0 .984C18.083.984 23.016 5.916 23.016 12S18.084 23.016 12 23.016.984 18.084.984 12C.984 5.917 5.916.984 12 .984zm0 .938C6.434 1.922 1.922 6.434 1.922 12c0 4.437 2.867 8.205 6.85 9.55-.237-.82-.776-2.753-1.6-6.052-1.184-4.741-2.064-8.606 2.379-9.813.047-.011.064-.064.03-.093-.514-.467-1.382-.548-2.233-.38a.06.06 0 0 1-.07-.058c0-.011 0-.023.011-.035.205-.286.572-.507.822-.64a1.843 1.843 0 0 0-.607-.335c-.059-.022-.059-.12-.006-.144.006-.006.012-.012.024-.012 1.749-.233 3.586.292 4.49 1.448.011.011.023.017.035.023 2.968.635 3.509 4.837 3.328 5.998a9.607 9.607 0 0 0 2.346-.576c.746-.286 1.008-.222 1.101-.053.1.193-.018.513-.28.81-.496.567-1.393 1.01-2.974 1.137-.546.044-1.029.024-1.445.006-.789-.035-1.339-.059-1.633.39-.192.298-.041.998 1.487 1.22 1.09.157 2.078.047 2.798-.034.643-.07 1.073-.118 1.172.069.21.402-.996 1.207-3.066 1.224-.158 0-.315-.006-.467-.011-1.283-.065-2.227-.414-2.816-.735a.094.094 0 0 1-.035-.017c-.105-.059-.31.045-.188.267.07.134.444.478 1.004.776-.058.466.087 1.184.338 2l.088-.016c.041-.009.087-.019.134-.025.507-.082.775.012.926.175.717-.536 1.913-1.294 2.03-1.154.583.694.66 2.332.53 2.99-.004.012-.017.024-.04.035-.274.117-1.783-.296-1.783-.511-.059-1.075-.26-1.173-.493-1.225h-.156c.006.006.012.018.018.03l.052.12c.093.257.24 1.063.13 1.26-.112.199-.835.297-1.284.303-.443.006-.543-.158-.637-.408-.07-.204-.103-.675-.103-.95a.857.857 0 0 1 .012-.216c-.134.058-.333.193-.397.281-.017.262-.017.682.123 1.149.07.221-1.518 1.164-1.74.99-.227-.181-.634-1.952-.459-2.67-.187.017-.338.075-.42.191-.367.508.093 2.933.582 3.248.257.169 1.54-.553 2.176-1.095.105.145.305.158.553.158.326-.012.782-.06 1.103-.158.192.45.423.972.613 1.388 4.47-1.032 7.803-5.037 7.803-9.82 0-5.566-4.512-10.078-10.078-10.078zm1.791 5.646c-.42 0-.678.146-.795.332-.023.047.047.094.094.07.14-.075.357-.161.701-.156.328.006.516.09.67.159l.023.01c.041.017.088-.03.059-.065-.134-.18-.332-.35-.752-.35zm-5.078.198a1.24 1.24 0 0 0-.522.082c-.454.169-.67.526-.67.76 0 .051.112.057.141.011.081-.123.21-.31.617-.478.408-.17.73-.146.951-.094.047.012.083-.041.041-.07a.989.989 0 0 0-.558-.211zm5.434 1.423a.651.651 0 0 0-.655.647.652.652 0 0 0 1.307 0 .646.646 0 0 0-.652-.647zm.283.262h.008a.17.17 0 0 1 .17.17c0 .093-.077.17-.17.17a.17.17 0 0 1-.17-.17c0-.09.072-.165.162-.17zm-5.358.076a.752.752 0 0 0-.758.758c0 .42.338.758.758.758s.758-.337.758-.758a.756.756 0 0 0-.758-.758zm.328.303h.01c.112 0 .2.089.2.2 0 .11-.088.197-.2.197a.195.195 0 0 1-.197-.198c0-.107.082-.194.187-.199z"/></svg>

Everruns integrates with [DuckDuckGo](https://duckduckgo.com/) to provide instant answers via the [DuckDuckGo Instant Answer API](https://api.duckduckgo.com/api). Agents can look up facts, definitions, topic summaries (from Wikipedia and other sources), and related topics, all without an API key.

## What You Get

- **Instant Answers**: Direct answers for calculations, IP lookups, conversions, and more
- **Topic Abstracts**: Wikipedia-style summaries for well-known topics
- **Definitions**: Dictionary definitions from Wiktionary and other sources
- **Related Topics**: Links to related topics for deeper exploration
- **No API Key Required**: The DuckDuckGo Instant Answer API is completely free

## Quick Start

### 1. No Setup Needed

Unlike other integrations, DuckDuckGo requires no API key or configuration. The DuckDuckGo Instant Answer API is free and public.

### 2. Enable the Capability

Add the `duckduckgo` capability to your agent or harness configuration. In dev mode, it's available as an experimental capability.

### 3. Use in Sessions

Agents with the DuckDuckGo capability can use this tool:

| Tool | Description |
|------|-------------|
| `duckduckgo_instant_answer` | Look up instant answers, abstracts, definitions, and related topics. Instant-answer lookup only, not a full web/SERP search |

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | Yes | Search query |
| `no_html` | boolean | No | Strip HTML from result text (default: `true`) |

### Response Fields

The tool returns a JSON object with available fields:

| Field | Description |
|-------|-------------|
| `query` | The original query |
| `type` | Response type: `article`, `disambiguation`, `category`, `name`, `exclusive`, or `nothing` |
| `heading` | Topic heading |
| `abstract` | `{ text, source, url }`, topic summary |
| `answer` | `{ text, type }`, direct answer (calculations, etc.) |
| `definition` | `{ text, source, url }`, dictionary definition |
| `related_topics` | Array of `{ text, url }`, related topics (max 10) |
| `results` | Array of `{ text, url }`, official/direct results |
| `note` | Present only when no instant answer was found, a caveat that this is not a definitive web-search result and matching web pages may still exist |

Only non-empty fields are included in the response.

:::note
This tool queries the DuckDuckGo **Instant Answer** API, not full web search.
A `nothing` result (or a `note` field) means DuckDuckGo has no curated instant
answer for the query, it does **not** mean no web pages match. Prefer a
web-search or web-fetch tool (or Brave Search) for general web discovery; when
none is available, this tool can still serve as a lightweight search for quick
facts, definitions, and related topics.
:::

## When to Use DuckDuckGo vs Brave Search

| Use Case | DuckDuckGo | Brave Search |
|----------|------------|--------------|
| Quick facts and definitions | ✅ Best choice | Works but slower |
| Wikipedia-style summaries | ✅ Best choice | Not available |
| Calculations and conversions | ✅ Direct answers | Not available |
| Full web search results | ❌ Not available | ✅ Best choice |
| Current news and articles | ❌ Limited | ✅ Best choice |
| API key required | No | Yes |

Both capabilities can be enabled simultaneously, the agent will choose the right tool based on the task.

## Security

- **No secrets**: No API key or credentials to manage
- **Read-only**: The API only returns information, no write operations
- **Privacy**: DuckDuckGo does not track searches

## Status

**Experimental**: available in dev mode only. This capability may change in future releases.

## Links

- [DuckDuckGo](https://duckduckgo.com/)
- [DuckDuckGo Instant Answer API](https://api.duckduckgo.com/api)
