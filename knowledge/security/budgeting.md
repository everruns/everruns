---
type: Specification
title: "Costs and Usage Policies Specification"
description: "Org cost accounting (usage record, price list, cost basis) and usage policies (budgets) that notify, ask to continue, or stop."
tags:
  - everruns
  - security
---
# Costs and Usage Policies Specification

## Abstract

Two things, kept apart on purpose:

- **Costs**: what usage cost the organization, by the org's own price list. Every
  usage is recorded whether or not a policy covers it, and every cost says where
  its number came from (its **basis**).
- **Usage policies** (stored as budgets): rules on cost or tokens over a window,
  with steps that **notify**, **ask to continue**, or **stop**.

Credits are not an OSS concept. A host that bills usage itself (the hosted
platform charges Everruns credits for built-in models) reports what it charged
through `BudgetService::record_host_charge`, and that charge replaces the
estimate for the usage. The API keeps the `/v1/budgets` paths; the UI says
"usage policy".

Budget enforcement remains owned by this domain. Analytical budget reports are
derived asynchronously from `usage_ledger` and related budget metadata; see
[knowledge/evaluation/reporting.md](../evaluation/reporting.md).

Key use cases:
- **Volunteer budget**: User sets a $10 budget when creating a session. Session stops at $10.
- **Admin budget**: Admin sets a $50/month budget on a user. All sessions for that user stop when the user hits $50.
- **Stacked budgets**: Session has a $10 volunteer budget AND the user has a $50 admin budget. The session stops at whichever limit is hit first (most restrictive wins).

## Implementation

See source files for full definitions:
- Core types: `crates/contracts/src/runtime/budget.rs`
- Typed IDs: `crates/contracts/src/typed_id.rs` (`BudgetId`, `LedgerEntryId`)
- Events: `crates/core/src/events.rs` (budget event constants and `BudgetEventData`)
- Storage: `crates/server/src/storage/repositories/budgets/`
- Service: `crates/server/src/domains/budgets/service.rs`
- API: `crates/server/src/api/budgets.rs`
- Capability: `crates/core/src/builtins/budgeting.rs`
- Loop gate: `crates/core/src/host/budget_gate.rs` (applied in `execute_reason_activity_with_prompt_messages`, `crates/core/src/host/host.rs`)
- Migrations: `crates/server/migrations/legacy/010_v0.8.9.sql`, `crates/server/migrations/legacy/020_budget_journal_ledger.sql`

## Cost of each usage

Each usage journal row gets exactly one cost posting: a `usage_ledger` row with
no `budget_id`, `currency = usd`, and a `basis`. Bases, in priority order
([rating](../../crates/server/src/domains/budgets/costs.rs)):

1. `host`: the host reported its charge (a later `cost_host` posting carries
   the difference from the first rating, so the usage's cost is the charge).
2. `rate`: a row of the org's price list matched (`rate_id` says which).
3. `provider`: the provider returned a cost with the response.
4. `list_price`: the model catalog's cache-aware estimate.
5. `unpriced`: no price anywhere. The cost is unknown, never zero, and never a
   token count read as dollars (that fallback was a bug before 2026-11-01).

Totals that include unpriced usage are partial and the UI says so.

### Price list

`cost_rates`: per org, per meter (`model.tokens`, `sandbox.time`), a `match` on
`provider` / `model` / `source` (`managed` or `own_keys`) or `provider` / `size`,
and either a fixed `unit_price_usd` (per million tokens, per hour) or a
`list_price_multiplier` (0.8 is a 20% discount, 0 counts usage at no cost).
Rows are never edited: a change is a new row with a later `effective_from`, and
retiring is a row with `retired = true`, so past costs keep the rate they used.
The most specific match wins, then the latest start.

### Sandbox time

A cluster job ([meter](../../crates/server/src/background/sandbox_time_meter.rs))
turns each closed `ready` span of a sandbox into a `sandbox_time` usage record,
priced only by a price-list rate (sandboxes have no list price). Running
sandboxes are charged when they stop.

### Host charges

`record_host_charge(HostCharge { source_type, source_id, amount_usd, label, .. })`
finds the usage by its journal source (`event` + the `llm.generation` event id),
posts `cost_host` with the difference, and debits dollar policies by the same
difference (`host_correction`), so a $10 policy means $10 of what the host
charged. A second report for the same usage changes nothing.

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
│  currency: (usd | tokens)  policy: steps, source    │
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

**Subject types**: `session`, `agent_channel`, `agent_trigger`, `agent`, `user`, `org`, budgets cascade through the hierarchy from most specific (session) to most general (org). A session's effective budgets include all matching levels. Two more levels bind only unattended turns: `session_schedule` (the schedule whose fire started the turn) and `org_unattended` (the org's daily cap); see [Background Work Limits](../operations/background-limits.md). One more binds only eval case sessions: `eval_run`, the spending cap of the run whose case created the session, resolved from `eval_case_results` for sessions with the `eval` source; see [Evals](../evaluation/evals.md).

**Subject identifiers**: a `subject_id` is always the identifier the API exposes for that subject, because that is the only identifier a caller can put in a budget — `session_…`, `agent_…` (`agents.public_id`), `usr_…`, `org_…`, the endpoint's `appchan_…` public id, and the trigger's `trg_…` id. Internal primary keys never appear in a budget subject. The hierarchy resolver therefore has to translate the internal ids it holds — it takes `sessions.agent_id` and `sessions.channel_id`, both FKs to internal `id` columns, and looks up the corresponding public id before matching. `sessions.trigger_id` is the exception that proves the rule: a trigger has no separate `public_id`, so its API id is rendered straight from the primary key and no lookup is needed. Rendering a typed id directly would spell the internal uuid and match nothing, which is what made agent-scoped budgets silently never bind until EVE-1136.

`agent_channel` budgets resolve from the session's structural endpoint reference. Slack and FCP routing tags do not determine budget identity.

The `app` level is retired (EVE-1129, migration 151). Each `app` budget was converted onto its App's agent, preserving limit, recorded spend and the in-flight window, before the rows and the subject type were dropped. Conversion inserted alongside any cap the agent already had rather than skipping, because every matching budget binds and the most restrictive wins — skipping would have left a looser existing cap as the only one binding, raising the ceiling.

`app_channel` is retired too (EVE-1138, migration 153), and with it the last budget subject resolved from a session tag. Migration 138 had moved App webhooks onto `agent_triggers`, moved their budgets back from `agent_channel` to `app_channel`, and deleted the endpoint rows, leaving those caps keyed on the `app_channel:<channel_id>` tag because nothing structural survived to key them on. Migration 153 adds `sessions.trigger_id` and re-keys them onto the `agent_trigger` subject.

The new subject is keyed on the trigger's API id (`trg_…`), not on `agent_triggers.ingress_id`. `ingress_id` would have made the conversion a pure rename, since it is exactly what the surviving rows are keyed by, but it is a nullable compatibility column carried only by the webhooks migration 138 moved — a trigger created today has none and could never be given a budget. An `app_channel` budget with no matching trigger fails the migration rather than being dropped: deleting an enforced ceiling is the harm the conversion exists to avoid.

The four App-era prefixes stay in `RESERVED_SESSION_TAG_PREFIXES` regardless of what reads them — that list is append-only.

**Currencies**: `usd` (the rated cost of each usage) and `tokens`. New policies accept only these two. The retired `credits` currency (tokens / 1000) was converted to `tokens` by migration `20261101154145` (limit, soft limit and balance × 1000, `metadata.converted_from = "credits"`); any other legacy currency is never debited.

**Policy**: `budgets.policy` holds an optional name, the steps, and a source filter (`all`, `managed` built-in models only, `own_keys` only, which includes sandboxes). Steps are `{at_percent, action}` with action `notify`, `ask` or `stop`; a policy always stops at 100%. A policy without steps keeps the long-standing behavior: notify at 80%, ask at the soft limit when one is set, stop at the limit ([UsagePolicy](../../crates/contracts/src/runtime/budget.rs)).

**Windows**: calendar days, weeks and months start at midnight in the org's timezone (`organization_settings.timezone`, UTC until set). The unattended daily cap follows it too.

**Balance**: `limit - SUM(debits) + SUM(credits)`. Denormalized on `budgets.balance`, updated atomically with each budget-scoped ledger insert (transaction + `SELECT ... FOR UPDATE`).

**Status lifecycle**: `active` → `exhausted` (a stop step reached) → `disabled` (soft-deleted). Raising the limit or changing the policy reactivates an exhausted policy. An ask step does not change the status: it is answered per session (below). `paused` is only a manual status now; approving a session clears it.

### Usage Journal

Immutable raw activity fact, written for every usage whether or not a policy covers it. Current writers:
- `llm_generation` from `llm.generation` events (`measures.quantity` in tokens; `metadata` carries `managed`, `source_key`/`source_label`, `started_by`)
- `sandbox_time` from closed sandbox spans (`measures.quantity` in hours)
- synthetic adjustment rows for budget top-ups / manual compatibility writes

The journal stores scope (`org_id`, `user_id`, `principal_id`, `session_id`, `agent_id`, `harness_id`) plus raw `measures` and free-form `metadata`.

A generation can declare billable components beyond its tokens (`cost_components` on `llm.generation`, used by the OpenAI Agents API backend for hosted tools and containers). The meter debits the priced ones and records every component whose amount is unknown in the journal metadata and the ledger's rating metadata as `cost_unknown_components`, so a total never silently treats an unknown amount as zero. A generation whose token usage is unknown but declares components still journals, with zero tokens ([meter](../../crates/server/src/domains/budgets/service.rs), `GenerationMeter`). When the provider reports that usage later (an Agents API turn, EVE-1145), the late tokens are metered once more as their own journal entry, sourced `llm_generation_late_usage` with the generation record's id, so the unique `(source_type, source_id)` index refuses a second late debit of the same generation ([late usage](../execution/openai-agents-api-runtime.md#observability-and-cost)).

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
  (root session → agent channel → agent trigger → agent → user → org)
  │
  ▼ (for each matching budget)
rate the usage once (price list > provider > list price > unpriced)
  + INSERT its cost posting (budget_id NULL, basis)
  │
  ▼ (for each matching active policy whose source filter counts it)
debit
  │  "usd"    → the rated cost; unpriced usage is not debited
  │  "tokens" → the token count
  │
  ▼
INSERT usage_ledger row (journal_id -> budget_id)
  + UPDATE budget.balance (atomic)
  │
  ▼
evaluate_rules(budget): the most restrictive step reached
  │  stop   → mark the policy "exhausted"
  │  ask    → nothing stored; the loop gate asks before the next call
  │  notify → notify org owners and admins, once per step and window
  │           (usage_policy_notices), linking to Settings → Costs
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

**Loop gate**: debits are post-hoc, so the loop reads budget status before every provider call instead. Each reason atom first asks the host's `BudgetChecker` (`RuntimeHostAdapter::budget_checker`: direct in the server, gRPC from remote workers, `InProcessRuntimeBuilder::budget_checker` for embedders) once. `exhausted` or `paused` ends the turn before the model runs: the gate emits `budget.exhausted` / `budget.paused` and the canonical error message, and the engine's failed-reason path emits a single `turn.failed` with the `budget_exhausted` / `budget_paused` code. The in-process loop, durable workflows and worker activities all reach the gate through the one reason entry point; the OpenAI Agents API backend applies the same evaluation before each tool batch. A checker error fails open (logged), because an unreachable budget store must not stop every turn. Overshoot is bounded by the one generation already in flight. The checker resolves the full hierarchy (`root session`, `agent channel`, `agent trigger`, `agent`, `user`, `org`) from the session owner and org context.

## Ask to continue

An ask step is answered by the person using the agent, per session:

1. Spend crosses the step. The loop gate (via `check_budgets_for_session`) reports `pause` before the next provider call, the turn fails with `budget_paused`, and chat shows a "Continue this turn" card.
2. `POST /v1/sessions/{id}/resume` (any member who can view policies) records a `usage_policy_approvals` row for the session and the policy's current window, and the person sends again.
3. That session (and its delegation tree, through the root) continues; other sessions are still asked. The approval ends when the window resets. A stop step is never passed by an approval.

Unattended work has no one to ask: a schedule or trigger turn stops at an ask step the same way, and its owner sees it in the session.

## Events

| Event Type | When | Data (BudgetEventData) |
|-----------|------|------|
| `budget.warning` | Balance ≤ 20% of limit, at the first reason of a turn (once per turn) | `budget_id, balance, limit, currency, message` |
| `budget.paused` | The loop gate stops a turn on a paused budget | `budget_id, balance, limit, currency, soft_limit, message` |
| `budget.exhausted` | The loop gate stops a turn on an exhausted budget | `budget_id, balance, limit, currency, message` |
| `budget.resumed` | User resumes after pause/top-up (not emitted yet) | `budget_id, balance, limit, currency` |

All four events use the same `BudgetEventData` struct. See `crates/core/src/events.rs`. The loop gate emits the first three, because the debit path (`BudgetService` as an event listener) has no event emitter. `budget_id` comes from `BudgetSummary.budget_id` and is empty when a checker does not report one.

## API

### Budget CRUD

```
POST   /v1/budgets                        Create budget
GET    /v1/budgets                        List budgets (?subject_type=&subject_id=)
GET    /v1/budgets/{id}                   Get budget with current balance
PATCH  /v1/budgets/{id}                   Update limit / soft_limit / period / policy / status
DELETE /v1/budgets/{id}                   Soft-delete (sets status=disabled)

POST   /v1/budgets/{id}/top-up           Allow more in this window (negative ledger entry)
GET    /v1/budgets/{id}/ledger           Paginated ledger entries (?limit=&offset=)
GET    /v1/budgets/{id}/check            Check budget status
```

### Session shortcuts

```
GET    /v1/sessions/{id}/budgets          List budgets for this session
GET    /v1/sessions/{id}/budget-check     Check all budgets (session + hierarchy)
POST   /v1/sessions/{id}/resume           Let this session past ask steps for the current window
```

### Costs

```
GET    /v1/cost-rates                     Price list, history included
POST   /v1/cost-rates                     Add a rate (never edits one)
POST   /v1/cost-rates/{id}/retire         Retire a rate from a date
GET    /v1/costs/summary                  Org cost by basis, source, agent, person, day
GET    /v1/agents/{id}/costs              One agent's cost this month
GET    /v1/costs/settings                 Org timezone for calendar windows
PATCH  /v1/costs/settings
```

Commands: [cost_commands.rs](../../crates/server/src/domains/budgets/cost_commands.rs).

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

**Give an agent $50 a month, notify at 80%, ask at 90%:**
```json
POST /v1/budgets
{
  "subject_type": "agent",
  "subject_id": "agent_xyz789",
  "currency": "usd",
  "limit": 50,
  "period": { "type": "calendar", "unit": "month" },
  "policy": {
    "name": "Research assistant",
    "steps": [
      { "at_percent": 80, "action": "notify" },
      { "at_percent": 90, "action": "ask" },
      { "at_percent": 100, "action": "stop" }
    ]
  }
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
This allows $5 more in the current window. If the policy was `paused` or `exhausted` and now has positive balance, it reactivates.

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

File: `crates/core/src/builtins/self_budget.rs`.

## Design Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Pre-check vs post-check | Post-check debit, plus a status read before each provider call | Debiting stays off the hot path; one checker call per reason stops the next generation, so overshoot is at most the generation in flight |
| Raw activity vs rated usage | Split into `usage_journal` then `usage_ledger` | Preserves raw facts, enables replay/backfill, keeps pricing/rating separate |
| Balance storage | Denormalized on `budgets` + append-only usage ledger | Fast reads; ledger is source of truth for reconciliation |
| Currency as enum vs string | String | Extensible without migrations |
| Budget scope | Per-subject with hierarchy | Flexible: session, agent, user, or org level |
| Pause mechanism | Session status + turn loop yield | Reuses existing session lifecycle; non-destructive |
| Multiple budgets per subject | Allowed, most restrictive wins | User may want both cost and token limits |
| Period handling | JSONB with type discriminator + `period_started_at` snapshot | `Duration { seconds }`, `Rolling { window }` (e.g. `5h`, `30d`), and `Calendar { unit }` (`hour | day | week | month | year`) cover sliding and calendar resets without bespoke columns. The service rolls the budget on every check by comparing `period_started_at` against the current clock. |
| Headless enforcement | Hard stop at balance ≤ 0 | No human to interact with pause; hard stop is safer |
| Cost lookup | `ModelProfile` from `everruns-model-profiles` | Already has per-model pricing from models.dev |

## Channel Budgets

| Subject | When it applies |
|---------|-----------------|
| `agent_trigger` | sessions a trigger's ingress started, matched on `sessions.trigger_id` |

`app` and `app_channel` are both retired — see **Budget** above for what their ceilings were converted onto. No budget subject is resolved from a session tag any more.

Neither can be created through the API: `validate_subject_type` in `crates/server/src/domains/budgets/commands.rs` accepts only the live set, and never accepted either of them, which is why migration 153 could assume every `app_channel` row came from migration 138. Budget management has no feature flag: the `budget.view` and `budget.manage` policies decide who sees and changes budgets, and the check pipeline always honours existing rows.

UI: the Agent Integrations tab surfaces budget controls (shown to members with `budget.view`) for the agent and for each of its channels, and exposes a form for the common period presets (sliding 1h / 5h / 24h / 7d / 30d, calendar month) plus a "Custom JSON" escape hatch that accepts the raw `BudgetPeriod` payload, the in-product DSL, so advanced rules ship without waiting for first-class form fields.

## Future Work

- **ToolCallMeter**, **DataProcessedMeter**: additional meters
- **Externalized rating rules**: replace hardcoded Rust rating with configurable scripts/expressions
- **Valkey-cached balance**: for high-throughput without DB round-trip per check
- **Budget analytics dashboard**: spend by model, by agent, over time
- **Custom meter registration API**: for external integrations
- **Rule-based DSL**: promote the JSON escape hatch to a typed expression language with a syntax-highlighted editor
