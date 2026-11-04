-- Qualification metadata contains boundaries, counters and fixed error codes,
-- never copied payloads or diffs. No canonical event table is modified.
CREATE TABLE IF NOT EXISTS trace_store_backfills (
    id UUID PRIMARY KEY,
    store_name TEXT NOT NULL REFERENCES trace_store_locations(store_name),
    org_id BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS trace_store_backfill_targets (
    run_id UUID NOT NULL REFERENCES trace_store_backfills(id) ON DELETE CASCADE,
    session_id UUID NOT NULL,
    target_sequence INTEGER NOT NULL CHECK(target_sequence > 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','copied','skipped','failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    error_code TEXT CHECK(error_code IS NULL OR error_code = 'export_failed'),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(run_id,session_id)
);
CREATE INDEX IF NOT EXISTS trace_store_backfill_pending ON trace_store_backfill_targets(run_id,status,updated_at);

CREATE TABLE IF NOT EXISTS trace_store_shadow_checks (
    store_name TEXT NOT NULL REFERENCES trace_store_locations(store_name),
    org_id BIGINT NOT NULL,
    session_id UUID NOT NULL,
    next_sequence INTEGER NOT NULL DEFAULT 0,
    checked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    report JSONB NOT NULL,
    PRIMARY KEY(store_name,org_id,session_id)
);
