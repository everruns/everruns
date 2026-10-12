-- Bound periodic reporting repair to an indexed slice of supported events.
-- Full historical reconciliation remains available through the admin backfill.

CREATE TABLE reporting_event_repair_cursor (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    last_event_created_at TIMESTAMPTZ,
    last_event_id UUID,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (last_event_created_at IS NULL AND last_event_id IS NULL)
        OR (last_event_created_at IS NOT NULL AND last_event_id IS NOT NULL)
    )
);

INSERT INTO reporting_event_repair_cursor (singleton) VALUES (TRUE);

CREATE INDEX idx_events_reporting_repair
    ON events(created_at, id)
    WHERE event_type IN (
        'tool.completed',
        'capability.usage',
        'output.message.replaced',
        'turn.completed',
        'turn.failed',
        'turn.cancelled'
    );

COMMENT ON TABLE reporting_event_repair_cursor IS
    'Global watermark for bounded, cyclic verification of reporting event projections';
COMMENT ON INDEX idx_events_reporting_repair IS
    'Partial ordered index for bounded reporting event projection repair';
