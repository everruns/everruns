CREATE TABLE IF NOT EXISTS trace_store_locations (
    store_name TEXT PRIMARY KEY,
    object_prefix TEXT NOT NULL UNIQUE
);

-- Analytical copies only. Intentionally retain manifests after a session is
-- deleted so the collector can remove its objects. Every query joins live
-- sessions under org scope; manifests never grant access on their own.
CREATE TABLE IF NOT EXISTS trace_store_sessions (
    store_name TEXT NOT NULL REFERENCES trace_store_locations (store_name),
    org_id BIGINT NOT NULL,
    session_id UUID NOT NULL,
    exported_sequence INTEGER NOT NULL DEFAULT 0 CHECK (exported_sequence >= 0),
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (store_name, org_id, session_id)
);

CREATE TABLE IF NOT EXISTS trace_store_segments (
    id UUID PRIMARY KEY,
    store_name TEXT NOT NULL,
    org_id BIGINT NOT NULL,
    session_id UUID NOT NULL,
    first_sequence INTEGER NOT NULL CHECK (first_sequence > 0),
    last_sequence INTEGER NOT NULL CHECK (last_sequence >= first_sequence),
    min_ts TIMESTAMPTZ NOT NULL,
    max_ts TIMESTAMPTZ NOT NULL CHECK (max_ts >= min_ts),
    row_count BIGINT NOT NULL CHECK (row_count > 0),
    index_key TEXT NOT NULL UNIQUE,
    payload_key TEXT NOT NULL UNIQUE,
    index_bytes BIGINT NOT NULL CHECK (index_bytes > 0),
    payload_bytes BIGINT NOT NULL CHECK (payload_bytes > 0),
    index_sha256 TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL,
    format_version INTEGER NOT NULL DEFAULT 1 CHECK (format_version = 1),
    retired BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (store_name, org_id, session_id)
        REFERENCES trace_store_sessions (store_name, org_id, session_id)
);
CREATE INDEX IF NOT EXISTS trace_store_segments_session ON trace_store_segments
    (store_name, org_id, session_id, first_sequence) WHERE NOT retired;
CREATE INDEX IF NOT EXISTS trace_store_segments_time ON trace_store_segments
    (store_name, org_id, min_ts) WHERE NOT retired;
