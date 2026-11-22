-- sandbox_provider_status_checks
-- Provider polling has its own clock. A read must never make an idle sandbox
-- look recently used, and a failed provider must not monopolize the batch.
ALTER TABLE sandboxes ADD COLUMN provider_status_checked_at TIMESTAMPTZ;
ALTER TABLE sandboxes ADD COLUMN provider_status_observed_at TIMESTAMPTZ;
ALTER TABLE sandboxes ADD COLUMN provider_status_external_id TEXT;
ALTER TABLE sandboxes ADD COLUMN provider_status_observed_state TEXT
    CHECK (provider_status_observed_state IN ('ready', 'paused', 'lost'));

CREATE INDEX sandboxes_provider_status_due
    ON sandboxes (provider_status_checked_at ASC NULLS FIRST, id)
    WHERE deleted_at IS NULL AND role = 'primary'
      AND profile_snapshot->'target'->>'kind' = 'managed';
