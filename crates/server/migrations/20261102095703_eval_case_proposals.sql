-- Evals rebuild step 8: the Eval Builder.
--
-- A run is one request to propose cases for an agent. It reads the agent's
-- configuration, a sample of its recent sessions (those with findings or low
-- observer scores first) and the cases its datasets already have, and writes
-- proposals. A proposal is one suggested case with the reason for it and the
-- sessions that inspired it. Nothing becomes a case until a person accepts it
-- into a dataset; dismissing keeps the row (and its note) so the builder does
-- not offer the same idea again.

CREATE TABLE eval_builder_runs (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'running',
    requested_by uuid,
    started_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    model text,
    cost_usd double precision,
    sessions_read integer NOT NULL DEFAULT 0,
    proposals integer NOT NULL DEFAULT 0,
    error text,
    CONSTRAINT eval_builder_runs_status_check
        CHECK (status IN ('running', 'done', 'failed', 'unavailable'))
);

CREATE INDEX idx_eval_builder_runs_agent ON eval_builder_runs (agent_id, started_at DESC);

CREATE TABLE eval_case_proposals (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES eval_builder_runs(id) ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'proposed',
    name text NOT NULL,
    category text NOT NULL,
    reason text NOT NULL,
    source_session_ids uuid[] NOT NULL DEFAULT '{}',
    -- The case as it would be created: a `CreateEvalCaseRequest`.
    "case" jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    decided_at timestamptz,
    decided_by uuid,
    eval_id uuid,
    eval_case_id uuid,
    dismiss_note text,
    CONSTRAINT eval_case_proposals_status_check
        CHECK (status IN ('proposed', 'accepted', 'dismissed')),
    CONSTRAINT eval_case_proposals_category_check
        CHECK (category IN ('happy_path', 'edge_case', 'failure', 'adversarial'))
);

CREATE INDEX idx_eval_case_proposals_agent ON eval_case_proposals (agent_id, status, created_at DESC);
