-- Memory v2 (knowledge/runtime-resources/memory.md): memory is folders of
-- files under /memory/<store>, with typed entries as Markdown files.
--
-- 1. A team store: owned by the coordinator agent, shared read-write by it
--    and every thread or agent it starts, across runs. scope = 'team',
--    keyed by owner_agent_id like the agent store.
-- 2. Every change to a memory file is kept in memory_file_versions, written
--    by a trigger so no write path can skip it. Nothing is ever purged:
--    forgetting an entry deletes the file and records a `delete` version.
--    Room memories keep their own room_file_versions and are skipped here.
-- 3. Source-backed (GitHub/Git) memories are retired with memory v1; their
--    rows are archived, never deleted.

ALTER TABLE memories DROP CONSTRAINT memories_scope_check;
ALTER TABLE memories ADD CONSTRAINT memories_scope_check
    CHECK (scope IN ('org', 'agent', 'user', 'run', 'room', 'team'));

ALTER TABLE memories DROP CONSTRAINT memories_scope_owner_shape;
ALTER TABLE memories ADD CONSTRAINT memories_scope_owner_shape CHECK (
    (scope = 'org' AND owner_agent_id IS NULL AND owner_user_id IS NULL AND owner_session_id IS NULL AND owner_room_id IS NULL)
    OR (scope IN ('agent', 'team') AND owner_agent_id IS NOT NULL AND owner_user_id IS NULL AND owner_session_id IS NULL AND owner_room_id IS NULL)
    OR (scope = 'user' AND owner_agent_id IS NULL AND owner_user_id IS NOT NULL AND owner_session_id IS NULL AND owner_room_id IS NULL)
    OR (scope = 'run' AND owner_agent_id IS NULL AND owner_user_id IS NULL AND owner_session_id IS NOT NULL AND owner_room_id IS NULL)
    OR (scope = 'room' AND owner_agent_id IS NULL AND owner_user_id IS NULL AND owner_session_id IS NULL AND owner_room_id IS NOT NULL)
);

CREATE UNIQUE INDEX idx_memories_org_team_scope_owner_active
    ON memories (org_id, owner_agent_id)
    WHERE scope = 'team' AND status <> 'deleted';

CREATE TABLE memory_file_versions (
    id uuid DEFAULT public.uuidv7() NOT NULL PRIMARY KEY,
    memory_id uuid NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    path text NOT NULL,
    -- write (created or changed) or delete.
    action text NOT NULL,
    -- The content after the change; NULL for a delete.
    content bytea,
    size_bytes bigint DEFAULT 0 NOT NULL,
    -- Who changed it, when the writer said (`SET LOCAL everruns.memory_actor`).
    changed_by text,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT memory_file_versions_action_check CHECK (action IN ('write', 'delete'))
);

CREATE INDEX memory_file_versions_path_idx
    ON memory_file_versions (memory_id, path, created_at DESC);

COMMENT ON TABLE memory_file_versions IS 'Every version of every memory file, never purged. See knowledge/runtime-resources/memory.md.';

CREATE FUNCTION record_memory_file_version() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    actor text := NULLIF(current_setting('everruns.memory_actor', true), '');
    target uuid := CASE WHEN TG_OP = 'DELETE' THEN OLD.memory_id ELSE NEW.memory_id END;
BEGIN
    IF EXISTS (SELECT 1 FROM memories WHERE id = target AND scope = 'room') THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF NOT OLD.is_directory THEN
            INSERT INTO memory_file_versions (memory_id, path, action, changed_by)
            VALUES (OLD.memory_id, OLD.path, 'delete', actor);
        END IF;
        RETURN NULL;
    END IF;
    IF NEW.is_directory THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.path = OLD.path AND NEW.content IS NOT DISTINCT FROM OLD.content THEN
            RETURN NULL;
        END IF;
        IF NEW.path <> OLD.path THEN
            INSERT INTO memory_file_versions (memory_id, path, action, changed_by)
            VALUES (OLD.memory_id, OLD.path, 'delete', actor);
        END IF;
    END IF;
    INSERT INTO memory_file_versions (memory_id, path, action, content, size_bytes, changed_by)
    VALUES (NEW.memory_id, NEW.path, 'write', NEW.content, NEW.size_bytes, actor);
    RETURN NULL;
END;
$$;

CREATE TRIGGER memory_file_versions_record
    AFTER INSERT OR UPDATE OR DELETE ON memory_files
    FOR EACH ROW EXECUTE FUNCTION record_memory_file_version();

-- The current files become each file's first version.
INSERT INTO memory_file_versions (memory_id, path, action, content, size_bytes, created_at)
SELECT f.memory_id, f.path, 'write', f.content, f.size_bytes, f.updated_at
FROM memory_files f
JOIN memories m ON m.id = f.memory_id
WHERE NOT f.is_directory AND m.scope <> 'room';

UPDATE memories
SET status = 'archived', archived_at = now(), sync_status = 'idle'
WHERE source_type IN ('github', 'git') AND status = 'active';

-- 4. The v1 `memory` capability copied named memories into /workspace when a
--    session started. Its attachments become `agent_memory` attachments of
--    the same memories, live at /memory/<name>, when the name is a valid
--    store name (lower-case letters, digits and dashes); the capability
--    keeps `memory` as an alias, so older definitions still load.
CREATE FUNCTION memory_v1_to_v2_config(config jsonb) RETURNS jsonb
LANGUAGE sql STABLE AS $$
    SELECT jsonb_build_object('attach', COALESCE(jsonb_agg(DISTINCT jsonb_build_object(
        'memory', m.name,
        'mode', CASE WHEN mount->>'mode' = 'readwrite' AND NOT m.is_readonly
                     THEN 'read_write' ELSE 'read' END
    )), '[]'::jsonb))
    FROM jsonb_array_elements(COALESCE(config->'mounts', '[]'::jsonb)) AS mount
    JOIN memories m ON m.public_id = mount->>'memory' AND m.status = 'active'
    WHERE m.name ~ '^[a-z][a-z0-9-]{0,61}[a-z0-9]$'
      AND m.name NOT IN ('agent', 'user', 'org', 'team', 'parent', 'named', 'shared')
$$;

UPDATE agent_capabilities ac
SET capability_id = 'agent_memory', config = memory_v1_to_v2_config(ac.config)
WHERE ac.capability_id = 'memory'
  AND NOT EXISTS (
      SELECT 1 FROM agent_capabilities other
      WHERE other.agent_id = ac.agent_id AND other.capability_id = 'agent_memory'
  );

UPDATE harness_capabilities hc
SET capability_id = 'agent_memory', config = memory_v1_to_v2_config(hc.config)
WHERE hc.capability_id = 'memory'
  AND NOT EXISTS (
      SELECT 1 FROM harness_capabilities other
      WHERE other.harness_id = hc.harness_id AND other.capability_id = 'agent_memory'
  );

DROP FUNCTION memory_v1_to_v2_config(jsonb);
