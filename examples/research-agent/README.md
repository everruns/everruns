# Research Agent

A self-contained Framework research agent. It uses OpenRouter's
`z-ai/glm-5.2` with a typed Brave Search tool, then cites source URLs and
separates facts from inferences.

## Run

![Research Agent terminal demo](demo.gif)

```bash
OPENROUTER_API_KEY=... BRAVE_SEARCH_API_KEY=... cargo run -p everruns-research-agent
cargo test -p everruns-research-agent
```

The test only validates agent construction; `cargo run` makes the real provider
and web-search calls.
