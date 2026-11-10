-- Rooms: org-owned conversation spaces where people and agents talk
-- (knowledge/runtime-resources/rooms.md). A room is a platform primitive like
-- a session; products build their own boards on top of it through the API.
--
-- Nothing here is ever purged: archiving a room, removing a member and
-- deleting a message all set a timestamp and keep the row.

CREATE TABLE rooms (
    id uuid DEFAULT public.uuidv7() NOT NULL PRIMARY KEY,
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    name text NOT NULL,
    topic text,
    -- The builder's own id for this room, such as a ticket or a project key.
    external_ref text,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    -- Where a message opens in the builder's product: `{room_id}`,
    -- `{message_id}` and `{thread_id}` are filled in.
    link_template text,
    -- Loop limits and other per-room settings; empty means the defaults.
    settings jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_by_user_id uuid,
    archived_at timestamptz,
    last_message_at timestamptz,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT rooms_name_check CHECK (char_length(name) BETWEEN 1 AND 120)
);

CREATE INDEX rooms_org_idx ON rooms (org_id, archived_at, last_message_at DESC);
CREATE UNIQUE INDEX rooms_org_external_ref_idx ON rooms (org_id, external_ref)
    WHERE external_ref IS NOT NULL;

COMMENT ON TABLE rooms IS
    'Org-owned conversation space for people, Everruns agents, outside agents and guests.';

CREATE TABLE room_members (
    id uuid DEFAULT public.uuidv7() NOT NULL PRIMARY KEY,
    room_id uuid NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    org_id bigint NOT NULL,
    -- person: an org user. agent: an Everruns agent the room wakes.
    -- outside_agent: an agent elsewhere that posts through the API.
    -- guest: someone a builder's product speaks for (an end user).
    kind text NOT NULL,
    user_id uuid,
    agent_id uuid REFERENCES agents(id) ON DELETE CASCADE,
    handle text NOT NULL,
    display_name text NOT NULL,
    role text DEFAULT 'member' NOT NULL,
    -- When an agent member is woken: mentions (and threads it is in),
    -- everything, or never.
    wake text DEFAULT 'mentions' NOT NULL,
    -- One agent conversation per thread, or one for the whole room.
    session_scope text DEFAULT 'thread' NOT NULL,
    instructions text,
    left_at timestamptz,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT room_members_kind_check CHECK (kind IN ('person', 'agent', 'outside_agent', 'guest')),
    CONSTRAINT room_members_role_check CHECK (role IN ('owner', 'member')),
    CONSTRAINT room_members_wake_check CHECK (wake IN ('mentions', 'everything', 'never')),
    CONSTRAINT room_members_session_scope_check CHECK (session_scope IN ('thread', 'room')),
    CONSTRAINT room_members_person_check CHECK (kind <> 'person' OR user_id IS NOT NULL),
    CONSTRAINT room_members_agent_check CHECK (kind <> 'agent' OR agent_id IS NOT NULL),
    CONSTRAINT room_members_handle_check CHECK (handle ~ '^[a-z0-9][a-z0-9_.-]{0,39}$')
);

CREATE UNIQUE INDEX room_members_handle_idx ON room_members (room_id, handle) WHERE left_at IS NULL;
CREATE UNIQUE INDEX room_members_user_idx ON room_members (room_id, user_id)
    WHERE left_at IS NULL AND user_id IS NOT NULL;
CREATE UNIQUE INDEX room_members_agent_idx ON room_members (room_id, agent_id)
    WHERE left_at IS NULL AND agent_id IS NOT NULL;
CREATE INDEX room_members_user_rooms_idx ON room_members (org_id, user_id) WHERE left_at IS NULL;

CREATE TABLE room_messages (
    id uuid DEFAULT public.uuidv7() NOT NULL PRIMARY KEY,
    room_id uuid NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    org_id bigint NOT NULL,
    -- NULL for a room notice the platform wrote.
    author_member_id uuid REFERENCES room_members(id),
    -- Root message of the thread this is a reply in; NULL at the top level.
    -- Replies are one level deep.
    thread_id uuid REFERENCES room_messages(id),
    text text NOT NULL,
    parts jsonb DEFAULT '[]'::jsonb NOT NULL,
    mentions uuid[] DEFAULT '{}'::uuid[] NOT NULL,
    -- How many agent posts led here since a person or guest last wrote.
    hops integer DEFAULT 0 NOT NULL,
    -- The agent session that sent it, for agent posts, and the session
    -- message it came from, so a redelivered event cannot post it twice.
    session_id uuid,
    source_ref text,
    reply_count integer DEFAULT 0 NOT NULL,
    last_reply_at timestamptz,
    pinned_at timestamptz,
    edited_at timestamptz,
    deleted_at timestamptz,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT room_messages_text_check CHECK (char_length(text) <= 40000)
);

CREATE INDEX room_messages_room_idx ON room_messages (room_id, created_at) WHERE thread_id IS NULL;
CREATE INDEX room_messages_thread_idx ON room_messages (thread_id, created_at) WHERE thread_id IS NOT NULL;
CREATE INDEX room_messages_mentions_idx ON room_messages USING gin (mentions);
CREATE UNIQUE INDEX room_messages_source_ref_idx ON room_messages (room_id, source_ref)
    WHERE source_ref IS NOT NULL;

CREATE TABLE room_reactions (
    message_id uuid NOT NULL REFERENCES room_messages(id) ON DELETE CASCADE,
    member_id uuid NOT NULL REFERENCES room_members(id) ON DELETE CASCADE,
    emoji text NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    PRIMARY KEY (message_id, member_id, emoji),
    CONSTRAINT room_reactions_emoji_check CHECK (char_length(emoji) BETWEEN 1 AND 64)
);

-- One high-water mark per reader and scope: the room's top level (scope_id =
-- the room id) or one thread (scope_id = the thread's root message id).
CREATE TABLE room_read_marks (
    member_id uuid NOT NULL REFERENCES room_members(id) ON DELETE CASCADE,
    scope_id uuid NOT NULL,
    room_id uuid NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    read_through timestamptz NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    PRIMARY KEY (member_id, scope_id)
);

CREATE INDEX room_read_marks_scope_idx ON room_read_marks (scope_id);

-- The room's change feed, in order: what SSE, webhooks and polling read.
CREATE TABLE room_events (
    seq bigserial PRIMARY KEY,
    room_id uuid NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    org_id bigint NOT NULL,
    event_type text NOT NULL,
    message_id uuid,
    member_id uuid,
    data jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL
);

CREATE INDEX room_events_room_idx ON room_events (room_id, seq);
CREATE INDEX room_events_org_idx ON room_events (org_id, seq);

-- The agent conversation a member has in one scope (a thread, or the room).
CREATE TABLE room_agent_sessions (
    member_id uuid NOT NULL REFERENCES room_members(id) ON DELETE CASCADE,
    scope_id uuid NOT NULL,
    room_id uuid NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    session_id uuid NOT NULL,
    -- The message that last woke the agent here: its reply lands in that
    -- message's thread and counts one hop further.
    last_wake_message_id uuid,
    last_wake_thread_id uuid,
    last_wake_hops integer DEFAULT 0 NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    PRIMARY KEY (member_id, scope_id)
);

CREATE UNIQUE INDEX room_agent_sessions_session_idx ON room_agent_sessions (session_id);

-- Sessions an agent runs for a room say so.
ALTER TABLE sessions DROP CONSTRAINT sessions_source_check;
ALTER TABLE sessions ADD CONSTRAINT sessions_source_check CHECK ((source = ANY (ARRAY[
    'chat'::text, 'playground'::text, 'api'::text, 'slack'::text, 'ag_ui'::text, 'fcp'::text,
    'schedule'::text, 'webhook'::text, 'a2a'::text, 'eval'::text, 'subagent'::text,
    'room'::text, 'unknown'::text
])));
