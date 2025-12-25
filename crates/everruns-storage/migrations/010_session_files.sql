-- Session Virtual Filesystem
-- Decision: Files are stored per-session, deleted with session (ON DELETE CASCADE)
-- Decision: Content stored as BYTEA for both text and binary files
-- Decision: Path is normalized (no trailing slashes, forward slashes only)
-- Decision: Root directory "/" always exists implicitly (no stored entry)

-- ============================================
-- Session Files Table
-- ============================================

CREATE TABLE session_files (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,

    -- File path (normalized: starts with /, no trailing slash, forward slashes only)
    path TEXT NOT NULL,

    -- Content (NULL for directories)
    content BYTEA,

    -- File type
    is_directory BOOLEAN NOT NULL DEFAULT FALSE,

    -- Metadata
    is_readonly BOOLEAN NOT NULL DEFAULT FALSE,
    size_bytes BIGINT NOT NULL DEFAULT 0,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Constraints
    CONSTRAINT session_files_path_check CHECK (
        path ~ '^/([^/\0]+(/[^/\0]+)*)?$' -- Valid path format
    ),
    CONSTRAINT session_files_directory_no_content CHECK (
        NOT is_directory OR content IS NULL -- Directories cannot have content
    )
);

-- Unique path per session
CREATE UNIQUE INDEX idx_session_files_path ON session_files(session_id, path);

-- For listing directory contents (parent path lookup)
CREATE INDEX idx_session_files_parent ON session_files(session_id, (substring(path from '^(.*)/[^/]+$')));

-- For session cleanup
CREATE INDEX idx_session_files_session_id ON session_files(session_id);

-- For searching by name pattern
CREATE INDEX idx_session_files_name ON session_files(session_id, (substring(path from '[^/]+$')));

-- Auto-update updated_at
CREATE TRIGGER update_session_files_updated_at
    BEFORE UPDATE ON session_files
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Helper Functions
-- ============================================

-- Get parent directory path
CREATE OR REPLACE FUNCTION session_files_parent_path(file_path TEXT)
RETURNS TEXT AS $$
BEGIN
    IF file_path = '/' THEN
        RETURN NULL;
    END IF;
    RETURN COALESCE(substring(file_path from '^(.*)/[^/]+$'), '/');
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Get file name from path
CREATE OR REPLACE FUNCTION session_files_name(file_path TEXT)
RETURNS TEXT AS $$
BEGIN
    IF file_path = '/' THEN
        RETURN '/';
    END IF;
    RETURN substring(file_path from '[^/]+$');
END;
$$ LANGUAGE plpgsql IMMUTABLE;
