-- Rename Workspace Volume to Memory; drop legacy memory_stores capability.
-- See specs/memory.md (durable design) and docs/advanced/memory-model.md.
--
-- This migration performs three independent jobs:
--   1. Drop the legacy `memory_stores`/`memories` (recall) tables and
--      everything that depended on them. The persistent_memory capability
--      (remember/recall/forget) is removed; rows there are not preserved.
--   2. Rename the `volumes`/`volume_files`/`session_volume_mounts` tables to
--      `memories`/`memory_files`/`session_memory_mounts`, plus the
--      `volume_id` FK column and all indexes, triggers, and constraints whose
--      names embedded the old word.
--   3. Rewrite public IDs from the `vol_` prefix to `mem_` so existing rows
--      surface under the new ID schema.

BEGIN;

-- ============================================
-- 1. Drop legacy memory_stores (the recall feature)
-- ============================================

DROP TABLE IF EXISTS memories CASCADE;
DROP TABLE IF EXISTS memory_stores CASCADE;

-- ============================================
-- 2. Rename tables, column, indexes, triggers, constraints
-- ============================================

ALTER TABLE volumes              RENAME TO memories;
ALTER TABLE volume_files         RENAME TO memory_files;
ALTER TABLE session_volume_mounts RENAME TO session_memory_mounts;

ALTER TABLE memory_files          RENAME COLUMN volume_id TO memory_id;
ALTER TABLE session_memory_mounts RENAME COLUMN volume_id TO memory_id;

-- memories indexes
ALTER INDEX idx_volumes_org_id              RENAME TO idx_memories_org_id;
ALTER INDEX idx_volumes_status              RENAME TO idx_memories_status;
ALTER INDEX idx_volumes_org_public_id       RENAME TO idx_memories_org_public_id;
ALTER INDEX idx_volumes_org_name_active     RENAME TO idx_memories_org_name_active;
ALTER INDEX idx_volumes_org_lower_name_active RENAME TO idx_memories_org_lower_name_active;
ALTER INDEX idx_volumes_public_id           RENAME TO idx_memories_public_id;
ALTER INDEX idx_volumes_source_type         RENAME TO idx_memories_source_type;
ALTER INDEX idx_volumes_sync_status         RENAME TO idx_memories_sync_status;

-- memory_files indexes
ALTER INDEX idx_volume_files_path      RENAME TO idx_memory_files_path;
ALTER INDEX idx_volume_files_parent    RENAME TO idx_memory_files_parent;
ALTER INDEX idx_volume_files_volume_id RENAME TO idx_memory_files_memory_id;
ALTER INDEX idx_volume_files_name      RENAME TO idx_memory_files_name;

-- session_memory_mounts indexes
ALTER INDEX idx_session_volume_mounts_path       RENAME TO idx_session_memory_mounts_path;
ALTER INDEX idx_session_volume_mounts_session_id RENAME TO idx_session_memory_mounts_session_id;
ALTER INDEX idx_session_volume_mounts_volume_id  RENAME TO idx_session_memory_mounts_memory_id;

-- Triggers
ALTER TRIGGER update_volumes_updated_at ON memories
    RENAME TO update_memories_updated_at;
ALTER TRIGGER update_volume_files_updated_at ON memory_files
    RENAME TO update_memory_files_updated_at;

-- Named CHECK constraints
ALTER TABLE memories             RENAME CONSTRAINT volumes_public_id_format       TO memories_public_id_format;
ALTER TABLE memories             RENAME CONSTRAINT volumes_manual_source_shape    TO memories_manual_source_shape;
ALTER TABLE memories             RENAME CONSTRAINT volumes_source_readonly        TO memories_source_readonly;
ALTER TABLE memory_files         RENAME CONSTRAINT volume_files_path_check        TO memory_files_path_check;
ALTER TABLE memory_files         RENAME CONSTRAINT volume_files_directory_no_content TO memory_files_directory_no_content;
ALTER TABLE session_memory_mounts RENAME CONSTRAINT session_volume_mounts_path_check TO session_memory_mounts_path_check;

-- ============================================
-- 3. Rewrite public IDs: vol_* → mem_*
-- ============================================
-- The old CHECK constraint accepted ^vol_[0-9a-f]{32}$. Drop it, rewrite the
-- existing rows, and reinstate the new CHECK that accepts ^mem_[0-9a-f]{32}$.

ALTER TABLE memories DROP CONSTRAINT memories_public_id_format;

UPDATE memories
SET public_id = 'mem_' || substr(public_id, 5)
WHERE public_id LIKE 'vol\_%' ESCAPE '\';

ALTER TABLE memories ADD CONSTRAINT memories_public_id_format
    CHECK (public_id ~ '^mem_[0-9a-f]{32}$');

-- ============================================
-- 4. Comments
-- ============================================

COMMENT ON TABLE memories IS 'Memories — org-scoped named stores. See specs/memory.md.';
COMMENT ON TABLE memory_files IS 'Files and directories inside a Memory. Path validation mirrors session_files.';
COMMENT ON TABLE session_memory_mounts IS 'Per-session snapshot of resolved memory capability mounts.';

COMMIT;
