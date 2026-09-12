-- Files table for model-input file attachments (e.g. PDFs).
-- Mirrors the images table minus thumbnails: files are always inlined as
-- BYTEA (no object-store offload, no thumbnails). Prompt-attached files are
-- size-capped at upload, so inline storage is sufficient.
CREATE TABLE files (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id BIGINT NOT NULL,
    filename TEXT,
    content_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    data BYTEA NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_files_org_id ON files(org_id);
