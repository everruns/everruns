-- Workspaces — promote the per-session virtual filesystem to a first-class
-- org-scoped entity. See specs/workspace.md.
--
-- This migration:
--   1. Creates the `workspaces` table with the standard building-block
--      lifecycle (active/archived/deleted) and a dual-ID (`wsp_<32-hex>`)
--      public_id.
--   2. Adds `sessions.workspace_id` and backfills it: every existing
--      session gets its own freshly minted Workspace, named after the
--      session, with the session re-pointed at that workspace.
--   3. Renames `session_files` → `workspace_files`, the FK column
--      `session_id` → `workspace_id`, and re-points the FK to
--      `workspaces(id) ON DELETE CASCADE`. All other shape constraints,
--      indexes, and triggers on session_files are preserved verbatim.
--
-- Runs as a single transaction; either the rename completes end-to-end or
-- the database is left untouched.

BEGIN;

-- ============================================
-- 1. Workspaces (org-scoped named working areas)
-- ============================================

CREATE TABLE workspaces (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    public_id TEXT NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    owner_principal_id TEXT,
    resolved_owner_user_id UUID,
    status VARCHAR(50) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,

    CONSTRAINT workspaces_public_id_format
        CHECK (public_id ~ '^wsp_[0-9a-f]{32}$')
);

CREATE INDEX idx_workspaces_org_id ON workspaces(org_id);
CREATE INDEX idx_workspaces_status ON workspaces(status);
CREATE UNIQUE INDEX idx_workspaces_org_public_id ON workspaces(org_id, public_id);
CREATE UNIQUE INDEX idx_workspaces_public_id ON workspaces(public_id);
CREATE UNIQUE INDEX idx_workspaces_org_name_active
    ON workspaces(org_id, name)
    WHERE status != 'deleted';
CREATE UNIQUE INDEX idx_workspaces_org_lower_name_active
    ON workspaces(org_id, LOWER(name))
    WHERE status != 'deleted';

CREATE TRIGGER update_workspaces_updated_at BEFORE UPDATE ON workspaces
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE workspaces IS 'Org-scoped named working areas. See specs/workspace.md.';

-- ============================================
-- 2. Backfill: one Workspace per existing session, with matching UUIDs
-- ============================================
--
-- For each existing session, INSERT a Workspace whose internal `id` UUID is
-- *the same as the session's UUID*. That keeps every existing FK reference
-- in `workspace_files` (formerly keyed by session_id, now by workspace_id)
-- pointing at the same Uuid value — no per-row backfill needed for
-- workspace_files, and existing application code that uses `session.id` as
-- the file-store key keeps working unchanged.
--
-- Sessions that later attach to a shared Workspace get a different UUID;
-- only the default case has session.id == workspace.id.

ALTER TABLE sessions ADD COLUMN workspace_id UUID REFERENCES workspaces(id);

INSERT INTO workspaces (
    id, org_id, public_id, name, description, status, created_at, updated_at
)
SELECT
    s.id,
    s.org_id,
    'wsp_' || replace(s.id::text, '-', ''),
    'session-' || substring(s.id::text from 1 for 8),
    'Default workspace for session ' || s.id::text,
    'active',
    s.created_at,
    NOW()
FROM sessions s
WHERE s.workspace_id IS NULL;

-- Each session's workspace_id is its own id (matching UUIDs).
UPDATE sessions SET workspace_id = id WHERE workspace_id IS NULL;

-- Enforce NOT NULL now that every existing row is backfilled.
ALTER TABLE sessions ALTER COLUMN workspace_id SET NOT NULL;

-- Helpful index for session-by-workspace lookups (used by the alias routes).
CREATE INDEX idx_sessions_workspace_id ON sessions(workspace_id);

-- ============================================
-- 3. Rename session_files → workspace_files
-- ============================================

ALTER TABLE session_files RENAME TO workspace_files;
ALTER TABLE workspace_files RENAME COLUMN session_id TO workspace_id;

-- The FK currently references sessions(id) ON DELETE CASCADE. Drop the
-- session FK and add the workspace FK with the same cascade behavior.
ALTER TABLE workspace_files
    DROP CONSTRAINT IF EXISTS session_files_session_id_fkey;
ALTER TABLE workspace_files
    ADD CONSTRAINT workspace_files_workspace_id_fkey
        FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE;

-- No per-row backfill needed: the column was renamed in place, and each
-- session's workspace.id is set to equal its session.id, so existing FK
-- values continue to point at the right workspace row.

-- Rename indexes that embed the old name.
ALTER INDEX idx_session_files_path RENAME TO idx_workspace_files_path;
ALTER INDEX idx_session_files_parent RENAME TO idx_workspace_files_parent;
ALTER INDEX idx_session_files_session_id RENAME TO idx_workspace_files_workspace_id;
ALTER INDEX idx_session_files_name RENAME TO idx_workspace_files_name;

-- Rename trigger + check constraints.
ALTER TRIGGER update_session_files_updated_at ON workspace_files
    RENAME TO update_workspace_files_updated_at;
ALTER TABLE workspace_files
    RENAME CONSTRAINT session_files_path_check TO workspace_files_path_check;
ALTER TABLE workspace_files
    RENAME CONSTRAINT session_files_directory_no_content
        TO workspace_files_directory_no_content;

COMMENT ON TABLE workspace_files IS 'Files and directories inside a Workspace. Path validation mirrors the original session_files shape.';

COMMIT;
