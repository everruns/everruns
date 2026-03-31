-- Extensible budgeting system
-- See specs/budgeting.md

-- Add 'paused' to session status CHECK constraint
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_status_check;
ALTER TABLE sessions ADD CONSTRAINT sessions_status_check
    CHECK (status IN ('started', 'active', 'idle', 'waiting_for_tool_results', 'paused'));

----------------------------------------------------------------------
-- Budgets table
----------------------------------------------------------------------

CREATE TABLE budgets (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    -- Subject: what entity this budget constrains
    subject_type TEXT NOT NULL CHECK (subject_type IN ('session', 'agent', 'user', 'org')),
    subject_id TEXT NOT NULL,  -- public_id of the subject entity
    -- Currency: "usd", "tokens", "credits", or custom string
    currency TEXT NOT NULL,
    -- Spending limits
    "limit" DOUBLE PRECISION NOT NULL,
    soft_limit DOUBLE PRECISION,
    -- Denormalized balance: limit - sum(debits) + sum(credits)
    balance DOUBLE PRECISION NOT NULL,
    -- Optional period for recurring budgets (JSONB with type discriminator)
    period JSONB,
    -- Arbitrary metadata for extensions
    metadata JSONB DEFAULT '{}',
    -- Status
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'exhausted', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Fast lookup: active budgets for a subject
CREATE INDEX idx_budgets_subject ON budgets(org_id, subject_type, subject_id)
    WHERE status = 'active';

-- Trigger for updated_at
CREATE TRIGGER set_budgets_updated_at
    BEFORE UPDATE ON budgets
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

----------------------------------------------------------------------
-- Budget ledger (append-only)
----------------------------------------------------------------------

CREATE TABLE budget_ledger (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    budget_id UUID NOT NULL REFERENCES budgets(id),
    -- Positive = debit (consumption), negative = credit (top-up/refund)
    amount DOUBLE PRECISION NOT NULL,
    -- Which meter produced this: "llm_tokens", "tool_calls", etc.
    meter_source TEXT NOT NULL,
    -- Reference to source record
    ref_type TEXT,
    ref_id UUID,
    -- Session context
    session_id UUID,
    -- Human-readable note
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- No UPDATE/DELETE on ledger (append-only)
CREATE TRIGGER prevent_budget_ledger_mutation
    BEFORE UPDATE OR DELETE ON budget_ledger
    FOR EACH ROW
    EXECUTE FUNCTION prevent_event_mutation();

CREATE INDEX idx_budget_ledger_budget ON budget_ledger(budget_id, created_at);
CREATE INDEX idx_budget_ledger_session ON budget_ledger(session_id, created_at);
