-- v0.8.9: squashed migrations 010–015
-- BREAKING CHANGE: Requires fresh database (drop existing _sqlx_migrations table)
-- This file replaces individual migrations 010_audit_domains through
-- 015_agent_addressable_name. It is NOT idempotent and cannot be re-run
-- against a database that already has those migrations applied.

----------------------------------------------------------------------
-- 010: Audit log domains (EVE-226)
----------------------------------------------------------------------

-- New columns for structured audit domains
ALTER TABLE audit_logs
    ADD COLUMN IF NOT EXISTS domain TEXT NOT NULL DEFAULT 'management',
    ADD COLUMN IF NOT EXISTS action TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS target_type TEXT,
    ADD COLUMN IF NOT EXISTS target_id TEXT;

-- Composite index for domain-scoped queries (primary access pattern)
CREATE INDEX IF NOT EXISTS idx_audit_logs_org_domain_created
    ON audit_logs (org_id, domain, created_at DESC);

-- Action-specific queries (e.g. "show all member.invited events")
CREATE INDEX IF NOT EXISTS idx_audit_logs_org_action_created
    ON audit_logs (org_id, action, created_at DESC)
    WHERE action != '';

----------------------------------------------------------------------
-- 011: Extensible budgeting system (specs/budgeting.md)
----------------------------------------------------------------------

-- Add 'paused' to session status CHECK constraint
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_status_check;
ALTER TABLE sessions ADD CONSTRAINT sessions_status_check
    CHECK (status IN ('started', 'active', 'idle', 'waiting_for_tool_results', 'paused'));

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

-- Budget ledger (append-only)
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

----------------------------------------------------------------------
-- 012: Configurable max_iterations (EVE-211)
----------------------------------------------------------------------

-- Resolution priority: session override > agent config > system default (500)
ALTER TABLE agents ADD COLUMN max_iterations INTEGER;
ALTER TABLE sessions ADD COLUMN max_iterations INTEGER;

----------------------------------------------------------------------
-- 013: Network access list (specs/network-access.md)
----------------------------------------------------------------------

-- Merge semantics: allowed=intersect, blocked=union across layers
ALTER TABLE harnesses ADD COLUMN network_access JSONB;
ALTER TABLE agents ADD COLUMN network_access JSONB;
ALTER TABLE sessions ADD COLUMN network_access JSONB;

----------------------------------------------------------------------
-- 014: Harness addressable name
----------------------------------------------------------------------

-- Makes harness `name` the URL/CLI-friendly addressable identifier (slug).
-- Adds `display_name` for the human-readable label.

-- Step 1: Add display_name column, copy existing name values
ALTER TABLE harnesses ADD COLUMN display_name TEXT;
UPDATE harnesses SET display_name = name;
ALTER TABLE harnesses ALTER COLUMN display_name SET NOT NULL;

-- Step 2: Convert name to slug format (lowercase, spaces/underscores -> hyphens,
-- strip non-alphanumeric-hyphen, collapse consecutive hyphens, trim hyphens).
-- Fall back to 'harness-<short-id>' if slugification yields empty string.
UPDATE harnesses
SET name = CASE
    WHEN TRIM(BOTH '-' FROM
        regexp_replace(
            regexp_replace(
                regexp_replace(LOWER(name), '[^a-z0-9-]', '-', 'g'),
                '-+', '-', 'g'
            ),
            '^-|-$', '', 'g'
        )
    ) = '' THEN 'harness-' || LEFT(REPLACE(id::text, '-', ''), 8)
    ELSE TRIM(BOTH '-' FROM
        regexp_replace(
            regexp_replace(
                regexp_replace(LOWER(name), '[^a-z0-9-]', '-', 'g'),
                '-+', '-', 'g'
            ),
            '^-|-$', '', 'g'
        )
    )
END;

-- Step 3: Handle duplicates within same org by appending a suffix.
WITH dupes AS (
    SELECT id, org_id, name,
           ROW_NUMBER() OVER (PARTITION BY org_id, name ORDER BY created_at, id) AS rn
    FROM harnesses
),
max_existing AS (
    SELECT d.org_id, d.name,
           COALESCE(MAX(
               CASE
                   WHEN h2.name ~ ('^' || d.name || '-[0-9]+$')
                   THEN SUBSTRING(h2.name FROM '-([0-9]+)$')::integer
                   ELSE 0
               END
           ), 0) AS max_suffix
    FROM (SELECT DISTINCT org_id, name FROM dupes WHERE rn > 1) d
    JOIN harnesses h2 ON h2.org_id = d.org_id
    GROUP BY d.org_id, d.name
)
UPDATE harnesses h
SET name = d.name || '-' || (me.max_suffix + d.rn)
FROM dupes d
JOIN max_existing me ON me.org_id = d.org_id AND me.name = d.name
WHERE h.id = d.id AND d.rn > 1;

-- Step 4: Add unique constraint per org (only for non-deleted harnesses)
CREATE UNIQUE INDEX idx_harnesses_org_name ON harnesses (org_id, name) WHERE status != 'deleted';

----------------------------------------------------------------------
-- 015: Agent addressable name
----------------------------------------------------------------------

-- Similar to the harness pattern (section 014): `name` becomes a
-- URL/CLI-friendly slug, `display_name` holds the human-readable label.
-- Unlike harnesses, agent display_name is intentionally nullable
-- (Option<String> in AgentRow) because the agent API allows omitting it.

-- Step 1: Add display_name column, copy existing name values
ALTER TABLE agents ADD COLUMN display_name TEXT;
UPDATE agents SET display_name = name;

-- Step 2: Convert name to lowercase slug format.
-- Fall back to 'agent-<short-id>' if result is empty.
UPDATE agents
SET name = CASE
    WHEN TRIM(BOTH '-' FROM
        regexp_replace(
            regexp_replace(
                regexp_replace(LOWER(name), '[^a-z0-9-]', '-', 'g'),
                '-+', '-', 'g'
            ),
            '^-|-$', '', 'g'
        )
    ) = '' THEN 'agent-' || LEFT(REPLACE(id::text, '-', ''), 8)
    ELSE TRIM(BOTH '-' FROM
        regexp_replace(
            regexp_replace(
                regexp_replace(LOWER(name), '[^a-z0-9-]', '-', 'g'),
                '-+', '-', 'g'
            ),
            '^-|-$', '', 'g'
        )
    )
END;

-- Step 3: Handle duplicates within same org by appending a suffix.
WITH dupes AS (
    SELECT id, org_id, name,
           ROW_NUMBER() OVER (PARTITION BY org_id, name ORDER BY created_at, id) AS rn
    FROM agents
),
max_existing AS (
    SELECT d.org_id, d.name,
           COALESCE(MAX(
               CASE
                   WHEN a2.name ~ ('^' || d.name || '-[0-9]+$')
                   THEN SUBSTRING(a2.name FROM '-([0-9]+)$')::integer
                   ELSE 0
               END
           ), 0) AS max_suffix
    FROM (SELECT DISTINCT org_id, name FROM dupes WHERE rn > 1) d
    JOIN agents a2 ON a2.org_id = d.org_id
    GROUP BY d.org_id, d.name
)
UPDATE agents a
SET name = d.name || '-' || (me.max_suffix + d.rn)
FROM dupes d
JOIN max_existing me ON me.org_id = d.org_id AND me.name = d.name
WHERE a.id = d.id AND d.rn > 1;

-- Step 4: Add unique constraint per org (only for non-deleted agents)
CREATE UNIQUE INDEX idx_agents_org_name ON agents (org_id, name) WHERE status != 'deleted';
