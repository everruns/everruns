-- sandbox_template_builds
-- Build inputs and results are distinct from immutable template revisions.
CREATE TABLE sandbox_template_builds (
    id uuid PRIMARY KEY,
    org_id bigint NOT NULL REFERENCES organizations(org_id),
    sandbox_template_id uuid NOT NULL REFERENCES execution_environments(id),
    status text NOT NULL CHECK (status IN ('queued','running','ready','failed','cancelled','applied','discarded')),
    cleanup_pending boolean NOT NULL DEFAULT false,
    record jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX sandbox_template_builds_template_idx ON sandbox_template_builds(org_id,sandbox_template_id,created_at DESC);
CREATE INDEX sandbox_template_builds_pending_idx ON sandbox_template_builds(updated_at)
    WHERE status IN ('queued','running') OR cleanup_pending;
-- Bound billed concurrency per tenant, including two simultaneous API requests.
CREATE UNIQUE INDEX sandbox_template_builds_one_active_org ON sandbox_template_builds(org_id)
    WHERE status IN ('queued','running');

-- Exact immutable artifact/account lookup survives copying a complete spec.
CREATE INDEX sandbox_template_builds_artifact_idx ON sandbox_template_builds
    (org_id,(record->>'snapshot_id'),(record#>>'{source_spec,target,credential,connection_id}'))
    WHERE status='applied';

CREATE INDEX sandbox_template_builds_history_retention_idx ON sandbox_template_builds(created_at)
    WHERE status IN ('failed','cancelled','discarded') AND NOT cleanup_pending;

CREATE INDEX sandbox_template_builds_account_idx ON sandbox_template_builds
    ((record#>>'{source_spec,target,credential,connection_id}'))
    WHERE status IN ('queued','running','ready') OR cleanup_pending;
