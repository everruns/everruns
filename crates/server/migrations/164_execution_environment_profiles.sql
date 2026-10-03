-- First-class Environment configuration and pinned Session snapshots.
--
-- Agent rows carry the editable head; immutable agent_versions already copy it
-- into authored_config. A Session resolves one profile into its logical
-- `sandboxes` row so later Agent edits cannot change active compute.

ALTER TABLE agents
    ADD COLUMN environments JSONB;

ALTER TABLE sandboxes
    ADD COLUMN profile_name TEXT,
    ADD COLUMN profile_snapshot JSONB,
    ADD COLUMN desired_state TEXT NOT NULL DEFAULT 'ready'
        CHECK (desired_state IN ('ready', 'paused', 'deleted')),
    ADD COLUMN observed_state TEXT NOT NULL DEFAULT 'absent'
        CHECK (observed_state IN ('absent', 'provisioning', 'ready', 'paused', 'lost', 'deleting', 'deleted', 'failed')),
    ADD COLUMN last_activity_at TIMESTAMPTZ;

-- The public model owns exactly one logical Environment per Session. The old
-- provider key remains for compatibility with pre-profile rows and driver
-- lookups, but it no longer permits two mutable compute worlds in one Session.
CREATE UNIQUE INDEX sandboxes_one_profiled_environment_per_session
    ON sandboxes (session_id)
    WHERE profile_snapshot IS NOT NULL;

COMMENT ON COLUMN agents.environments IS
    'Named authored Environment profiles. Immutable Agent versions copy this value into authored_config.';

COMMENT ON COLUMN sandboxes.profile_snapshot IS
    'Fully resolved, immutable Environment profile pinned when the owning Session is created.';

COMMENT ON COLUMN sandboxes.desired_state IS
    'Control-plane lifecycle intent. Reconciliation moves observed_state toward this value.';
