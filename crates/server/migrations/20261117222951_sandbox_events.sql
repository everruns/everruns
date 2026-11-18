-- Sandbox events: who changed a Sandbox, or decided not to, and why.
--
-- `sandbox_state_transitions` (trigger-written) says what state a Sandbox
-- was in and when. It cannot say who moved it or why, and a decision to do
-- nothing leaves no row at all. This table holds those causes: the idle
-- timer, the idle sweep, the retention sweep and people acting through the
-- API record each action, failure, and each decision not to act. Kept for
-- as long as the Sandbox record (records are never purged).
CREATE TABLE sandbox_events (
    id bigserial PRIMARY KEY,
    sandbox_id uuid NOT NULL REFERENCES sandboxes(id) ON DELETE CASCADE,
    org_id bigint NOT NULL,
    generation bigint NOT NULL,
    at timestamptz NOT NULL DEFAULT now(),
    actor text NOT NULL,
    action text NOT NULL,
    outcome text NOT NULL,
    reason text NOT NULL,
    message text NOT NULL,
    detail jsonb NOT NULL DEFAULT '{}'::jsonb,
    CONSTRAINT sandbox_events_outcome_check CHECK (outcome IN ('done', 'skipped', 'failed'))
);

COMMENT ON TABLE sandbox_events IS 'Causes of Sandbox lifecycle changes and of decisions not to change: actor, action, outcome, reason code and a plain sentence. Free text is secret-scrubbed before insert.';
COMMENT ON COLUMN sandbox_events.actor IS 'idle_timer, idle_sweep, retention_sweep, person (API) or provider.';
COMMENT ON COLUMN sandbox_events.reason IS 'Stable reason code, such as idle_limit, stale_turn, session_active, idle_pause_off, retention, idle_action_delete.';

CREATE INDEX sandbox_events_sandbox_at_idx ON sandbox_events (sandbox_id, at DESC, id DESC);
CREATE INDEX sandbox_events_org_at_idx ON sandbox_events (org_id, at DESC);
