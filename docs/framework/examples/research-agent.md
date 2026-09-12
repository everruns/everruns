---
title: Research Agent
description: Research a question with GLM, OpenRouter, and typed Brave web search.
---

The [Research Agent source](https://github.com/everruns/everruns/tree/main/examples/research-agent)
uses OpenRouter's `z-ai/glm-5.2` plus a typed Brave Search tool. Its
instructions require source URLs, an explicit distinction between facts and
inferences, and a caveat when the available research is incomplete.

![Research Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/research-agent/demo.gif)

This screencast replays a real run in readable pages, with waiting time removed.
[Read the complete displayed transcript](https://github.com/everruns/everruns/blob/main/examples/research-agent/demo.txt).
Tool-result previews are shortened; the agent receives the full tool response.

```bash
OPENROUTER_API_KEY=... BRAVE_SEARCH_API_KEY=... cargo run -p everruns-research-agent
```

The example caps each query at five results. Use a web-fetch integration when a
research task needs to inspect the contents of a particular source.

## The important part

```rust
// Brave Search arrives as a capability, not a hand-written tool: a capability
// packages one or more tools, their schemas, and their configuration behind a
// single value you attach to the agent.
let search = BraveSearch::from_env()?; // reads BRAVE_SEARCH_API_KEY

let agent = Agent::builder()
    .name("research-agent")
    // The citation discipline lives in the instructions: use the tool first,
    // cite what it returned, and label anything that goes beyond it.
    .instructions("You are a research agent. Use brave_web_search before \
        answering factual questions. Cite the returned source URLs, distinguish \
        facts from inferences, and say when the evidence is incomplete.")
    // The model is an OpenRouter slug; the provider is its own driver crate.
    .provider(everruns_openrouter::provider("openrouter", api_key))
    .model("z-ai/glm-5.2")
    .capability(search) // `.capability(..)` for packaged tools, `.tool(..)` for one function
    .build()?;
```

Capabilities and typed tools compose freely on the same builder, so an agent
can carry a search integration and a bespoke function side by side.
