-- Room files: a shared folder per room for what should outlast the chat
-- (notes, plans, decisions, data), edited in place, with every version kept
-- (knowledge/runtime-resources/rooms.md).
--
-- The current files live in a room-owned Memory, so an agent's room session
-- mounts them at /room with the ordinary file tools. Every change to them also
-- lands in room_file_versions, in the same transaction, with who made it.
-- Nothing here is ever purged: deleting a file records a `delete` version and
-- keeps every earlier one.

ALTER TABLE memories ADD COLUMN owner_room_id uuid REFERENCES rooms(id);

ALTER TABLE memories DROP CONSTRAINT memories_scope_check;
ALTER TABLE memories ADD CONSTRAINT memories_scope_check
    CHECK (scope IN ('org', 'agent', 'user', 'run', 'room'));

ALTER TABLE memories DROP CONSTRAINT memories_scope_owner_shape;
ALTER TABLE memories ADD CONSTRAINT memories_scope_owner_shape CHECK (
    (scope = 'org' AND owner_agent_id IS NULL AND owner_user_id IS NULL AND owner_session_id IS NULL AND owner_room_id IS NULL)
    OR (scope = 'agent' AND owner_agent_id IS NOT NULL AND owner_user_id IS NULL AND owner_session_id IS NULL AND owner_room_id IS NULL)
    OR (scope = 'user' AND owner_agent_id IS NULL AND owner_user_id IS NOT NULL AND owner_session_id IS NULL AND owner_room_id IS NULL)
    OR (scope = 'run' AND owner_agent_id IS NULL AND owner_user_id IS NULL AND owner_session_id IS NOT NULL AND owner_room_id IS NULL)
    OR (scope = 'room' AND owner_agent_id IS NULL AND owner_user_id IS NULL AND owner_session_id IS NULL AND owner_room_id IS NOT NULL)
);

CREATE UNIQUE INDEX idx_memories_org_room_scope_owner_active
    ON memories (org_id, owner_room_id)
    WHERE scope = 'room' AND status <> 'deleted';

CREATE TABLE room_file_versions (
    id uuid DEFAULT public.uuidv7() NOT NULL PRIMARY KEY,
    room_id uuid NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    org_id bigint NOT NULL,
    -- The file's identity: its memory_files row id, which a move keeps and a
    -- restore of a deleted file reuses, so one history follows the file.
    file_id uuid NOT NULL,
    -- Where the file was after this change.
    path text NOT NULL,
    -- Counts from 1 per file.
    version integer NOT NULL,
    -- write, move (path changed), delete, or restore (an older version
    -- written back).
    action text NOT NULL,
    -- The content after this change; NULL for a delete.
    content bytea,
    size_bytes bigint DEFAULT 0 NOT NULL,
    moved_from text,
    -- NULL when nobody in the room made the change (an org admin without a
    -- membership never writes without one, so in practice: a platform write).
    author_member_id uuid REFERENCES room_members(id),
    -- The agent session that wrote it, for agent writes through /room.
    session_id uuid,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT room_file_versions_action_check CHECK (action IN ('write', 'move', 'delete', 'restore')),
    CONSTRAINT room_file_versions_version_check CHECK (version >= 1)
);

CREATE UNIQUE INDEX room_file_versions_file_idx ON room_file_versions (file_id, version);
CREATE INDEX room_file_versions_path_idx ON room_file_versions (room_id, path, created_at DESC);
