-- Evals rebuild step 7: the Inspector (detection layer 3).
--
-- An investigation is the Inspector's write-up of one finding episode: what
-- happened, how serious it is, what to do, and which cluster of similar
-- findings it belongs to. A cluster groups findings of one kind in one org
-- that describe the same problem, so ten copies show as one.
--
-- Rows are kept: a resolved finding keeps its write-up, and a dismissal keeps
-- its reason so detectors can be calibrated against it later.

CREATE TABLE finding_clusters (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    code text NOT NULL,
    title text NOT NULL,
    first_seen_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_finding_clusters_org_code ON finding_clusters (org_id, code, last_seen_at DESC);

CREATE TABLE finding_investigations (
    health_issue_id uuid NOT NULL REFERENCES health_issues(id) ON DELETE CASCADE,
    episode_id uuid NOT NULL,
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'running',
    attempts integer NOT NULL DEFAULT 1,
    claimed_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    what_happened text,
    severity text,
    suggested_action text,
    cluster_id uuid REFERENCES finding_clusters(id) ON DELETE SET NULL,
    model text,
    cost_usd double precision,
    error text,
    dismissed_at timestamptz,
    dismissed_by uuid,
    dismiss_reason text,
    PRIMARY KEY (health_issue_id, episode_id),
    CONSTRAINT finding_investigations_status_check
        CHECK (status IN ('running', 'done', 'failed', 'unavailable')),
    CONSTRAINT finding_investigations_severity_check
        CHECK (severity IS NULL OR severity IN ('high', 'medium', 'low'))
);

CREATE INDEX idx_finding_investigations_cluster ON finding_investigations (cluster_id)
    WHERE cluster_id IS NOT NULL;
CREATE INDEX idx_finding_investigations_org ON finding_investigations (org_id, claimed_at DESC);
