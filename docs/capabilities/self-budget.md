---
title: Self-Budget
description: Prompt-only guidance for agents to reason about a user-requested indicative budget using session usage data. Distinct from the platform-enforced `budgeting` capability.
---

| | |
|---|---|
| **ID** | `self_budget` |
| **Category** | System |
| **Features** | *(none)* |
| **Tools** | *(none)* |
| **Included in** | Generic harness (default) |
| **Dependencies** | None |

Teaches the agent how to self-manage an **indicative** budget that the user mentions in conversation — for example, "you have $7" or "keep this under 20k tokens". The capability contributes prompt text only; it adds no tools and performs no enforcement.

For platform-enforced budgets (authoritative limits that pause or stop sessions automatically), use the separate [`budgeting`](/capabilities/budgeting/) capability.

## How It Works

`self_budget` is prompt-only. When the capability is enabled the agent's system prompt gets a "Self-Managed Budget" section that explains:

- The self-budget is an **agent-managed soft target**, not a hard limit.
- Session usage metadata (exposed via `get_session_info`) is the source of truth for current spend.
- The agent decides when to start tracking, when to re-check, and when to stop.
- As the target tightens, the agent should adapt — shorter outputs, fewer retries, narrower exploration, fewer redundant tool calls.
- The agent avoids claiming exact cost certainty when only token counts or partial pricing are available.
- The agent distinguishes between platform-enforced budgets and user-requested indicative budgets when reporting progress.

There is no `self_budget` tool. Usage data comes from `get_session_info`, which is provided by the [`session`](/capabilities/session/) capability (bundled by default in the Generic harness).

## Self-Budget vs Budgeting

| Aspect | `self_budget` | `budgeting` |
|---|---|---|
| What it is | Agent-managed soft target | Platform-enforced limit |
| Tools | None | `check_budget` |
| Enforcement | None (prompt guidance only) | Session is paused/stopped automatically |
| Data source | `get_session_info` cumulative usage | Budgets table / ledger |
| Use case | User says "you have $7" in chat | Org/session has a configured budget in the platform |

The two capabilities are non-conflicting and can run together. The Generic harness includes both.

## Use Cases

- **User-set budget in prompt** — "Spend at most $3 on this investigation." The agent tracks usage via `get_session_info`, adapts scope, and reports progress.
- **Coaching the agent to be frugal** — Even without an exact dollar target, the prompt guidance nudges the agent toward shorter outputs and fewer redundant tool calls.
- **Budget-aware estimation** — The agent qualifies cost estimates ("roughly", "on the order of") rather than over-stating certainty.

## Example

```
User: You have about $2 of compute for this task. Analyze the three largest
      dependencies in package.json and tell me which one to remove.

Agent: [calls get_session_info — current usage $0.14]
Agent: Got it. I'll focus on the three largest dependencies and keep this tight.
       [... scoped analysis ...]
Agent: [calls get_session_info — current usage $1.62]
Agent: Roughly $1.60 spent so far. Based on size and use count, `moment` is the
       best removal candidate — replacement path is straightforward. I'll stop
       here to stay under your $2 target.
```

## Related

- [Budgeting](/capabilities/budgeting/) — platform-enforced budgets with the `check_budget` tool
- [Session](/capabilities/session/) — provides `get_session_info`, the usage data source
- [Budgets](/advanced/budgets/) — full budgeting system documentation
