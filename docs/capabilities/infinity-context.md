---
title: Infinity Context
description: Keep long conversations manageable by trimming live prompt history and querying older messages on demand. Unlimited conversation length for complex agent tasks.
sidebar:
  order: 85
---

| | |
|---|---|
| **ID** | `infinity_context` |
| **Category** | Optimization |
| **Features** | None |
| **Dependencies** | None |

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

## Use Cases

- Long debugging or implementation sessions
- Agents that need to preserve early requirements or credentials mentioned far back in the thread
- Platform chat sessions that accumulate substantial history over time

## Limitations

- Search is keyword-based, not semantic
- The tool reads full session history; it does not currently restrict itself to only the trimmed portion
- Budgeting uses a heuristic message-count estimate, not model-specific tokenization

## See Also

- [Capabilities Overview](/capabilities/)
- [Harnesses](/features/harnesses/)
