---
title: "Yolop searches with the shared web_search tool"
description: "Yolop's default harness now has web_search: the model's own search first, then Brave or Parallel with a key, then free search."
date: 2026-10-18
kind: feature
significance: notable
products: [yolop]
surfaces: [cli]
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/features/tools.md#web"]
  prs: [everruns/yolop#863]
---

Yolop now ships the same `web_search` tool as Everruns agents. On OpenAI, ChatGPT plan, Anthropic, Gemini, and OpenRouter sessions it uses the model's own search. Otherwise it uses Brave Search or Parallel when `BRAVE_SEARCH_API_KEY` or `PARALLEL_API_KEY` is set, and free search when neither is.

It replaces `duckduckgo_instant_answer` and `free_web_search` in the default harness. The DuckDuckGo lookup stays available as an opt-in capability. Pin a provider or filter domains in `settings.toml`:

```toml
[[capabilities]]
ref = "web_search"
provider = "brave"        # or auto | native | free | parallel
blocked_domains = ["pinterest.com"]
```

See [Web tools](https://github.com/everruns/yolop/blob/main/docs/features/tools.md#web) and [#863](https://github.com/everruns/yolop/pull/863).
