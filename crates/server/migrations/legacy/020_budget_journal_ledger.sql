-- Generic activity journal plus rated usage ledger for budgeting and future metering.

CREATE TABLE usage_journal (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    kind TEXT NOT NULL,
    source_type TEXT,
    source_id TEXT,
    event_id UUID REFERENCES events(id) ON DELETE SET NULL,
    session_id UUID REFERENCES sessions(id) ON DELETE SET NULL,
    turn_id UUID,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    principal_id UUID REFERENCES principals(id) ON DELETE SET NULL,
    agent_id UUID REFERENCES agents(id) ON DELETE SET NULL,
    harness_id UUID REFERENCES harnesses(id) ON DELETE SET NULL,
    measures JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_usage_journal_source
    ON usage_journal(source_type, source_id)
    WHERE source_type IS NOT NULL AND source_id IS NOT NULL;

CREATE UNIQUE INDEX idx_usage_journal_kind_event
    ON usage_journal(kind, event_id)
    WHERE event_id IS NOT NULL;

CREATE INDEX idx_usage_journal_org_created
    ON usage_journal(org_id, created_at DESC);
CREATE INDEX idx_usage_journal_session_created
    ON usage_journal(session_id, created_at DESC)
    WHERE session_id IS NOT NULL;
CREATE INDEX idx_usage_journal_kind_created
    ON usage_journal(kind, created_at DESC);

CREATE TRIGGER prevent_usage_journal_mutation
    BEFORE UPDATE OR DELETE ON usage_journal
    FOR EACH ROW
    EXECUTE FUNCTION prevent_event_mutation();

CREATE TABLE usage_ledger (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    journal_id UUID NOT NULL REFERENCES usage_journal(id) ON DELETE CASCADE,
    budget_id UUID REFERENCES budgets(id) ON DELETE SET NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    session_id UUID REFERENCES sessions(id) ON DELETE SET NULL,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    principal_id UUID REFERENCES principals(id) ON DELETE SET NULL,
    agent_id UUID REFERENCES agents(id) ON DELETE SET NULL,
    harness_id UUID REFERENCES harnesses(id) ON DELETE SET NULL,
    currency TEXT NOT NULL,
    amount DOUBLE PRECISION NOT NULL,
    meter_source TEXT NOT NULL,
    ref_type TEXT,
    ref_id UUID,
    description TEXT,
    rating_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_usage_ledger_journal_budget_currency
    ON usage_ledger(journal_id, budget_id, currency);

CREATE INDEX idx_usage_ledger_budget_created
    ON usage_ledger(budget_id, created_at DESC)
    WHERE budget_id IS NOT NULL;
CREATE INDEX idx_usage_ledger_journal_created
    ON usage_ledger(journal_id, created_at DESC);
CREATE INDEX idx_usage_ledger_session_created
    ON usage_ledger(session_id, created_at DESC)
    WHERE session_id IS NOT NULL;
CREATE INDEX idx_usage_ledger_org_created
    ON usage_ledger(org_id, created_at DESC);

CREATE TRIGGER prevent_usage_ledger_mutation
    BEFORE UPDATE OR DELETE ON usage_ledger
    FOR EACH ROW
    EXECUTE FUNCTION prevent_event_mutation();

INSERT INTO usage_journal (
    id,
    org_id,
    kind,
    source_type,
    source_id,
    session_id,
    measures,
    metadata,
    created_at
)
SELECT
    uuidv7(),
    b.org_id,
    CASE
        WHEN bl.amount < 0 OR bl.ref_type = 'top_up' THEN 'top_up'
        ELSE 'legacy_budget_charge'
    END,
    'legacy_budget_ledger',
    bl.id::text,
    bl.session_id,
    jsonb_build_object(
        'legacy_amount', bl.amount,
        'legacy_currency', b.currency
    ),
    jsonb_build_object(
        'legacy_budget_id', bl.budget_id,
        'legacy_ref_type', bl.ref_type,
        'legacy_ref_id', bl.ref_id,
        'legacy_description', bl.description
    ),
    bl.created_at
FROM budget_ledger bl
JOIN budgets b ON b.id = bl.budget_id;

INSERT INTO usage_ledger (
    id,
    journal_id,
    budget_id,
    org_id,
    session_id,
    currency,
    amount,
    meter_source,
    ref_type,
    ref_id,
    description,
    rating_metadata,
    created_at
)
SELECT
    uuidv7(),
    uj.id,
    bl.budget_id,
    b.org_id,
    bl.session_id,
    b.currency,
    bl.amount,
    bl.meter_source,
    bl.ref_type,
    bl.ref_id,
    bl.description,
    jsonb_build_object(
        'ruleset_version', 'legacy-v1',
        'backfilled_from', 'budget_ledger'
    ),
    bl.created_at
FROM budget_ledger bl
JOIN budgets b ON b.id = bl.budget_id
JOIN usage_journal uj
    ON uj.source_type = 'legacy_budget_ledger'
   AND uj.source_id = bl.id::text;
