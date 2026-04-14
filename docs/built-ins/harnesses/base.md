---
title: Base Harness
description: An empty harness with no capabilities for full control over session configuration.
---

The **Base** harness is a blank-slate starting point with no bundled capabilities.

## When to Use

- Full control over which tools and behaviors are available
- Testing individual capabilities in isolation
- Minimal-overhead sessions where no default tools are needed

## Configuration

| Property | Value |
|----------|-------|
| **Type** | `base` |
| **Capabilities** | None |
| **System Prompt** | "You are a helpful assistant." |
| **Default Model** | None (inherits from agent or organization) |

## Usage

Assign the Base harness when creating an agent or session:

```bash
curl -X POST http://localhost:9300/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Minimal Agent",
    "harness_id": "<base-harness-id>",
    "capabilities": ["web_fetch"]
  }'
```

The agent's own capabilities are added on top of the empty harness. In this example, only `web_fetch` would be available.

## See Also

- [Generic Harness](/built-ins/harnesses/generic/) — recommended default with core capabilities
- [Harnesses feature guide](/features/harnesses/) — harness selection and API management
