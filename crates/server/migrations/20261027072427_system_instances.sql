-- Running processes of the install that are not workers (knowledge/operations/system-doctor.md).
-- Each server process writes its row every 30 seconds, so a doctor run on any
-- replica sees every replica. Rows are kept after a process stops: a stale
-- row is shown as gone, never deleted. `kind` leaves room for other process
-- types (an egress proxy) without a schema change.
CREATE TABLE system_instances (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    hostname TEXT,
    version TEXT NOT NULL,
    -- Newest migration this process was built with.
    schema_version BIGINT,
    -- Settings other replicas compare against, e.g. whether NATS is used.
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    started_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_system_instances_kind_last_seen ON system_instances (kind, last_seen_at DESC);
