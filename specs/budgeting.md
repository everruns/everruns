# Budgeting Specification

## Abstract

Extensible budgeting system for controlling resource consumption across sessions, agents, users, and organizations. Supports multiple currencies (USD cost, token counts, custom credits), pluggable metering sources, pluggable policy rules, and soft enforcement (pause/warn before hard-stop).

## Concepts

```
┌─────────────────────────────────────────────────────┐
│                    Budget                             │
│  subject: (session | agent | user | org)             │
│  currency: (usd | tokens | credits | custom)        │
│  limit: Decimal                                      │
│  soft_limit: Option<Decimal>  (pause/warn threshold) │
│  period: Option<Period>       (rolling/calendar)     │
│  balance: computed from ledger                       │
└──────────────┬──────────────────────────────────────┘
               │ 1:N
┌──────────────▼──────────────────────────────────────┐
│                 Ledger Entry                          │
│  budget_id, amount, meter_source, ref_id, timestamp  │
└─────────────────────────────────────────────────────┘

┌─────────────────────────┐    ┌──────────────────────┐
│  Meter (pluggable)      │    │  Rule  (pluggable)   │
│  Emits ledger entries   │    │  Evaluates budgets   │
│  e.g. LlmTokenMeter,   │    │  e.g. PauseRule,     │
│  ToolCallMeter,         │    │  WarnRule,           │
│  DataProcessedMeter     │    │  HardStopRule        │
└─────────────────────────┘    └──────────────────────┘
```

### Budget

A spending cap bound to a **subject** (who) in a **currency** (what unit).

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID | Internal PK |
| `public_id` | TEXT | Prefixed ID (`bdgt_xxx`) |
| `org_id` | BIGINT | Owning organization |
| `subject_type` | TEXT | `session`, `agent`, `user`, `org` |
| `subject_id` | UUID | FK to the subject entity |
| `currency` | TEXT | `usd`, `tokens`, `credits`, or custom string |
| `limit` | DECIMAL | Hard ceiling |
| `soft_limit` | DECIMAL | Optional. Triggers soft enforcement (pause/warn) |
| `period` | JSONB | Optional. `{ "type": "rolling", "window": "24h" }` or `{ "type": "calendar", "unit": "month" }`. NULL = lifetime. |
| `metadata` | JSONB | Arbitrary KV for extensions |
| `status` | TEXT | `active`, `paused`, `exhausted`, `disabled` |
| `created_at` | TIMESTAMPTZ | |
| `updated_at` | TIMESTAMPTZ | |

A subject can have multiple budgets (e.g. a session with both a $10 USD budget and a 100k token budget). All active budgets are evaluated; the most restrictive one wins.

### Ledger Entry

Immutable, append-only record of resource consumption against a budget.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID | PK |
| `budget_id` | UUID | FK to budget |
| `amount` | DECIMAL | Positive = debit (consumption), negative = credit (top-up/refund) |
| `meter_source` | TEXT | Which meter produced this (`llm_tokens`, `tool_calls`, `data_bytes`, ...) |
| `ref_type` | TEXT | Reference entity type (`llm_generation`, `tool_execution`, `manual`) |
| `ref_id` | UUID | FK to source record (e.g. `llm_generations.id`) |
| `session_id` | UUID | Session context (for aggregation) |
| `description` | TEXT | Human-readable note |
| `created_at` | TIMESTAMPTZ | Immutable |

**Balance** = `budget.limit - SUM(ledger.amount)`. Maintained as a denormalized `balance` column on `budgets` updated atomically with each ledger insert (single transaction, `SELECT ... FOR UPDATE`).

### Currency

Currencies are strings, not an enum — new currencies added without migrations.

| Currency | Unit | Typical meter source |
|----------|------|---------------------|
| `usd` | Dollar amount | `llm_tokens` (token count * model cost/token) |
| `tokens` | Raw token count | `llm_tokens` (input + output) |
| `credits` | Platform credits | Any (1 credit = org-defined value) |
| Custom | User-defined | Custom meter |

**Conversion**: Meters emit in their native unit. A `CurrencyConverter` trait maps native units to budget currency. For example, the `LlmTokenMeter` emits token counts; when the budget currency is `usd`, the converter multiplies by the model's cost-per-token from `LlmModelProfile`.

```rust
trait CurrencyConverter: Send + Sync {
    /// Convert a metered amount to the budget's currency.
    /// Returns None if conversion is not possible (incompatible currency).
    fn convert(
        &self,
        amount: Decimal,
        from_unit: &str,      // e.g. "tokens"
        to_currency: &str,    // e.g. "usd"
        context: &MeterContext,
    ) -> Option<Decimal>;
}
```

## Meters (Pluggable Sources)

A **Meter** observes system events and produces ledger entries. Meters are registered in a `MeterRegistry` (similar to `CapabilityRegistry`).

```rust
#[async_trait]
trait Meter: Send + Sync + 'static {
    /// Unique meter identifier, e.g. "llm_tokens"
    fn id(&self) -> &str;

    /// Which event types this meter listens to
    fn event_types(&self) -> &[&str];

    /// Produce zero or more ledger debits from an event.
    /// Returns (amount_in_native_unit, native_unit, ref_type, ref_id).
    async fn measure(&self, event: &EventData, ctx: &MeterContext) -> Vec<MeterReading>;
}

struct MeterReading {
    amount: Decimal,
    native_unit: String,    // "tokens", "calls", "bytes"
    ref_type: String,
    ref_id: Uuid,
}
```

### Built-in Meters

| Meter | Listens to | Native unit | What it measures |
|-------|-----------|-------------|-----------------|
| `LlmTokenMeter` | `llm.generation` | `tokens` | `input_tokens + output_tokens` per generation |
| `ToolCallMeter` | `tool.completed` | `calls` | 1 per tool invocation |
| `DataProcessedMeter` | `tool.completed` | `bytes` | Size of tool input/output payloads |

Adding a new meter: implement `Meter`, register in `MeterRegistry`. No schema changes needed.

## Rules (Pluggable Policies)

A **Rule** defines what happens when a budget crosses a threshold.

```rust
#[async_trait]
trait BudgetRule: Send + Sync + 'static {
    fn id(&self) -> &str;

    /// Evaluate after each ledger entry. Returns an action.
    async fn evaluate(&self, budget: &Budget, new_balance: Decimal) -> RuleAction;
}

enum RuleAction {
    /// No action needed
    Continue,
    /// Emit a warning event to the session
    Warn { message: String },
    /// Pause the session — requires user input to resume
    Pause { message: String },
    /// Hard stop — terminate the current turn
    Stop { message: String },
}
```

### Built-in Rules

| Rule | Triggers when | Action |
|------|--------------|--------|
| `SoftLimitWarnRule` | balance < soft_limit | `Warn` — emits `budget.warning` event |
| `SoftLimitPauseRule` | balance < soft_limit | `Pause` — emits `budget.paused` event, sets session to `paused` |
| `HardLimitStopRule` | balance <= 0 | `Stop` — emits `budget.exhausted` event, terminates turn |

Rules are evaluated in order: warn → pause → stop. First `Pause` or `Stop` wins.

### Soft Enforcement: Pause

Pause is a **soft prevention** mechanism:

1. Budget crosses soft_limit threshold
2. `SoftLimitPauseRule` fires, returns `RuleAction::Pause`
3. Engine emits `budget.paused` SSE event with context (which budget, how much consumed, etc.)
4. Session status transitions to `paused` (new status value)
5. Current turn completes its in-flight atom but does not start the next one
6. User sees a banner: "Budget limit reached. Add funds or increase limit to continue."
7. User can: (a) increase limit via API/UI, (b) add credits, (c) dismiss and continue (override)
8. On resume, session returns to `active`

```
Session states: started → active → idle
                          ↓        ↑
                        paused ────┘ (resume)
```

## Evaluation Pipeline

On every metered event:

```
Event (e.g. llm.generation)
  │
  ▼
MeterRegistry.dispatch(event)
  │
  ├─► LlmTokenMeter.measure() → MeterReading { 1500 tokens }
  │
  ▼
Find active budgets for session's subject hierarchy:
  session → agent → user → org
  │
  ▼ (for each matching budget)
CurrencyConverter.convert(1500 tokens → $0.003 USD)
  │
  ▼
INSERT ledger entry + UPDATE budget.balance (single txn)
  │
  ▼
RuleEngine.evaluate(budget, new_balance)
  │
  ├─► SoftLimitWarnRule  → Warn?
  ├─► SoftLimitPauseRule → Pause?
  └─► HardLimitStopRule  → Stop?
  │
  ▼
Execute action (emit event / pause session / stop turn)
```

**Subject hierarchy**: budgets cascade. A session inherits its agent's, user's, and org's budgets. If any budget in the chain triggers, enforcement applies. This means an org-level $100/month budget constrains all sessions within that org.

### Performance

Budget checks happen **after** each metered event (post-hoc), not before. This avoids blocking the hot path. The window between check and next LLM call is small enough that minor overshoot is acceptable (and expected — the last generation that pushed over the limit completes).

For high-throughput scenarios, balance can be checked from a cached value (Valkey) with periodic reconciliation against the ledger.

## Events

| Event Type | When | Data |
|-----------|------|------|
| `budget.warning` | Balance crosses soft_limit | `{ budget_id, balance, soft_limit, limit, currency, message }` |
| `budget.paused` | Pause rule fires | `{ budget_id, balance, soft_limit, limit, currency, session_id }` |
| `budget.exhausted` | Balance reaches 0 | `{ budget_id, balance, limit, currency, session_id }` |
| `budget.resumed` | User resumes after pause | `{ budget_id, balance, limit, currency }` |
| `budget.updated` | Budget created/modified | `{ budget_id, limit, soft_limit, currency }` |

## API

### Budget CRUD

```
POST   /v1/budgets                    Create budget
GET    /v1/budgets                    List budgets (filterable by subject)
GET    /v1/budgets/:id                Get budget with current balance
PATCH  /v1/budgets/:id                Update limit / soft_limit / status
DELETE /v1/budgets/:id                Soft-delete

POST   /v1/budgets/:id/top-up        Add credits (negative ledger entry)
GET    /v1/budgets/:id/ledger         Paginated ledger entries
```

### Session-level shortcuts

```
POST   /v1/sessions/:id/budget       Attach a budget to this session
POST   /v1/sessions/:id/resume       Resume a paused session (budget override)
```

### Agent-level shortcuts

```
POST   /v1/agents/:id/budget         Attach a budget to this agent
```

### Example: Give a session $10

```json
POST /v1/budgets
{
  "subject_type": "session",
  "subject_id": "ses_abc123",
  "currency": "usd",
  "limit": 10.00,
  "soft_limit": 8.00
}
```

### Example: Give a user 100 credits/month

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

## Data Model (SQL)

```sql
CREATE TABLE budgets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    public_id TEXT NOT NULL UNIQUE,
    org_id BIGINT NOT NULL REFERENCES organizations(id),
    subject_type TEXT NOT NULL,  -- 'session', 'agent', 'user', 'org'
    subject_id TEXT NOT NULL,    -- public_id of the subject
    currency TEXT NOT NULL,      -- 'usd', 'tokens', 'credits', ...
    "limit" DECIMAL NOT NULL,
    soft_limit DECIMAL,
    balance DECIMAL NOT NULL,    -- denormalized: limit - sum(debits) + sum(credits)
    period JSONB,                -- null = lifetime
    metadata JSONB DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_budgets_subject ON budgets(org_id, subject_type, subject_id)
    WHERE status = 'active';

CREATE TABLE budget_ledger (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    budget_id UUID NOT NULL REFERENCES budgets(id),
    amount DECIMAL NOT NULL,
    meter_source TEXT NOT NULL,
    ref_type TEXT,
    ref_id UUID,
    session_id UUID,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Append-only: no UPDATE/DELETE trigger (same pattern as events table)
CREATE INDEX idx_budget_ledger_budget ON budget_ledger(budget_id, created_at);
CREATE INDEX idx_budget_ledger_session ON budget_ledger(session_id, created_at);
```

## Integration Points

### Where meters hook in

Meters subscribe to events via the existing event pipeline. The `BudgetService` registers as an event listener (same mechanism as usage tracking's `llm_generations` insert). No changes to the LLM driver or tool execution hot path.

```
LLM Driver → emits llm.generation event
                    │
                    ├──► Usage tracking (existing: insert llm_generations)
                    └──► BudgetService.on_event() (new: meter + ledger + rules)
```

### Where pause hooks in

The worker's turn loop already checks session status between atoms. Adding `paused` as a recognized status causes the loop to yield, waiting for a resume signal (same mechanism as user-initiated pause, if added). The turn does not abort — it suspends and can be resumed.

### Capability (optional)

Budgets can optionally be exposed as a capability (`budgeting`) that adds:
- System prompt section informing the agent of remaining budget
- A `check_budget` tool the agent can call to see its remaining balance
- Useful for agents that should self-regulate (e.g. "wrap up if budget is low")

## Implementation Phases

### Phase 1: Core

- `budgets` + `budget_ledger` tables and migrations
- `Budget` and `LedgerEntry` models in `everruns-core`
- `BudgetService` in server with CRUD API
- `LlmTokenMeter` (most impactful meter)
- `HardLimitStopRule` (simplest enforcement)
- USD and tokens currencies with `LlmModelProfile`-based conversion
- `budget.exhausted` and `budget.updated` events

### Phase 2: Soft enforcement

- `SoftLimitPauseRule` and `SoftLimitWarnRule`
- Session `paused` status and resume flow
- `budget.warning`, `budget.paused`, `budget.resumed` events
- UI: budget display on session page, pause banner, resume button

### Phase 3: Extended meters + periods

- `ToolCallMeter`, `DataProcessedMeter`
- Rolling and calendar period support (balance resets)
- Credits currency with org-defined exchange rate
- Budget inheritance (agent/user/org budgets apply to sessions)
- Valkey-cached balance for high-throughput

### Phase 4: Agent awareness

- `budgeting` capability with system prompt + `check_budget` tool
- Custom meter registration API (for external integrations)
- Budget analytics dashboard (spend by model, by agent, over time)

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
