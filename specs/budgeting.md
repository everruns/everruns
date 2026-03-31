# Budgeting Specification

## Abstract

Extensible budgeting system for controlling resource consumption across sessions, agents, users, and organizations. Supports multiple currencies (USD cost, token counts, custom credits), pluggable metering, pluggable policy rules, and soft enforcement (pause/warn before hard-stop).

Key use cases:
- **Volunteer budget**: User sets a $10 budget when creating a session. Session stops at $10.
- **Admin budget**: Admin sets a $50/month budget on a user. All sessions for that user stop when the user hits $50.
- **Stacked budgets**: Session has a $10 volunteer budget AND the user has a $50 admin budget. The session stops at whichever limit is hit first (most restrictive wins).

## Implementation

See source files for full definitions:
- Core types: `crates/core/src/budget.rs`
- Typed IDs: `crates/core/src/typed_id.rs` (`BudgetId`, `LedgerEntryId`)
- Events: `crates/core/src/events.rs` (budget event constants and `BudgetEventData`)
- Storage: `crates/server/src/storage/repositories/budgets.rs`, `crates/server/src/storage/memory/budgets.rs`
- Service: `crates/server/src/services/budget.rs`
- API: `crates/server/src/api/budgets.rs`
- Capability: `crates/core/src/capabilities/budgeting.rs`
- Migration: `crates/server/migrations/011_budgeting.sql`

## Concepts

```
┌─────────────────────────────────────────────────────┐
│                    Budget                             │
│  subject: (session | agent | user | org)             │
│  currency: (usd | tokens | credits | custom)        │
│  limit: f64                                          │
│  soft_limit: Option<f64>  (pause/warn threshold)     │
│  period: Option<Period>   (rolling/calendar)         │
│  balance: denormalized from ledger                   │
└──────────────┬──────────────────────────────────────┘
               │ 1:N
┌──────────────▼──────────────────────────────────────┐
│                 Ledger Entry                          │
│  budget_id, amount, meter_source, ref_id, timestamp  │
│  (append-only — no UPDATE/DELETE)                    │
└─────────────────────────────────────────────────────┘

┌─────────────────────────┐    ┌──────────────────────┐
│  Meter (pluggable)      │    │  Rule  (pluggable)   │
│  Observes events,       │    │  Evaluates budgets   │
│  produces debits         │    │  after each debit    │
│  ─────────────────────  │    │  ─────────────────── │
│  LlmTokenMeter          │    │  HardLimitStopRule   │
│  (future: ToolCallMeter,│    │  SoftLimitPauseRule  │
│   DataProcessedMeter)   │    │  WarnRule (80%)      │
└─────────────────────────┘    └──────────────────────┘
```

### Budget

A spending cap bound to a **subject** (who) in a **currency** (what unit). Multiple budgets per subject allowed; the most restrictive one wins.

**Subject types**: `session`, `agent`, `user`, `org` — budgets cascade through the hierarchy. A session's effective budgets include its own + its agent's + its user's + its org's.

**Currencies**: Strings (not enum) — new currencies added without migrations. Built-in: `usd` (via LlmModelProfile cost lookup), `tokens` (raw count), `credits` (1 credit = 1000 tokens).

**Balance**: `limit - SUM(debits) + SUM(credits)`. Denormalized on `budgets.balance`, updated atomically with each ledger insert (Postgres: `SELECT ... FOR UPDATE` in transaction; in-memory: lock + update).

**Status lifecycle**: `active` → `paused` (soft limit reached) → `exhausted` (balance ≤ 0) → `disabled` (soft-deleted). Budget can be resumed by top-up or limit increase.

### Ledger Entry

Immutable, append-only record of consumption or credit. Positive = debit, negative = credit (top-up/refund). Protected by `prevent_event_mutation()` trigger in Postgres.

## Evaluation Pipeline

On every `llm.generation` event:

```
llm.generation event arrives
  │
  ▼
BudgetService.on_event() (EventListener)
  │
  ▼
Extract tokens: input_tokens + output_tokens
  │
  ▼
Look up session → find active budgets in hierarchy
  (session → agent → user → org)
  │
  ▼ (for each matching budget)
compute_debit(currency, tokens, model, provider)
  │  "tokens" → raw count
  │  "usd"    → tokens * LlmModelProfile cost-per-token
  │  "credits" → tokens / 1000
  │
  ▼
INSERT ledger entry + UPDATE budget.balance (atomic)
  │
  ▼
evaluate_rules(budget)
  │  balance ≤ 0         → Stop  (mark budget "exhausted")
  │  spent > soft_limit  → Pause (mark budget "paused")
  │  balance ≤ 20% limit → Warn  (log warning)
  │  otherwise           → Continue
  │
  ▼
Execute action (set budget status)
```

**Post-hoc enforcement**: Budget checks happen after each metered event, not before. This avoids blocking the LLM hot path. Minor overshoot on the last generation is acceptable and expected.

**Worker integration**: The worker checks `BudgetCheckResult` between atoms via gRPC. When a budget is `paused` or `exhausted`, the turn loop stops scheduling the next atom.

## Soft Enforcement: Pause

Pause is a **soft prevention** mechanism for interactive sessions:

1. Budget spending exceeds `soft_limit` threshold
2. Budget status set to `paused`
3. Worker detects paused status between atoms → stops scheduling next atom
4. Session status transitions to `paused` (new status in session lifecycle)
5. User can: (a) increase limit, (b) top up, (c) resume via API

**Headless/API flow**: For headless sessions (no human watching), the `HardLimitStopRule` fires when balance ≤ 0 and terminates the turn. Soft limit pause is also respected — the API caller should poll `GET /v1/sessions/{id}/budget-check` or listen to `budget.paused` SSE events.

```
Session states: started → active → idle
                          ↓        ↑
                        paused ────┘ (resume)
```

## Events

| Event Type | When | Data (BudgetEventData) |
|-----------|------|------|
| `budget.warning` | Balance ≤ 20% of limit | `budget_id, balance, limit, currency, message` |
| `budget.paused` | Spending exceeds soft_limit | `budget_id, balance, limit, currency, soft_limit, message` |
| `budget.exhausted` | Balance ≤ 0 | `budget_id, balance, limit, currency, message` |
| `budget.resumed` | User resumes after pause/top-up | `budget_id, balance, limit, currency` |

All four events use the same `BudgetEventData` struct. See `crates/core/src/events.rs`.

## API

### Budget CRUD

```
POST   /v1/budgets                        Create budget
GET    /v1/budgets                        List budgets (?subject_type=&subject_id=)
GET    /v1/budgets/{id}                   Get budget with current balance
PATCH  /v1/budgets/{id}                   Update limit / soft_limit / status
DELETE /v1/budgets/{id}                   Soft-delete (sets status=disabled)

POST   /v1/budgets/{id}/top-up           Add credits (negative ledger entry)
GET    /v1/budgets/{id}/ledger           Paginated ledger entries (?limit=&offset=)
GET    /v1/budgets/{id}/check            Check budget status
```

### Session shortcuts

```
GET    /v1/sessions/{id}/budgets          List budgets for this session
GET    /v1/sessions/{id}/budget-check     Check all budgets (session + hierarchy)
POST   /v1/sessions/{id}/resume           Resume paused budgets for this session
```

### Examples

**Give a session $10 with $8 soft limit:**
```json
POST /v1/budgets
{
  "subject_type": "session",
  "subject_id": "session_01abc...",
  "currency": "usd",
  "limit": 10.00,
  "soft_limit": 8.00
}
```

**Give a user 100 credits/month (admin-defined):**
```json
POST /v1/budgets
{
  "subject_type": "user",
  "subject_id": "usr_xyz789",
  "currency": "credits",
  "limit": 100,
  "period": { "type": "calendar", "unit": "month" }
}
```

**Top up an exhausted budget:**
```json
POST /v1/budgets/{id}/top-up
{
  "amount": 5.00,
  "description": "Emergency top-up"
}
```
This adds $5 to the balance. If the budget was `paused` or `exhausted` and now has positive balance, it auto-reactivates to `active`.

## Agent Awareness (Capability)

The `budgeting` capability adds budget self-regulation to agents:

1. **System prompt**: "Budget Awareness" section with guidelines on efficient output when budget is constrained.
2. **`check_budget` tool**: Agent can call this to check remaining balance before expensive operations.

Enable on an agent by adding `budgeting` to its capabilities list.

## Design Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Pre-check vs post-check | Post-check (after event) | Avoids blocking hot path; minor overshoot acceptable |
| Balance storage | Denormalized on `budgets` + append-only ledger | Fast reads; ledger is source of truth for reconciliation |
| Currency as enum vs string | String | Extensible without migrations |
| Budget scope | Per-subject with hierarchy | Flexible: session, agent, user, or org level |
| Pause mechanism | Session status + turn loop yield | Reuses existing session lifecycle; non-destructive |
| Multiple budgets per subject | Allowed, most restrictive wins | User may want both cost and token limits |
| Period handling | JSONB with type discriminator | Rolling windows and calendar periods have different semantics |
| Headless enforcement | Hard stop at balance ≤ 0 | No human to interact with pause; hard stop is safer |
| Cost lookup | `LlmModelProfile` from `llm_model_profiles.rs` | Already has per-model pricing from models.dev |

## Future Work

- **ToolCallMeter**, **DataProcessedMeter** — additional meters
- **Rolling/calendar period support** — balance resets on period boundary
- **Valkey-cached balance** — for high-throughput without DB round-trip per check
- **Budget analytics dashboard** — spend by model, by agent, over time
- **Custom meter registration API** — for external integrations
