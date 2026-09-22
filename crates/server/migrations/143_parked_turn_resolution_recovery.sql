ALTER TABLE sessions
    DROP CONSTRAINT sessions_status_check;

ALTER TABLE sessions
    ADD CONSTRAINT sessions_status_check CHECK (
        status IN (
            'started',
            'active',
            'idle',
            'waiting_for_tool_results',
            'resolving_tool_results',
            'paused'
        )
    );

ALTER TABLE sessions
    ADD COLUMN turn_resolution_id UUID,
    ADD COLUMN turn_resolution_claim_token UUID,
    ADD COLUMN turn_resolution_lease_expires_at TIMESTAMPTZ,
    ADD COLUMN turn_resolution_plan JSONB;

ALTER TABLE events
    ADD COLUMN turn_resolution_id UUID,
    ADD COLUMN turn_resolution_event_index INT;

CREATE UNIQUE INDEX idx_events_turn_resolution
    ON events(turn_resolution_id, turn_resolution_event_index)
    WHERE turn_resolution_id IS NOT NULL;

CREATE UNIQUE INDEX idx_durable_task_queue_waiting_turn_resolution
    ON durable_task_queue(workflow_id, activity_id)
    WHERE workflow_id IS NOT NULL
      AND activity_id LIKE 'waiting_turn_resolution_%';
