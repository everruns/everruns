---
title: Budgeting
description: Expose active budgets to the agent so it can check the remaining balance and adjust its own spending.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `budgeting` |
| **Category** | Cost Control |
| **Features** | `budgeting` |
| **Included in** | Worker harness |
| **Dependencies** | None |

Makes an agent aware of its budget constraints. The agent receives budget information in its system prompt and can proactively check remaining balance before expensive operations.

## Tools

### `check_budget`

Query the budget status for the current session.

> The tool reports whether budgets are configured. For balances, limits, and the status of each budget, use `GET /v1/sessions/{id}/budget-check`.

| Parameter | Type | Required | Description |
|---|---|---|---|
| *(none)* | | | No parameters required |

## Behavior

When the `budgeting` capability is enabled:

1. **System prompt injection**: The agent's system prompt includes a "Budget Awareness" section with the current budget status and guidelines for efficient output.

2. **Self-regulation**: When budget is running low, the agent prioritizes completing current tasks efficiently rather than exploring new directions or generating verbose output.

3. **Proactive checking**: The agent can call `check_budget` before starting expensive operations (large code generation, multi-step tool chains) to decide whether to proceed or ask the user.

## Related

- [Budgets](/advanced/budgets/), full budgeting system documentation (limits, currencies, API, CLI)
