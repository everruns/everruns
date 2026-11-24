---
title: Completion Requirements
description: Check actual tool evidence or a final JSON schema before an agent finishes, using the same bounded continuation on Framework and Platform.
appliesTo: [framework, platform, cloud]
---

The `completion_requirements` capability adds deterministic evidence checks to
finishing turns. It adds no tools or extra model call. Configure it through an agent or harness API request, agent package, or
Framework code:

```json
{
  "ref": "completion_requirements",
  "config": {
    "checks": [
      {"type": "tool_result", "tool": "verify", "pointer": "/passed", "equals": true}
    ]
  }
}
```

A `tool_result` check requires the latest matching call in the current turn to
have a successful result whose JSON pointer equals the configured value. Set
`arguments` to require an exact argument object. An older success cannot hide a
new failure, an unanswered call, or missing current-turn evidence. Choose a
read-back tool to verify persisted state or an artifact; an assistant's prose
is never evidence for this check.

A `final_json` check takes a JSON `schema` and validates the final answer as
JSON. Markdown fences are not JSON. Configuration allows 1–32 checks and up to
64 KiB; invalid configuration is rejected.

Unmet requirements continue through the existing completion-check budget and
record `turn.completion_checked`. They apply even with the model-based
`completion_check` set to `off`. When that review is enabled, requirements run
first and the model review follows only after they pass. Questions requiring
human input, refusals, failures, and pending background work keep their existing
stop behavior. Budget exhaustion stops continuation and records an unmet
verdict; it does not establish that the requirements passed. Transport success
alone is not proof of task completion.

With the default `everruns` features:

```rust
use everruns::{Agent, CapabilityRef};
use serde_json::json;

let builder = Agent::builder().capability(CapabilityRef::with_config(
    "completion_requirements",
    json!({"checks": [{"type": "final_json", "schema": {"type": "object", "required": ["answer"]}}]}),
));
```
