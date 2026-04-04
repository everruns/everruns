---
title: Budgets
description: Control session spending with budget limits — set caps in dollars, tokens, or custom credits, with soft pause thresholds and automatic enforcement
sidebar:
  order: 5
---

Budgets let you cap how much a session (or agent, user, or organization) can spend. When a session hits its budget, Everruns stops scheduling further LLM calls. This prevents runaway costs from long-running or misbehaving agents.

```
┌──────────────────────────────────────────────────────┐
│                    Budget                             │
│                                                      │
│  subject: session | agent | user | org               │
│  currency: usd | tokens | credits | custom           │
│  limit: hard cap                                     │
│  soft_limit: optional pause threshold                │
│                                                      │
│  ┌────────────────────────────────────────────────┐  │
│  │ ░░░░░░░░░░░░░░████████████████████████████████ │  │
│  │ ^              ^                              ^ │  │
│  │ $0          soft_limit                    limit │  │
│  │             (pauses)                    (stops) │  │
│  └────────────────────────────────────────────────┘  │
│                                                      │
│  Ledger: append-only log of every debit/credit       │
└──────────────────────────────────────────────────────┘
```

## How It Works

After every LLM generation, Everruns computes the cost and debits it from any active budgets for that session. If the balance reaches zero, the session stops.

1. **LLM call completes** — Everruns extracts token counts (input + output).
2. **Compute debit** — Converts tokens to the budget's currency:
   - `usd` — uses per-model pricing (cost per million tokens for input/output)
   - `tokens` — raw token count
   - `credits` — 1 credit = 1,000 tokens
   - Custom currencies fall back to token count
3. **Debit ledger** — Appends an immutable ledger entry and updates the balance.
4. **Evaluate rules** — Checks thresholds:
   - Balance at 20% of limit → warning event
   - Spending exceeds soft limit → session pauses
   - Balance reaches zero → session stops

Enforcement is **post-hoc**: the check runs after each LLM call, not before. This avoids blocking the hot path. The last generation may slightly overshoot the limit — this is expected and by design.

## Currencies

| Currency | Unit | How cost is calculated |
|----------|------|----------------------|
| `usd` | US dollars | Per-model pricing from model profiles (input/output rates per million tokens) |
| `tokens` | Raw tokens | Direct count of input + output tokens |
| `credits` | 1 credit = 1,000 tokens | Token count divided by 1,000 |
| Custom | Any string | Falls back to raw token count |

USD budgets use real per-model pricing. A $10 budget on GPT-4o will last much longer than $10 on Claude Opus, because the per-token cost differs.

## Soft Limits and Pausing

A **soft limit** pauses the session before hitting the hard stop. This is useful in interactive sessions where a human can decide to top up or stop.

1. Spending exceeds `soft_limit` → budget status becomes `paused`
2. The worker detects the pause between atoms → stops scheduling the next LLM call
3. Session transitions to `paused` state
4. User can resume by increasing the limit, topping up, or calling the resume endpoint

For headless sessions (no human watching), the hard limit fires at balance zero and terminates the turn.

## Stacked Budgets

Multiple budgets can apply to the same session. The **most restrictive** budget wins. This lets you combine different types of limits:

- A **$10 USD session budget** caps dollar cost
- A **2M token budget** caps total token usage regardless of model pricing

Budget stacking is currently enforced for **session** and **agent** scopes — a session's effective budgets include its own plus its agent's, and the most restrictive wins.

User- and organization-scoped budgets can be created via the API, but cascading enforcement for user/org scopes is not yet wired into the session budget check. This is planned for a future iteration.

## CLI Usage

Set a budget when creating a session:

```bash
# $10 USD budget (currency defaults to usd)
everruns sessions create --budget-limit 10

# Explicit currency
everruns sessions create --budget-limit usd:10

# With soft limit — pauses at $8, hard stop at $10
everruns sessions create --budget-limit usd:10 --budget-soft-limit usd:8

# Token budget
everruns sessions create --budget-limit tokens:2000000

# Stacked — both limits, whichever hits first
everruns sessions create --budget-limit usd:10 --budget-limit tokens:2000000
```

## MCP Usage

The `agent_run` MCP tool accepts budget parameters directly:

```json
{
  "name": "agent_run",
  "arguments": {
    "message": "Analyze this codebase",
    "agent_id": "agent_abc123",
    "budget_limit": 10.00,
    "budget_currency": "usd",
    "budget_soft_limit": 8.00
  }
}
```

Budget operations are also available as catalog commands via the `execute` tool:

```bash
# Create a budget for an existing session
create_budget --subject_type session --subject_id ses_xxx \
  --currency usd --limit 10 --soft_limit 8

# Check a session's budget status
check_session_budgets --session_id ses_xxx

# Top up an exhausted budget
top_up_budget --budget_id bdg_xxx --amount 5 --description "Extra allowance"

# List all budgets for a session
list_session_budgets --session_id ses_xxx
```

## API

### Budget CRUD

```
POST   /v1/budgets                  Create budget
GET    /v1/budgets                  List budgets (?subject_type=&subject_id=)
GET    /v1/budgets/{id}             Get budget with current balance
PATCH  /v1/budgets/{id}             Update limit / soft_limit / status
DELETE /v1/budgets/{id}             Soft-delete (sets status=disabled)
```

### Budget operations

```
POST   /v1/budgets/{id}/top-up     Add credits (negative ledger entry)
GET    /v1/budgets/{id}/ledger     Paginated ledger entries (?limit=&offset=)
GET    /v1/budgets/{id}/check      Check budget status
```

### Session shortcuts

```
GET    /v1/sessions/{id}/budgets       List budgets for this session
GET    /v1/sessions/{id}/budget-check  Check all budgets (session + hierarchy)
POST   /v1/sessions/{id}/resume        Resume paused budgets
```

### Create a session budget

```bash
curl -X POST https://your-instance/api/v1/budgets \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "subject_type": "session",
    "subject_id": "ses_01abc...",
    "currency": "usd",
    "limit": 10.00,
    "soft_limit": 8.00
  }'
```

### Top up an exhausted budget

```bash
curl -X POST https://your-instance/api/v1/budgets/{budget_id}/top-up \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "amount": 5.00,
    "description": "Emergency top-up"
  }'
```

If the budget was `paused` or `exhausted` and now has positive balance, it automatically reactivates.

## Events

Subscribe to budget events via SSE to react in real time:

| Event | When | Key data |
|-------|------|----------|
| `budget.warning` | Balance at 20% of limit | `budget_id`, `balance`, `limit`, `currency` |
| `budget.paused` | Spending exceeds soft limit | `budget_id`, `balance`, `soft_limit` |
| `budget.exhausted` | Balance reaches zero | `budget_id`, `balance`, `limit` |
| `budget.resumed` | User resumes after pause/top-up | `budget_id`, `balance`, `limit` |

## Agent Awareness

The `budgeting` capability is included in the **Generic harness** by default. Any session using the Generic harness automatically gets budget-aware behavior:

- The agent's system prompt includes a "Budget Awareness" section with guidelines for efficient output when budget is constrained
- The agent gets a `check_budget` tool to query remaining balance before expensive operations

When budget is running low, a budget-aware agent will prioritize completing current tasks efficiently rather than exploring new directions.

## Budget Lifecycle

```
                     create
                       │
                       ▼
              ┌──────────────┐
              │    active     │
              └──────┬───────┘
                     │
         ┌───────────┼───────────┐
         ▼           ▼           ▼
   soft_limit     balance≤0   disabled
   exceeded                   (deleted)
         │           │
         ▼           ▼
   ┌──────────┐ ┌───────────┐
   │  paused  │ │ exhausted │
   └────┬─────┘ └─────┬─────┘
        │              │
        └──────┬───────┘
          top-up / resume
               │
               ▼
        ┌──────────────┐
        │    active     │
        └──────────────┘
```
