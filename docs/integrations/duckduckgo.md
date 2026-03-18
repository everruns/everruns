---
title: DuckDuckGo
description: Give agents free instant answers, definitions, and topic summaries via DuckDuckGo. No API key required — lightweight search for general knowledge queries.
---

Everruns integrates with [DuckDuckGo](https://duckduckgo.com/) to provide instant answers via the [DuckDuckGo Instant Answer API](https://api.duckduckgo.com/api). Agents can look up facts, definitions, topic summaries (from Wikipedia and other sources), and related topics — all without an API key.

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
| `duckduckgo_search` | Get instant answers, abstracts, definitions, and related topics |

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
| `abstract` | `{ text, source, url }` — topic summary |
| `answer` | `{ text, type }` — direct answer (calculations, etc.) |
| `definition` | `{ text, source, url }` — dictionary definition |
| `related_topics` | Array of `{ text, url }` — related topics (max 10) |
| `results` | Array of `{ text, url }` — official/direct results |

Only non-empty fields are included in the response.

## When to Use DuckDuckGo vs Brave Search

| Use Case | DuckDuckGo | Brave Search |
|----------|------------|--------------|
| Quick facts and definitions | ✅ Best choice | Works but slower |
| Wikipedia-style summaries | ✅ Best choice | Not available |
| Calculations and conversions | ✅ Direct answers | Not available |
| Full web search results | ❌ Not available | ✅ Best choice |
| Current news and articles | ❌ Limited | ✅ Best choice |
| API key required | No | Yes |

Both capabilities can be enabled simultaneously — the agent will choose the right tool based on the task.

## Security

- **No secrets**: No API key or credentials to manage
- **Read-only**: The API only returns information, no write operations
- **Privacy**: DuckDuckGo does not track searches

## Status

**Experimental** — available in dev mode only. This capability may change in future releases.

## Links

- [DuckDuckGo](https://duckduckgo.com/)
- [DuckDuckGo Instant Answer API](https://api.duckduckgo.com/api)
