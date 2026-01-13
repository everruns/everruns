-- Event Store Hardening: Append-Only + Atomic Sequences + Snapshots
--
-- This migration adds:
-- 1. Trigger to enforce append-only semantics on events table
-- 2. event_sequences table for atomic per-session sequence allocation
-- 3. event_snapshots table for replay optimization

-- ============================================
-- 1. Append-Only Events Trigger
-- ============================================
-- Prevent UPDATE and DELETE on events table to ensure immutability

CREATE OR REPLACE FUNCTION prevent_event_mutation()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'events are append-only: % operations are not allowed', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER events_append_only_update
    BEFORE UPDATE ON events
    FOR EACH ROW EXECUTE FUNCTION prevent_event_mutation();

CREATE TRIGGER events_append_only_delete
    BEFORE DELETE ON events
    FOR EACH ROW EXECUTE FUNCTION prevent_event_mutation();

COMMENT ON FUNCTION prevent_event_mutation() IS 'Enforces append-only semantics on the events table';

-- ============================================
-- 2. Atomic Per-Session Sequence Allocation
-- ============================================
-- Use INSERT ... ON CONFLICT DO UPDATE for race-free sequence allocation

CREATE TABLE event_sequences (
    session_id UUID PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    next_sequence INTEGER NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE event_sequences IS 'Per-session sequence counter for atomic event sequence allocation';
COMMENT ON COLUMN event_sequences.next_sequence IS 'Next sequence number to allocate (1-indexed)';

-- Function to atomically allocate the next sequence number
CREATE OR REPLACE FUNCTION allocate_event_sequence(p_session_id UUID)
RETURNS INTEGER AS $$
DECLARE
    allocated_seq INTEGER;
BEGIN
    INSERT INTO event_sequences (session_id, next_sequence, updated_at)
    VALUES (p_session_id, 2, NOW())
    ON CONFLICT (session_id) DO UPDATE
    SET next_sequence = event_sequences.next_sequence + 1,
        updated_at = NOW()
    RETURNING next_sequence - 1 INTO allocated_seq;

    RETURN allocated_seq;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION allocate_event_sequence(UUID) IS 'Atomically allocate the next event sequence number for a session';

-- Initialize event_sequences from existing events
INSERT INTO event_sequences (session_id, next_sequence, updated_at)
SELECT session_id, MAX(sequence) + 1, NOW()
FROM events
GROUP BY session_id
ON CONFLICT (session_id) DO UPDATE
SET next_sequence = EXCLUDED.next_sequence,
    updated_at = NOW();

-- ============================================
-- 3. Event Snapshots Table
-- ============================================
-- Stores session state snapshots to speed up replay

CREATE TABLE event_snapshots (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    state JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Unique constraint: one snapshot per session/sequence combination
    UNIQUE(session_id, sequence)
);

CREATE INDEX idx_event_snapshots_session_id ON event_snapshots(session_id);
CREATE INDEX idx_event_snapshots_session_sequence ON event_snapshots(session_id, sequence DESC);

COMMENT ON TABLE event_snapshots IS 'Session state snapshots for replay optimization';
COMMENT ON COLUMN event_snapshots.sequence IS 'Event sequence number up to which this snapshot represents';
COMMENT ON COLUMN event_snapshots.schema_version IS 'Schema version for forward compatibility';
COMMENT ON COLUMN event_snapshots.state IS 'Serialized session state at this sequence';
