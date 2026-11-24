---
title: Agent loops cache history and check completion evidence
description: GPT-6.2 reuses stable history with cheaper cache writes, agents can require completion evidence, and Gemini gains explicit conversation-storage control.
date: 2026-11-23
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [agent, code]
links:
  docs: [/capabilities/prompt-caching/, /capabilities/completion-requirements/, /capabilities/gemini-stateless/]
---

GPT-6.2 auto caching now covers growing stable history before volatile runtime
facts. [Prompt Caching](/capabilities/prompt-caching/) defaults to 30-minute
OpenAI writes; choose `12h` for reuse across longer pauses.

[Completion Requirements](/capabilities/completion-requirements/) checks actual
current-turn tool results or a final JSON schema before finishing. Unmet checks
continue within the existing budget. With default Framework features:

```rust
use everruns::{Agent, CapabilityRef};
use serde_json::json;

let builder = Agent::builder().capability(CapabilityRef::with_config(
    "completion_requirements",
    json!({"checks": [{"type": "tool_result", "tool": "verify", "pointer": "/passed", "equals": true}]}),
));
```

Add [Gemini Stateless](/capabilities/gemini-stateless/) to replay signed history
without storing an interaction chain. Completed generation metadata records the
sent streaming and conversation-storage settings when the request is captured.
