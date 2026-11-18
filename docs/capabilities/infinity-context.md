---
title: Infinity Context
description: Trim live prompt history and query older messages on demand, so conversation length is not bounded by the context window.
sidebar:
  order: 85
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `infinity_context` |
| **Category** | Optimization |
| **Features** | None |
| **Dependencies** | None |

> **Deprecated configuration entry:** use [`compaction`](/advanced/compaction/) for managed long-session context. It includes permitted history retrieval and coordinates portable and native checkpoints. Existing `infinity_context` configurations remain supported. Used alone, this entry does not enable paid summaries.

Limits the live prompt to recent conversation history while keeping older messages accessible through `query_history`.

This is useful for long-running sessions where the agent should stay responsive without losing access to earlier decisions, identifiers, or requirements.

## Tools

| Tool | Purpose |
|---|---|
| `query_history` | Search or retrieve earlier messages from the current session |

## How It Works

1. A message filter caps the number of recent messages sent to the model.
2. If older messages are excluded, the model sees a system notice telling it to use `query_history`.
3. The `query_history` tool can keyword-search history or fetch a specific absolute message range.

## Configuration

Default configuration:

```json
{
  "capabilities": ["infinity_context"]
}
```

Custom budget:

```json
{
  "capabilities": [
    {
      "ref": "infinity_context",
      "config": {
        "context_budget_tokens": 80000,
        "min_recent_messages": 12
      }
    }
  ]
}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `context_budget_tokens` | integer | `100000` | Approximate token budget reserved for message history |
| `min_recent_messages` | integer | `10` | Minimum recent messages to keep even when the budget is tight |

## Limitations

- Search is keyword-based, not semantic; database-backed hosts use indexed full-text search
- Search and range retrieval use bounded storage queries over retained session history, including messages still visible in the prompt
- Budgeting uses token estimates, not guaranteed provider token counts

## See Also

- [Context Compaction](/advanced/compaction/), The replacement capability for managed long-session context
- [Capabilities Overview](/capabilities/)
- [Harnesses](/features/harnesses/)
