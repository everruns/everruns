---
title: Base Harness
description: The root of the built-in harness tree, carrying the system essentials every agent needs to run reliably.
---

The **Base** harness is the root of the [built-in harness tree](/features/harnesses/#built-in-harnesses).
It carries only what every agent needs to run reliably, and adds no workspace, shell or chat tools.

## When to Use

- Custom tool composition on a reliable foundation
- Testing individual capabilities without a workspace or delegation
- Building your own harness family with Base as the parent

## Configuration

| Property | Value |
|----------|-------|
| **Name** | `base` |
| **Parent** | None |
| **Capabilities** | Context compaction, error disclosure, tool-call repair, loop detection, parallel tool calls, soft approval |
| **System Prompt** | "You are a helpful assistant." |
| **Default Model** | None (inherits from agent or organization) |

Soft approval is guidance, not a permission wall. It adds the approval tools and costs nothing on
safe work; turn it off per agent with `{"mode": "off"}`.

## Usage

Assign the Base harness when creating an agent or session:

```bash
curl -X POST http://localhost:9300/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Minimal Agent",
    "harness_name": "base",
    "capabilities": ["web_fetch"]
  }'
```

The agent's own capabilities are added on top. In this example `web_fetch` is the only domain tool.

```rust
let harness = everruns::Harness::base();
```

For a harness with no capabilities at all, build one with `Harness::builder` in the Framework or
create a custom harness without a parent on the Platform.

## See Also

- [Conversation Harness](/built-ins/harnesses/conversation/), the default for simple dialogue
- [Worker Harness](/built-ins/harnesses/worker/), the worker kit without compute
- [Harnesses feature guide](/features/harnesses/), the tree and how to choose
