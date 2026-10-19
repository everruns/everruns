-- Durable canonical model-input replacements. Raw session events remain unchanged.
CREATE TABLE session_compaction_checkpoints (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    source_sequence INTEGER NOT NULL CHECK (source_sequence >= 0),
    provider_type TEXT NOT NULL,
    model TEXT NOT NULL,
    format_version INTEGER NOT NULL CHECK (format_version > 0),
    payload_encrypted BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, provider_type, model, format_version)
);

CREATE INDEX idx_session_compaction_checkpoints_session
    ON session_compaction_checkpoints (session_id, source_sequence DESC);
