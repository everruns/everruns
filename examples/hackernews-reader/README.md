# HackerNews Reader Agent

An autonomous HackerNews browsing agent built on Everruns — equivalent to the
[GitHub Copilot SDK HackerNews demo](https://x.com/i/status/2019435734365794710)
but using Everruns capabilities instead of hand-coded tool functions.

## How It Compares

| Copilot SDK approach | Everruns approach |
|---------------------|-------------------|
| Define 5 tool functions (getTopStories, getItem, getComments, getUser, etc.) | Zero custom code — `web_fetch` capability calls the HN API directly |
| Write orchestration logic | Everruns agent loop handles orchestration + tool execution |
| Build a UI | Everruns UI included (or use the API) |
| In-memory state | Durable execution with automatic retries |

The agent definition is a single markdown file: [`hackernews-reader.md`](hackernews-reader.md)

## Quick Start

```bash
# 1. Start Everruns
just start-dev

# 2. Install deps
pip install requests pyyaml

# 3. Run
python examples/hackernews-reader/run.py

# Or with a custom prompt
python examples/hackernews-reader/run.py "Show me today's top Show HN posts"
```

## What the Agent Can Do

- Fetch top/new/best/ask/show stories from HackerNews
- Read and summarize comment threads (2-3 levels deep)
- Look up author profiles (karma, account age, notable posts)
- Save research findings to the session filesystem
- Knows the current time for relative timestamps

## Capabilities Used

- **web_fetch** — HTTP requests to the HN Firebase API
- **current_time** — relative time display ("posted 2 hours ago")
- **session_file_system** — persist notes and summaries

## Architecture

```
User prompt
  |
  v
Everruns API (POST /v1/sessions/{id}/messages)
  |
  v
Agent Loop (durable workflow)
  |
  +---> ReasonAtom (LLM decides what to do)
  |       |
  |       v
  +---> ActAtom (execute web_fetch tool calls)
  |       |
  |       v
  +---> Loop back until done
  |
  v
Agent response (streamed via events)
```
