-- Evals rebuild step 11: self-healing policy.
--
-- An org decides, per kind of action, whether Everruns takes it on its own
-- when a finding opens (auto), offers it on the finding for a person to take
-- (suggest), or does neither (off). The policy lives in one JSONB column on
-- organization_settings, like background_limits; NULL means the code
-- defaults.
--
-- finding_actions records every action taken on a finding, automatic or by a
-- person, and its undo. Rows are kept: they are the record people read when
-- they ask why a schedule stopped.

ALTER TABLE organization_settings ADD COLUMN self_healing jsonb;

CREATE TABLE finding_actions (
    id uuid PRIMARY KEY,
    public_id text NOT NULL UNIQUE
        CONSTRAINT finding_actions_public_id_format CHECK (public_id ~ '^fact_[0-9a-f]{32}$'),
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    health_issue_id uuid NOT NULL REFERENCES health_issues(id) ON DELETE CASCADE,
    episode_id uuid NOT NULL,
    kind text NOT NULL,
    -- What the action acted on, by public id (a schedule's `sched_…`).
    target_id text NOT NULL,
    -- NULL when Everruns took the action on its own under the org's policy.
    taken_by uuid,
    undone_by uuid,
    undone_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT finding_actions_kind_check CHECK (kind IN ('pause_schedule')),
    CONSTRAINT finding_actions_episode_unique UNIQUE (health_issue_id, episode_id, kind)
);

CREATE INDEX idx_finding_actions_org ON finding_actions (org_id, created_at DESC);
