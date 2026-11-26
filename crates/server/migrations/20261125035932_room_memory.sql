-- room_memory: what a room knows (knowledge/runtime-resources/rooms.md, "Room memory").
--
-- A room remembers two kinds of entries next to its messages and files:
-- facts (a typed statement such as a decision or a rule, with the quote it
-- rests on) and notes (text published into the room's memory: a finished
-- session's sittings, or outside text sent through the ingest endpoint).
-- Recall searches messages and entries together by their words.
--
-- A newer fact of the same kind and subject replaces the older one: the older
-- row gets `superseded_at` (the time the replacing fact happened) and
-- `superseded_by`, and both stay. Nothing is purged: forgetting an entry sets
-- `hidden_at`, which takes it out of recall at once.

CREATE TABLE room_memory_entries (
    id uuid DEFAULT public.uuidv7() NOT NULL PRIMARY KEY,
    room_id uuid NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    org_id bigint NOT NULL,
    entry_kind text NOT NULL,
    -- For facts: decision, rule, claim, deadline, question or observation.
    fact_kind text,
    -- For facts: what the statement is about, normalized; replacement matches on it.
    subject text,
    statement text NOT NULL,
    -- The exact words of the source the entry rests on.
    quote text,
    -- stated, proposed (an option nobody adopted), hypothetical, or retracted.
    status text DEFAULT 'stated' NOT NULL,
    -- When the thing happened, which is not always when it was recorded.
    occurred_at timestamptz NOT NULL,
    author_member_id uuid REFERENCES room_members(id),
    -- message, session, ingest or member: where the entry came from.
    source_kind text NOT NULL,
    source_message_id uuid REFERENCES room_messages(id),
    source_session_id uuid,
    -- The caller's own id for an ingested note or a session sitting.
    source_ref text,
    label text,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    -- extraction (the room's background pass), member, ingest or session.
    recorded_by text NOT NULL,
    superseded_at timestamptz,
    superseded_by uuid REFERENCES room_memory_entries(id),
    hidden_at timestamptz,
    hidden_by_member_id uuid REFERENCES room_members(id),
    created_at timestamptz DEFAULT now() NOT NULL,
    terms tsvector GENERATED ALWAYS AS (
        to_tsvector(
            'english',
            coalesce(subject, '') || ' ' || statement || ' ' || coalesce(quote, '') || ' ' || coalesce(label, '')
        )
    ) STORED,
    CONSTRAINT room_memory_entries_kind_check CHECK (entry_kind IN ('fact', 'note')),
    CONSTRAINT room_memory_entries_fact_kind_check CHECK (
        (entry_kind = 'fact' AND fact_kind IN ('decision', 'rule', 'claim', 'deadline', 'question', 'observation')
            AND subject IS NOT NULL)
        OR (entry_kind = 'note' AND fact_kind IS NULL)
    ),
    CONSTRAINT room_memory_entries_status_check CHECK (status IN ('stated', 'proposed', 'hypothetical', 'retracted')),
    CONSTRAINT room_memory_entries_source_check CHECK (source_kind IN ('message', 'session', 'ingest', 'member')),
    CONSTRAINT room_memory_entries_recorded_by_check CHECK (recorded_by IN ('extraction', 'member', 'ingest', 'session')),
    CONSTRAINT room_memory_entries_statement_check CHECK (char_length(statement) BETWEEN 1 AND 40000),
    CONSTRAINT room_memory_entries_subject_check CHECK (subject IS NULL OR char_length(subject) BETWEEN 1 AND 200)
);

CREATE INDEX room_memory_entries_terms_idx ON room_memory_entries USING gin (terms);
CREATE INDEX room_memory_entries_room_idx ON room_memory_entries (room_id, occurred_at DESC);
CREATE INDEX room_memory_entries_subject_idx ON room_memory_entries (room_id, fact_kind, subject, occurred_at)
    WHERE entry_kind = 'fact' AND hidden_at IS NULL;
CREATE INDEX room_memory_entries_message_idx ON room_memory_entries (source_message_id)
    WHERE source_message_id IS NOT NULL;
-- One live entry per caller id: a re-send with new content hides the old one.
CREATE UNIQUE INDEX room_memory_entries_source_ref_idx ON room_memory_entries (room_id, source_ref)
    WHERE source_ref IS NOT NULL AND hidden_at IS NULL;

COMMENT ON TABLE room_memory_entries IS
    'Facts and notes a room remembers, searched with its messages by room recall.';

-- Recall searches message text by its words, stemmed.
CREATE INDEX room_messages_terms_idx ON room_messages
    USING gin (to_tsvector('english', text)) WHERE deleted_at IS NULL;

-- Whether an agent member gets what the room remembers about a message
-- when that message wakes it. Off by default: it adds to every turn's context.
ALTER TABLE room_members ADD COLUMN memory_recall boolean DEFAULT false NOT NULL;
