---
title: "Claude message threads send only new messages"
description: "Opt into short-lived Claude message threads to reduce repeated history uploads, with full local transcripts and automatic replay when state expires."
date: 2026-11-02
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [api, code]
links:
  docs: [/providers/anthropic/]
---

The `claude_message_threads` capability lets compatible Claude conversations send
only new messages after a completed response. Everruns retains the full
transcript and replays it when Anthropic cannot continue a thread.

In the Framework, enable the `anthropic` feature and add the capability:

```rust
use everruns::{Agent, providers::anthropic::Anthropic};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _agent = Agent::builder()
        .provider(Anthropic::from_env()?)
        .model("claude-sonnet-5-5")
        .capability("claude_message_threads")
        .build()?;
    Ok(())
}
```

Threads are optional and separate from prompt caching. They do not change token
pricing. Anthropic temporarily retains thread content; this beta is not eligible
for zero data retention or HIPAA. See [Anthropic](/providers/anthropic/) for
retention, recovery, and configuration details.
