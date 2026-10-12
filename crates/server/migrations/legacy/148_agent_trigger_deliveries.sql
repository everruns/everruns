-- Agent trigger deliveries: one row per event that reached a trigger.
--
-- The trigger event pipeline records every event it receives (schedule fire,
-- webhook request, GitHub delivery) with its outcome, so a user can see why an
-- event did or did not start a run. `event_id` is the source's idempotency key:
-- the partial unique index makes a redelivered event a recorded duplicate
-- instead of a second run. Rows are pruned per trigger to a bounded history by
-- the pipeline itself (see domains/agent_triggers/events.rs).

CREATE TABLE agent_trigger_deliveries (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    trigger_id UUID NOT NULL REFERENCES agent_triggers(id) ON DELETE CASCADE,
    source VARCHAR(50) NOT NULL,
    event_id TEXT,
    event_type TEXT,
    subject TEXT,
    status VARCHAR(20) NOT NULL
        CHECK (status IN ('dispatched', 'filtered', 'duplicate', 'failed')),
    reason TEXT,
    session_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_trigger_deliveries_trigger_created
    ON agent_trigger_deliveries (trigger_id, created_at DESC);

-- Dispatched and filtered rows claim their event id. Duplicates are logged
-- beside the claim, and a failed row releases it so a source retry can run.
CREATE UNIQUE INDEX agent_trigger_deliveries_event_id_idx
    ON agent_trigger_deliveries (trigger_id, event_id)
    WHERE event_id IS NOT NULL AND status IN ('dispatched', 'filtered');
