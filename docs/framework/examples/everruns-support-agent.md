---
title: Everruns Support Agent
description: Troubleshoot Everruns Framework questions with Claude Opus and authoritative documentation links.
---

The [Everruns Support Agent source](https://github.com/everruns/everruns/tree/main/examples/everruns-support-agent)
shows a real Anthropic-backed troubleshooting agent. Its typed tool returns the
relevant Everruns documentation links before the model explains the smallest
safe next step.

![Everruns Support Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/everruns-support-agent/demo.gif)

This screencast replays a real run in readable pages, with waiting time removed.
[Read the complete displayed transcript](https://github.com/everruns/everruns/blob/main/examples/everruns-support-agent/demo.txt).
Tool-result previews are shortened; the agent receives the full tool response.

```bash
ANTHROPIC_API_KEY=... cargo run -p everruns-framework-support-agent
```

It uses `claude-opus-5`. Pass a different question after `--` to use the same
agent for another Framework support issue.

The documentation tool reads Markdown content from the official repository and
returns source links alongside that evidence.

## The important part

```rust
// A tool is ordinary async Rust, so it can reach the network. This one maps a
// topic to a documentation page and returns the page with its source link, so
// the model answers from fetched evidence rather than memory.
#[everruns::tool]
/// Read authoritative Framework documentation for a support topic.
async fn search_docs(topic: String) -> Result<String, String> {
    let topic = topic.to_lowercase();
    let page = if topic.contains("custom") {
        "custom-providers"
    } else if topic.contains("provider") || topic.contains("model") {
        "models-and-providers"
    } else {
        "examples"
    };
    // ... fetch https://raw.githubusercontent.com/.../docs/framework/{page}.md,
    // strip the frontmatter, and cap the body at 16 000 characters so one tool
    // result cannot swallow the context window.
    Ok(format!("Source: https://docs.everruns.com/framework/{page}/\n{content}"))
}

let agent = Agent::builder()
    .name("everruns-support-agent")
    // Instructions that separate evidence from hypothesis are what make the
    // fetched documentation useful rather than decorative.
    .instructions("You support Everruns Framework users. Use search_docs before \
        answering. Separate evidence from hypotheses and give the smallest safe \
        next step with relevant documentation links.")
    .provider(everruns_anthropic::provider("anthropic", api_key))
    .model("claude-opus-5")
    .tool(search_docs())
    .build()?;
```
