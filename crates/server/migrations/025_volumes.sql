-- Workspace Volumes (EVE-396)
-- See specs/volumes.md for full design intent.
--
-- A Volume is an org-scoped, named filesystem tree that users can mount into
-- session workspaces through the workspace_volumes capability. Volumes provide
-- shared, file-backed memory and durable data across sessions.
--
-- This migration adds the foundational schema:
--   - volumes:               the volume entity (lifecycle: active/archived/deleted)
--   - volume_files:          files and directories inside a volume (mirrors session_files)
--   - session_volume_mounts: per-session snapshot of resolved mount config
--
-- Volume CRUD APIs, filesystem APIs, mount resolution at session creation,
-- read/write integration with session_file_system tools, and UI ship as
-- follow-up vertical slices on top of this foundation.

-- ============================================
-- Volumes (org-scoped named filesystem trees)
-- ============================================

CREATE TABLE volumes (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    -- Dual-ID pattern: internal UUID (id) + external public_id (API-facing)
    public_id TEXT NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    -- Owner principal (creator). String column to match the pattern used elsewhere
    -- for principal references; resolution to a concrete user happens at the
    -- domain layer.
    owner_principal_id TEXT,
    resolved_owner_user_id UUID,
    status VARCHAR(50) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,

    -- Format validation: vol_ prefix + 32 lowercase hex chars
    CONSTRAINT volumes_public_id_format
        CHECK (public_id ~ '^vol_[0-9a-f]{32}$')
);

CREATE INDEX idx_volumes_org_id ON volumes(org_id);
CREATE INDEX idx_volumes_status ON volumes(status);
-- Unique per org (same public_id allowed across orgs)
CREATE UNIQUE INDEX idx_volumes_org_public_id ON volumes(org_id, public_id);
-- Unique name per org while not deleted (tombstones may share names with new rows)
CREATE UNIQUE INDEX idx_volumes_org_name_active
    ON volumes(org_id, name)
    WHERE status != 'deleted';

CREATE TRIGGER update_volumes_updated_at BEFORE UPDATE ON volumes
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE volumes IS 'Workspace volumes — org-scoped named filesystem trees. See specs/volumes.md.';

-- ============================================
-- Volume Files (mirrors session_files shape)
-- ============================================

CREATE TABLE volume_files (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    volume_id UUID NOT NULL REFERENCES volumes(id) ON DELETE CASCADE,

    -- File path (normalized: starts with /, no trailing slash, forward slashes only)
    path TEXT NOT NULL,

    -- Content (NULL for directories)
    content BYTEA,

    -- File type
    is_directory BOOLEAN NOT NULL DEFAULT FALSE,

    -- Metadata
    size_bytes BIGINT NOT NULL DEFAULT 0,
    -- Optional content hash (sha256:... for stale-edit protection)
    content_hash TEXT,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Constraints (mirror session_files validation exactly)
    CONSTRAINT volume_files_path_check CHECK (
        path ~ '^/([^/\0]+(/[^/\0]+)*)?$'
    ),
    CONSTRAINT volume_files_directory_no_content CHECK (
        NOT is_directory OR content IS NULL
    )
);

-- Unique path per volume
CREATE UNIQUE INDEX idx_volume_files_path ON volume_files(volume_id, path);

-- For listing directory contents (parent path lookup)
CREATE INDEX idx_volume_files_parent
    ON volume_files(volume_id, (substring(path from '^(.*)/[^/]+$')));

-- For volume cleanup
CREATE INDEX idx_volume_files_volume_id ON volume_files(volume_id);

-- For searching by name pattern
CREATE INDEX idx_volume_files_name
    ON volume_files(volume_id, (substring(path from '[^/]+$')));

-- Auto-update updated_at
CREATE TRIGGER update_volume_files_updated_at
    BEFORE UPDATE ON volume_files
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE volume_files IS 'Files and directories inside a workspace volume. Path validation mirrors session_files.';

-- ============================================
-- Session Volume Mounts (per-session snapshot)
-- ============================================
-- Snapshot of mount configuration at session creation time. Runtime behavior
-- stays stable even if the agent or harness config changes after session
-- start, and volume archival/deletion is handled gracefully against this
-- snapshot. The volume_id FK does not cascade-delete: when a Volume is hard
-- deleted, mounts must surface a clear error rather than silently disappearing.

CREATE TABLE session_volume_mounts (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    volume_id UUID NOT NULL REFERENCES volumes(id),

    -- Normalized mount path under /workspace
    mount_path TEXT NOT NULL,

    -- Access mode (default: readonly)
    access VARCHAR(16) NOT NULL DEFAULT 'readonly'
        CHECK (access IN ('readonly', 'readwrite')),

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT session_volume_mounts_path_check CHECK (
        mount_path ~ '^/workspace(/[^/\0]+)*$'
    )
);

-- Unique mount path per session — overlapping mounts are rejected at the
-- application layer; the unique index also catches duplicate exact paths.
CREATE UNIQUE INDEX idx_session_volume_mounts_path
    ON session_volume_mounts(session_id, mount_path);

CREATE INDEX idx_session_volume_mounts_session_id
    ON session_volume_mounts(session_id);

CREATE INDEX idx_session_volume_mounts_volume_id
    ON session_volume_mounts(volume_id);

COMMENT ON TABLE session_volume_mounts IS 'Per-session snapshot of resolved workspace_volumes capability mounts.';
