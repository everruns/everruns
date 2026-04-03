---
title: Budgeting
description: Budget-aware agent behavior. Agents receive information about active budgets, can check remaining balance, and prioritize efficiency when budget is constrained.
---

| | |
|---|---|
| **ID** | `budgeting` |
| **Category** | Cost Control |
| **Features** | `budgeting` |
| **Included in** | Generic harness (default) |
| **Dependencies** | None |

Makes an agent aware of its budget constraints. The agent receives budget information in its system prompt and can proactively check remaining balance before expensive operations.

## Tools

### `check_budget`

Query the budget status for the current session.

> **Note:** The current implementation returns a placeholder response indicating whether budgets are configured. Full budget data (balance, limit, status per budget) requires worker-side tool interception, which is planned for a future iteration. In the meantime, use the REST API (`GET /v1/sessions/{id}/budget-check`) for detailed budget status.

| Parameter | Type | Required | Description |
|---|---|---|---|
| *(none)* | | | No parameters required |

## Behavior

When the `budgeting` capability is enabled:

1. **System prompt injection** — The agent's system prompt includes a "Budget Awareness" section with the current budget status and guidelines for efficient output.

2. **Self-regulation** — When budget is running low, the agent prioritizes completing current tasks efficiently rather than exploring new directions or generating verbose output.

3. **Proactive checking** — The agent can call `check_budget` before starting expensive operations (large code generation, multi-step tool chains) to decide whether to proceed or ask the user.

## Use Cases

- **Cost-conscious sessions** — Agent adapts its behavior as budget depletes (shorter responses, fewer tool calls)
- **Pre-flight checks** — Agent checks budget before launching a long multi-step workflow
- **User communication** — Agent informs the user when budget is nearly exhausted so they can top up or reprioritize

## Example

```
User: Analyze all 200 files in this repository

Agent: [calls check_budget]
Agent: I have $2.30 remaining on a $10 budget. Analyzing all 200 files
       would likely exceed that. I'll focus on the 15 most critical files
       (entry points, config, core modules) and provide a summary. Want me
       to proceed, or would you like to increase the budget first?
```

## Related

- [Budgets](/advanced/budgets/) — full budgeting system documentation (limits, currencies, API, CLI)
