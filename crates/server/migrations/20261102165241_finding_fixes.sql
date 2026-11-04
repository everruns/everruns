-- Evals rebuild step 10: suggested fixes.
--
-- A fix is what the Inspector proposes for a finding about an agent's work:
-- text to add to the agent's instructions. It is checked by running one of
-- the agent's evals with and without the change, and applied as a new agent
-- revision only when a person approves it after a check with no regressions.
--
-- Rows are kept: an applied or rejected fix keeps its check and the reason,
-- so later proposals can be compared with what people accepted.

CREATE TABLE finding_fixes (
    id uuid PRIMARY KEY,
    public_id text NOT NULL UNIQUE
        CONSTRAINT finding_fixes_public_id_format CHECK (public_id ~ '^fix_[0-9a-f]{32}$'),
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    health_issue_id uuid NOT NULL REFERENCES health_issues(id) ON DELETE CASCADE,
    episode_id uuid NOT NULL,
    agent_id uuid NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    summary text NOT NULL,
    addition text NOT NULL,
    -- The agent's instructions when the fix was written, as a SHA-256 hex
    -- digest: a fix written against other instructions is not applied.
    base_instructions_sha256 text NOT NULL,
    status text NOT NULL DEFAULT 'proposed',
    experiment_id uuid REFERENCES eval_experiments(id) ON DELETE SET NULL,
    check_result jsonb,
    edited_by uuid,
    decided_by uuid,
    decided_at timestamptz,
    decision_note text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT finding_fixes_status_check CHECK (status IN (
        'proposed', 'checking', 'verified', 'regressed', 'check_failed',
        'applied', 'rejected'
    )),
    CONSTRAINT finding_fixes_episode_unique UNIQUE (health_issue_id, episode_id)
);

CREATE INDEX idx_finding_fixes_agent ON finding_fixes (org_id, agent_id, created_at DESC);
