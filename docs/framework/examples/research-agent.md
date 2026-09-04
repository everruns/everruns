---
title: Research Agent
description: Research a question with GLM, OpenRouter, and typed Brave web search.
---

The [Research Agent source](https://github.com/everruns/everruns/tree/main/examples/research-agent)
uses OpenRouter's `z-ai/glm-5.2` plus a typed Brave Search tool. Its
instructions require source URLs, an explicit distinction between facts and
inferences, and a caveat when the available research is incomplete.

![Research Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/research-agent/demo.gif)

```bash
OPENROUTER_API_KEY=... BRAVE_SEARCH_API_KEY=... cargo run -p everruns-research-agent
```

The example caps each query at five results. Use a web-fetch integration when a
research task needs to inspect the contents of a particular source.
