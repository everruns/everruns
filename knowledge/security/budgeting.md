---
type: Specification
title: "Budgeting Specification"
description: "Extensible budgeting system."
tags:
  - everruns
  - security
---
# Budgeting Specification

## Abstract

Extensible budgeting system for controlling resource consumption across sessions, agents, users, and organizations. Supports multiple currencies (USD cost, token counts, custom credits), pluggable metering, pluggable policy rules, and soft enforcement (pause/warn before hard-stop).

Budget enforcement remains owned by this domain. Analytical budget reports are
derived asynchronously from `usage_ledger` and related budget metadata; see
[knowledge/evaluation/reporting.md](../evaluation/reporting.md).

Key use cases:
- **Volunteer budget**: User sets a $10 budget when creating a session. Session stops at $10.
- **Admin budget**: Admin sets a $50/month budget on a user. All sessions for that user stop when the user hits $50.
- **Stacked budgets**: Session has a $10 volunteer budget AND the user has a $50 admin budget. The session stops at whichever limit is hit first (most restrictive wins).

## Implementation

See source files for full definitions:
- Core types: `crates/core/src/budget.rs`
- Typed IDs: `crates/provider/src/typed_id.rs` (`BudgetId`, `LedgerEntryId`)
- Events: `crates/core/src/events.rs` (budget event constants and `BudgetEventData`)
- Storage: `crates/server/src/storage/repositories/budgets.rs`, `crates/server/src/storage/memory/budgets.rs`
- Service: `crates/server/src/domains/budgets/service.rs`
- API: `crates/server/src/api/budgets.rs`
- Capability: `crates/builtins/src/budgeting.rs`
- Migrations: `crates/server/migrations/010_v0.8.9.sql`, `crates/server/migrations/020_budget_journal_ledger.sql`

## Concepts

```
┌─────────────────────────────────────────────────────┐
│                Usage Journal                        │
│  kind: llm_generation | top_up | ...               │
│  scope: org/user/principal/session/agent/harness    │
│  source: event_id or synthetic source_id            │
│  measures: raw facts JSONB                          │
│  metadata: trace / model / provider / notes         │
└──────────────┬──────────────────────────────────────┘
               │ 1:N rated postings
┌──────────────▼──────────────────────────────────────┐
│                 Usage Ledger                        │
│  journal_id, budget_id?, currency, amount           │
│  meter_source, ref_id, rating_metadata, timestamp   │
│  (append-only — no UPDATE/DELETE)                   │
└──────────────┬──────────────────────────────────────┘
               │ projected into
┌──────────────▼──────────────────────────────────────┐
│                    Budget                           │
│  subject: (session | agent | user | org)            │
│  currency: (usd | tokens | credits | custom)        │
│  limit / soft_limit / period                        │
│  balance: denormalized current snapshot             │
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

**Subject types**: `session`, `app_channel`, `app`, `agent`, `user`, `org`, budgets cascade through the hierarchy from most specific (session) to most general (org). A session's effective budgets include all matching levels.

`app` and `app_channel` are **gated behind the experimental `app_budgets` feature flag** (`FEATURE_APP_BUDGETS`, auto-enabled in `DeploymentGrade::Dev`). Sessions opt into these levels via the standard tags emitted by the apps domain (`app:<app_id>`, `app_channel:<channel_id>`). The legacy `slack:app:<id>` tag is also recognised for backwards compatibility.

**Currencies**: Strings (not enum), new currencies added without migrations. Built-in: `usd` (via ModelProfile cost lookup), `tokens` (raw count), `credits` (1 credit = 1000 tokens).

**Balance**: `limit - SUM(debits) + SUM(credits)`. Denormalized on `budgets.balance`, updated atomically with each budget-scoped ledger insert (Postgres: transaction + `SELECT ... FOR UPDATE`; in-memory: lock + update).

**Status lifecycle**: `active` → `paused` (soft limit reached) → `exhausted` (balance ≤ 0) → `disabled` (soft-deleted). Budget can be resumed by top-up or limit increase.

### Usage Journal

Immutable raw activity fact. Current writers:
- `llm_generation` from `llm.generation` events
- synthetic adjustment rows for budget top-ups / manual compatibility writes

The journal stores scope (`org_id`, `user_id`, `principal_id`, `session_id`, `agent_id`, `harness_id`) plus raw `measures` and free-form `metadata`.

### Usage Ledger

Immutable, append-only rated posting derived from a journal row. Positive = debit, negative = credit (top-up/refund). Each ledger row links back to `journal_id`; budget-scoped postings also carry `budget_id`. Protected by append-only triggers in Postgres.

### Detachment is the one permitted write

Spend outlives whatever produced it. When a session, event, or other referenced
row is deleted, the journal and ledger rows stay and their reference column is
nulled by `ON DELETE SET NULL` — the amount, currency, org, and measures are
never touched. The append-only trigger permits exactly that shape of update and
rejects everything else, including a hand-written update that nulls a reference
while editing another column. Deleting a journal or ledger row is never allowed.

This is why deleting a session no longer fails: before migration 122 the trigger
rejected the cascade itself, so `DELETE /v1/sessions/{id}` returned 500 for any
session that had spent anything.

## Evaluation Pipeline

On every `llm.generation` event:

```
llm.generation event arrives
  │
  ▼
BudgetService.on_event() (EventListener)
  │
  ▼
INSERT usage_journal row
  kind=llm_generation
  measures={input_tokens, output_tokens, total_tokens, provider_cost_usd}
  │
  ▼
Look up session → find active budgets in hierarchy
  (root session → app_channel → app → agent → user → org)
  │
  ▼ (for each matching budget)
compute_debit(currency, tokens, cache tokens, model, provider, provider_cost_usd)
  │  "tokens" → raw count
  │  "usd"    → provider_cost_usd when the provider reports it
  │            (e.g. OpenRouter usage.cost), else the cache-aware
  │            ModelProfile estimate (cached reads billed at the
  │            cache_read rate, not the input rate), else raw count
  │  "credits" → tokens / 1000
  │
  ▼
INSERT usage_ledger row (journal_id -> budget_id)
  + UPDATE budget.balance (atomic)
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

For subagent delegation trees, the session budget layer resolves to the
tree's `root_session_id`: a child or grandchild checks and debits the root
session's budget pool, while usage journal and ledger rows still retain the
actual child session id for attribution.

**Detached peer spawns** (`spawn_agent(lifetime=detached)`) remain lifecycle-
independent (`parent_session_id = NULL`) but explicitly inherit the origin
session's org-validated `root_session_id` for budget attribution. Their LLM
generations therefore debit the origin root pool, and detached chains stop when
that pool is exhausted. The override is carried only on trusted worker/internal
session-creation paths and is stripped at the public HTTP boundary. Storage
canonicalizes the referenced session's root under the creating org, preventing
cross-org linkage. Ordinary user forks carry lineage only and remain independent
budget roots. Detached count caps (`max_active_detached_tasks` /
`max_total_detached_tasks`) remain an independent admission bound (TM-DOS-030).

**Worker integration**: The worker checks `BudgetCheckResult` between atoms via gRPC. When a budget is `paused` or `exhausted`, the turn loop stops scheduling the next atom. Current implementation resolves the full hierarchy (`root session`, `app_channel`, `app`, `agent`, `user`, `org`) from the session owner and org context before checking.

## Soft Enforcement: Pause

Pause is a **soft prevention** mechanism for interactive sessions:

1. Budget spending exceeds `soft_limit` threshold
2. Budget status set to `paused`
3. Worker detects paused status between atoms → stops scheduling next atom
4. Session status transitions to `paused` (new status in session lifecycle)
5. User can: (a) increase limit, (b) top up, (c) resume via API

**Headless/API flow**: For headless sessions (no human watching), the `HardLimitStopRule` fires when balance ≤ 0 and terminates the turn. Soft limit pause is also respected, the API caller should poll `GET /v1/sessions/{id}/budget-check` or listen to `budget.paused` SSE events.

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

## Self-Managed vs Platform-Enforced Budgets

Everruns distinguishes two orthogonal concerns:

| Concern | Capability | Source of truth | Enforcement |
|---------|-----------|-----------------|-------------|
| Platform-enforced limit | `budgeting` | Budgets table / usage ledger | Session paused/stopped automatically at exhaustion |
| User-requested indicative target ("you have $7") | `self_budget` | Session cumulative usage (`get_session_info`) | None, agent adapts behavior via prompt guidance |

The `self_budget` capability contributes prompt-only guidance. It ships no tools (cumulative usage is already exposed by `get_session_info` from the `session` capability, which is present in the Generic harness). The prompt:

- frames the self-budget as an **agent-managed soft target**, not a hard limit
- directs the agent to `get_session_info` for current spend
- encourages periodic re-checks around expensive work rather than every turn
- coaches the agent to adapt (shorter outputs, fewer retries, narrower exploration) as the target tightens
- warns the agent not to claim exact cost certainty from token-only data
- explicitly separates the two budget types so the agent does not conflate platform enforcement with user-stated targets
- explicitly forbids creating or mutating platform budgets in response to a user-stated target

The two capabilities are non-conflicting and co-exist in the built-in `generic` harness. Agents without a need for platform enforcement can still enable only `self_budget`; agents that need strict enforcement should enable `budgeting` (and typically both).

File: `crates/builtins/src/self_budget.rs`.

## Design Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Pre-check vs post-check | Post-check (after event) | Avoids blocking hot path; minor overshoot acceptable |
| Raw activity vs rated usage | Split into `usage_journal` then `usage_ledger` | Preserves raw facts, enables replay/backfill, keeps pricing/rating separate |
| Balance storage | Denormalized on `budgets` + append-only usage ledger | Fast reads; ledger is source of truth for reconciliation |
| Currency as enum vs string | String | Extensible without migrations |
| Budget scope | Per-subject with hierarchy | Flexible: session, agent, user, or org level |
| Pause mechanism | Session status + turn loop yield | Reuses existing session lifecycle; non-destructive |
| Multiple budgets per subject | Allowed, most restrictive wins | User may want both cost and token limits |
| Period handling | JSONB with type discriminator + `period_started_at` snapshot | `Duration { seconds }`, `Rolling { window }` (e.g. `5h`, `30d`), and `Calendar { unit }` (`hour | day | week | month | year`) cover sliding and calendar resets without bespoke columns. The service rolls the budget on every check by comparing `period_started_at` against the current clock. |
| Headless enforcement | Hard stop at balance ≤ 0 | No human to interact with pause; hard stop is safer |
| Cost lookup | `ModelProfile` from `everruns-model-profiles` | Already has per-model pricing from models.dev |

## App / Channel Budgets

Apps own one or more channels. Both layers can hold budgets:

| Subject | When it applies |
|---------|-----------------|
| `app` | every session created for the app (any channel) |
| `app_channel` | sessions originating from a specific channel only |

The hierarchy resolver pulls these subjects from session tags (`app:<id>`, `app_channel:<id>`). The flag `FEATURE_APP_BUDGETS` (experimental, auto-on in dev) is required to create or list app/channel budgets via the API; the storage and check pipeline always honours existing rows so the flag can flip without a backfill.

UI: the App detail page surfaces a "Budgets" card (gated by `app_budgets`) that lists every budget attached to the app or any of its channels, and exposes a form for the common period presets (sliding 1h / 5h / 24h / 7d / 30d, calendar month) plus a "Custom JSON" escape hatch that accepts the raw `BudgetPeriod` payload, the in-product DSL, so advanced rules ship without waiting for first-class form fields.

## Future Work

- **ToolCallMeter**, **DataProcessedMeter**: additional meters
- **Externalized rating rules**: replace hardcoded Rust rating with configurable scripts/expressions
- **Valkey-cached balance**: for high-throughput without DB round-trip per check
- **Budget analytics dashboard**: spend by model, by agent, over time
- **Custom meter registration API**: for external integrations
- **Rule-based DSL**: promote the JSON escape hatch to a typed expression language with a syntax-highlighted editor
