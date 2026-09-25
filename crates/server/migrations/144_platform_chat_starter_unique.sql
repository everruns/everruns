-- One starter Platform Chat belongs to each owner in each organization. A
-- client-side existence check cannot arbitrate between browser tabs, stale
-- cache entries, or a request that succeeds after its response is lost.
-- Adopt an existing chat first so a later stale client cannot mint a new one.
-- Hold writers across adoption and index creation so old servers cannot insert
-- an unmarked starter in the gap between the two statements.
LOCK TABLE sessions IN SHARE ROW EXCLUSIVE MODE;

-- This tag was not reserved before this migration, so every existing use is
-- untrusted. Clear legacy values before electing starters; otherwise arbitrary
-- tagged sessions can collide on the unique index and block server startup.
UPDATE sessions
SET tags = array_remove(tags, 'platform-chat-starter')
WHERE 'platform-chat-starter' = ANY(tags);

WITH ranked AS (
    SELECT s.id,
           ROW_NUMBER() OVER (
               PARTITION BY s.org_id, s.owner_principal_id
               ORDER BY (s.event_count > 0) DESC,
                        (s.archived_at IS NULL) DESC,
                        s.event_count DESC,
                        s.created_at ASC,
                        s.id ASC
           ) AS position
    FROM sessions s
    JOIN harnesses h ON h.id = s.harness_id
    WHERE h.name = 'platform-chat'
      AND s.agent_id IS NULL
      AND s.tags && ARRAY['chat', 'global-chat']::text[]
)
UPDATE sessions s
SET tags = array_append(s.tags, 'platform-chat-starter')
FROM ranked r
WHERE s.id = r.id AND r.position = 1
  AND NOT 'platform-chat-starter' = ANY(s.tags);

-- Recognize the old browser request shape at the database boundary too. An
-- older server process can still serve a request during a rolling replacement.
CREATE FUNCTION mark_platform_chat_starter_tag() RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM harnesses h
        WHERE h.id = NEW.harness_id
          AND h.org_id = NEW.org_id
          AND h.name = 'platform-chat'
    ) AND NOT 'platform-chat-starter' = ANY(NEW.tags) THEN
        NEW.tags := array_append(NEW.tags, 'platform-chat-starter');
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER mark_platform_chat_starter_tag
    BEFORE INSERT ON sessions
    FOR EACH ROW
    WHEN (NEW.agent_id IS NULL AND NEW.title = 'Platform Chat'
          AND NEW.source IN ('chat', 'api') AND 'chat' = ANY(NEW.tags))
    EXECUTE FUNCTION mark_platform_chat_starter_tag();

-- Archived starters still count: archiving is a deliberate opt-out of the
-- automatic thread, not permission to recreate it on the next page load.
CREATE UNIQUE INDEX idx_sessions_platform_chat_starter_owner
    ON sessions (org_id, owner_principal_id)
    WHERE tags @> ARRAY['platform-chat-starter']::text[];

-- Session tags are editable. Preserve this one identity marker on updates so
-- editing tags cannot silently reopen the automatic creation path.
CREATE FUNCTION keep_platform_chat_starter_tag() RETURNS trigger AS $$
BEGIN
    IF 'platform-chat-starter' = ANY(OLD.tags)
       AND NOT 'platform-chat-starter' = ANY(NEW.tags) THEN
        NEW.tags := array_append(NEW.tags, 'platform-chat-starter');
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER keep_platform_chat_starter_tag
    BEFORE UPDATE OF tags ON sessions
    FOR EACH ROW EXECUTE FUNCTION keep_platform_chat_starter_tag();

-- Earlier browser-only guards left pinned, empty starter copies. Archive only
-- those copies; a thread with any events, a user-chosen title, or no owner pin
-- remains untouched. Archiving preserves the URL and permits restoration.
UPDATE sessions s
SET archived_at = NOW()
FROM harnesses h
WHERE h.id = s.harness_id
  AND h.name = 'platform-chat'
  AND s.agent_id IS NULL
  AND s.archived_at IS NULL
  AND s.event_count = 0
  AND s.title = 'Platform Chat'
  AND 'chat' = ANY(s.tags)
  AND NOT 'platform-chat-starter' = ANY(s.tags)
  AND EXISTS (
      SELECT 1 FROM pinned_sessions p
      WHERE p.session_id = s.id AND p.user_id = s.resolved_owner_user_id
  )
  AND EXISTS (
      SELECT 1 FROM sessions starter
      WHERE starter.org_id = s.org_id
        AND starter.owner_principal_id = s.owner_principal_id
        AND 'platform-chat-starter' = ANY(starter.tags)
  );
