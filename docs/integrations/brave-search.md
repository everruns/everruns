---
title: Brave Search
description: Web search for agents via the Brave Search API
---

Everruns integrates with [Brave Search](https://brave.com/search/api/) to give agents full web search capabilities. Agents can search the web and get relevant results including titles, URLs, and descriptions — perfect for research, fact-checking, and finding current information.

## What You Get

- **Full Web Search**: Query the web and get ranked results with titles, URLs, and descriptions
- **Freshness Filters**: Filter results by time (past day, week, month, year)
- **Pagination**: Navigate through large result sets
- **Source Attribution**: Every result includes a URL for citation and verification
- **Free Tier**: Brave Search offers a free plan with 2,000 queries/month

## Quick Start

### 1. Get Your API Key

1. Go to [Brave Search API](https://brave.com/search/api/)
2. Sign up for a **free** plan (2,000 queries/month)
3. Copy your **API Key** from the dashboard

### 2. Connect in Everruns

1. Go to **Settings** > **Connections**
2. Find **Brave Search** in the available providers
3. Click **Connect** and paste your API key

Once connected, the Brave Search capability is automatically available in agent sessions.

### 3. Use in Sessions

Agents with the Brave Search capability can use this tool:

| Tool | Description |
|------|-------------|
| `brave_web_search` | Search the web and return relevant results |

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | Yes | Search query |
| `count` | integer | No | Number of results (1-20, default: 10) |
| `offset` | integer | No | Pagination offset |
| `freshness` | string | No | Time filter: `pd` (past day), `pw` (past week), `pm` (past month), `py` (past year) |

### Response Fields

The tool returns a JSON object with:

| Field | Description |
|-------|-------------|
| `query` | The original query |
| `results` | Array of result objects |
| `count` | Number of results returned |

Each result object contains:

| Field | Description |
|-------|-------------|
| `title` | Page title |
| `url` | Page URL (use for citations) |
| `description` | Snippet/description of the page |
| `age` | How old the result is (e.g., "2 hours ago") — optional |

## When to Use Brave Search vs DuckDuckGo

| Use Case | Brave Search | DuckDuckGo |
|----------|--------------|------------|
| Full web search results | Best choice | Not available |
| Current news and articles | Best choice | Limited |
| Quick facts and definitions | Works but slower | Best choice |
| Wikipedia-style summaries | Not available | Best choice |
| Calculations and conversions | Not available | Direct answers |
| API key required | Yes (free tier available) | No |

Both capabilities can be enabled simultaneously — the agent will choose the right tool based on the task.

## Security

- API keys are encrypted at rest (AES-256-GCM envelope encryption)
- Keys are validated on connection (test query to Brave Search API)
- API key never appears in tool results or message history
- Rate limiting deferred to Brave Search API (returns 429 on limit)

## Status

**Experimental** — available in dev mode only. This capability may change in future releases.

## Links

- [Brave Search API](https://brave.com/search/api/)
- [Brave Search API Documentation](https://api.search.brave.com/app/#/documentation/web-search)
