---
title: Enforce a budget
description: Cap LLM spend per session or agent with USD, token, or credit-denominated budgets, soft pause thresholds, and stacked limits.
---

Budgets cap how much a session can spend on LLM calls. After every generation, Everruns debits the cost from any active budgets; when the balance reaches zero, the session stops. This guide creates and applies a budget.

For the design and enforcement semantics, see [Budgets](/advanced/budgets/).

## Create a USD budget for a session

```bash
curl -X POST http://localhost:9300/api/v1/budgets \
  -H "Content-Type: application/json" \
  -d '{
    "scope": "session",
    "scope_id": "session_...",
    "currency": "usd",
    "limit": 10.00,
    "soft_limit": 8.00
  }'
```

When session spend exceeds `soft_limit`, the session pauses (status becomes `paused`) so a human can decide to top up or stop. When it hits `limit`, the session terminates.

## Token-denominated budget

```bash
curl -X POST http://localhost:9300/api/v1/budgets \
  -H "Content-Type: application/json" \
  -d '{
    "scope": "agent",
    "scope_id": "agent_...",
    "currency": "tokens",
    "limit": 2000000
  }'
```

Token budgets are model-agnostic, they cap raw token usage regardless of which provider the session uses.

## Currencies at a glance

| Currency | Unit | Cost basis |
|---|---|---|
| `usd` | US dollars | Per-model pricing (input/output cost per million tokens) |
| `tokens` | Raw tokens | Direct count of input + output tokens |
| `credits` | 1 credit = 1,000 tokens | Token count ÷ 1,000 |
| Custom | Any string | Falls back to raw token count |

USD budgets reflect real costs: $10 lasts much longer on GPT-4o than on Claude Opus.

## Stack budgets for layered limits

You can apply multiple budgets to a session at once. The **most restrictive** wins. A common pattern:

- `$10 USD` session budget, caps dollar cost.
- `2,000,000 tokens` agent budget, caps total tokens regardless of pricing.

Both apply; whichever runs out first stops the session.

## Listen for budget events

Budget thresholds emit events you can subscribe to:

```python
async for event in client.events.stream(session.id):
    if event.type == "budget.warning":
        print(f"Budget warning: {event.data}")
    elif event.type == "budget.paused":
        print(f"Session paused at soft limit")
    elif event.type == "budget.exhausted":
        print(f"Session stopped — budget exhausted")
```

A warning fires at 20% remaining; pause fires when crossing the soft limit; exhaustion fires at zero balance.

## Resume a paused session

After a `budget.paused` event, you can:

- Increase `limit` to give the session more headroom:

  ```bash
  curl -X PATCH http://localhost:9300/api/v1/budgets/$BUDGET_ID \
    -H "Content-Type: application/json" \
    -d '{ "limit": 20.00, "soft_limit": 16.00 }'
  ```

- Or call the resume endpoint to continue against the existing limit (the next LLM call may push the budget over).

## A note on enforcement

Budget checks run **after** each LLM call, not before, to avoid latency on the hot path. The last generation can slightly overshoot the limit, this is expected and by design. Treat budgets as cost caps, not hard cutoffs measured in single tokens.

## See also

- [Budgets](/advanced/budgets/), full design, ledger semantics, custom currencies.
- [Self-Budget capability](/capabilities/self-budget/), let agents inspect their own budget at runtime.
