-- Session Git Objects & Refs (Option C: libgit2 custom ODB/refdb over PostgreSQL)
--
-- Strategy: Session-scoped objects. Each session has its own isolated git object
-- store and ref namespace. No cross-session deduplication.

CREATE TABLE session_git_objects (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    oid BYTEA NOT NULL,           -- 20-byte SHA1 (git's native hash)
    obj_type SMALLINT NOT NULL,   -- 1=commit, 2=tree, 3=blob, 4=tag
    size BIGINT NOT NULL,         -- uncompressed object size
    data BYTEA NOT NULL,          -- raw object content
    PRIMARY KEY (session_id, oid),
    -- Validate OID length and obj_type range
    CONSTRAINT chk_git_object_oid_len CHECK (octet_length(oid) = 20),
    CONSTRAINT chk_git_object_type CHECK (obj_type IN (1, 2, 3, 4))
);

-- Index for listing all objects in a session (cleanup, fork)
CREATE INDEX idx_session_git_objects_session
    ON session_git_objects (session_id);

CREATE TABLE session_git_refs (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    name TEXT NOT NULL,            -- e.g. "refs/heads/main", "HEAD"
    target BYTEA NOT NULL,         -- 20-byte oid (direct ref) or symbolic target
    is_symbolic BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (session_id, name),
    -- Validate target length for direct (non-symbolic) refs
    CONSTRAINT chk_git_ref_target_len CHECK (is_symbolic = true OR octet_length(target) = 20)
);

-- Index for listing all refs in a session
CREATE INDEX idx_session_git_refs_session
    ON session_git_refs (session_id);
