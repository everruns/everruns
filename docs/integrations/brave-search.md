---
title: Brave Search
description: Give agents live web search with Brave Search, ranked results, freshness filters, pagination, source attribution, and safe search. Requires a Brave API key.
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" fill="currentColor" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M15.68 0l2.096 2.38s1.84-.512 2.709.358c.868.87 1.584 1.638 1.584 1.638l-.562 1.381.715 2.047s-2.104 7.98-2.35 8.955c-.486 1.919-.818 2.66-2.198 3.633-1.38.972-3.884 2.66-4.293 2.916-.409.256-.92.692-1.38.692-.46 0-.97-.436-1.38-.692a185.796 185.796 0 01-4.293-2.916c-1.38-.973-1.712-1.714-2.197-3.633-.247-.975-2.351-8.955-2.351-8.955l.715-2.047-.562-1.381s.716-.768 1.585-1.638c.868-.87 2.708-.358 2.708-.358L8.321 0h7.36zm-3.679 14.936c-.14 0-1.038.317-1.758.69-.72.373-1.242.637-1.409.742-.167.104-.065.301.087.409.152.107 2.194 1.69 2.393 1.866.198.175.489.464.687.464.198 0 .49-.29.688-.464.198-.175 2.24-1.759 2.392-1.866.152-.108.254-.305.087-.41-.167-.104-.689-.368-1.41-.741-.72-.373-1.617-.69-1.757-.69zm0-11.278s-.409.001-1.022.206-1.278.46-1.584.46c-.307 0-2.581-.434-2.581-.434S4.119 7.152 4.119 7.849c0 .697.339.881.68 1.243l2.02 2.149c.192.203.59.511.356 1.066-.235.555-.58 1.26-.196 1.977.384.716 1.042 1.194 1.464 1.115.421-.08 1.412-.598 1.776-.834.364-.237 1.518-1.19 1.518-1.554 0-.365-1.193-1.02-1.413-1.168-.22-.15-1.226-.725-1.247-.95-.02-.227-.012-.293.284-.851.297-.559.831-1.304.742-1.8-.089-.495-.95-.753-1.565-.986-.615-.232-1.799-.671-1.947-.74-.148-.068-.11-.133.339-.175.448-.043 1.719-.212 2.292-.052.573.16 1.552.403 1.632.532.079.13.149.134.067.579-.081.445-.5 2.581-.541 2.96-.04.38-.12.63.288.724.409.094 1.097.256 1.333.256s.924-.162 1.333-.256c.408-.093.329-.344.288-.723-.04-.38-.46-2.516-.541-2.961-.082-.445-.012-.45.067-.579.08-.129 1.059-.372 1.632-.532.573-.16 1.845.009 2.292.052.449.042.487.107.339.175-.148.069-1.332.508-1.947.74-.615.233-1.476.49-1.565.986-.09.496.445 1.241.742 1.8.297.558.304.624.284.85-.02.226-1.026.802-1.247.95-.22.15-1.413.804-1.413 1.169 0 .364 1.154 1.317 1.518 1.554.364.236 1.355.755 1.776.834.422.079 1.08-.4 1.464-1.115.384-.716.039-1.422-.195-1.977-.235-.555.163-.863.355-1.066l2.02-2.149c.341-.362.68-.546.68-1.243 0-.697-2.695-3.96-2.695-3.96s-2.274.436-2.58.436c-.307 0-.972-.256-1.585-.461-.613-.205-1.022-.206-1.022-.206z"/></svg>

Everruns integrates with [Brave Search](https://brave.com/search/api/) to give agents full web search capabilities. Agents can search the web and get relevant results including titles, URLs, and descriptions, perfect for research, fact-checking, and finding current information.

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
| `age` | How old the result is (e.g., "2 hours ago"), optional |

## When to Use Brave Search vs DuckDuckGo

| Use Case | Brave Search | DuckDuckGo |
|----------|--------------|------------|
| Full web search results | Best choice | Not available |
| Current news and articles | Best choice | Limited |
| Quick facts and definitions | Works but slower | Best choice |
| Wikipedia-style summaries | Not available | Best choice |
| Calculations and conversions | Not available | Direct answers |
| API key required | Yes (free tier available) | No |

Both capabilities can be enabled simultaneously, the agent will choose the right tool based on the task.

## Security

- API keys are encrypted at rest (AES-256-GCM envelope encryption)
- Keys are validated on connection (test query to Brave Search API)
- API key never appears in tool results or message history
- Rate limiting deferred to Brave Search API (returns 429 on limit)

## Status

**Experimental**: available in dev mode only. This capability may change in future releases.

## Links

- [Brave Search API](https://brave.com/search/api/)
- [Brave Search API Documentation](https://api.search.brave.com/app/#/documentation/web-search)
