-- system_compaction_observers
-- System assessments remain org-scoped observer scores. Explicit consent is
-- stored independently of the observer lifecycle and checked at dispatch.
ALTER TABLE organization_settings ADD COLUMN quality_monitoring jsonb;
ALTER TABLE observers ADD COLUMN system_kind text
    CHECK (system_kind IS NULL OR system_kind = 'compaction');
CREATE UNIQUE INDEX observers_system_kind ON observers (org_id, system_kind)
    WHERE system_kind IS NOT NULL;
ALTER TABLE trace_scores DROP CONSTRAINT trace_scores_scope_check;
ALTER TABLE trace_scores ADD CONSTRAINT trace_scores_scope_check
    CHECK (scope IN ('turn', 'session', 'tool', 'compaction'));
ALTER TABLE trace_scores ADD COLUMN evidence_ref jsonb;

-- Unknown/failed calls retain their reservation. A retry reserves separately;
-- a crashed worker cannot give another caller the same spending allowance.
CREATE TABLE observer_budget_reservations (
    score_id uuid NOT NULL REFERENCES trace_scores(id) ON DELETE CASCADE,
    attempt integer NOT NULL CHECK (attempt > 0),
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    day date NOT NULL DEFAULT ((now() AT TIME ZONE 'UTC')::date),
    reserved_micros bigint NOT NULL CHECK (reserved_micros >= 0),
    actual_micros bigint CHECK (actual_micros >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (score_id, attempt)
);
CREATE INDEX observer_budget_org_day ON observer_budget_reservations (org_id, day);
