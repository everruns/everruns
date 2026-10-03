-- Agent presentation replaces harness presentation. Keep legacy columns only
-- for import/history; runtime and public surfaces no longer consume them.
UPDATE agents a SET
    intro_markdown = COALESCE(NULLIF(a.intro_markdown, ''), h.intro_markdown),
    short_description = COALESCE(NULLIF(a.short_description, ''), h.short_description),
    starters = CASE WHEN a.starters = '[]'::jsonb THEN h.starters ELSE a.starters END
FROM harnesses h WHERE h.id = a.harness_id AND h.org_id = a.org_id;

-- Reserve the managed name without replacing any user-authored Agent or ID.
UPDATE agents SET name = 'platform-chat-custom-' || left(replace(id::text, '-', ''), 12)
WHERE name = 'platform-chat' AND NOT is_built_in AND status != 'deleted';

INSERT INTO agents (id, public_id, org_id, name, display_name, system_prompt, harness_id, is_built_in)
SELECT id, 'agent_' || replace(id::text, '-', ''), org_id, 'platform-chat', 'Platform Chat', '', harness_id, true
FROM (SELECT gen_random_uuid() id, h.org_id, h.id harness_id FROM harnesses h
      WHERE h.name = 'generic' AND h.is_built_in AND h.status = 'active') seeds
ON CONFLICT (org_id, name) WHERE status != 'deleted' DO NOTHING;

-- The permanent conversation keeps its existing identity and transcript.
DROP TRIGGER mark_platform_chat_starter_tag ON sessions;
CREATE OR REPLACE FUNCTION mark_platform_chat_starter_tag() RETURNS trigger AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM agents a WHERE a.id = NEW.agent_id AND a.org_id = NEW.org_id
               AND a.is_built_in AND a.name = 'platform-chat')
       AND NOT 'platform-chat-starter' = ANY(NEW.tags) THEN
        NEW.tags := array_append(NEW.tags, 'platform-chat-starter');
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER mark_platform_chat_starter_tag BEFORE INSERT ON sessions
FOR EACH ROW WHEN (NEW.title IN ('Chat', 'Platform Chat') AND NEW.source = 'chat'
                  AND 'chat' = ANY(NEW.tags) AND NEW.parent_session_id IS NULL
                  AND NEW.forked_from_session_id IS NULL)
EXECUTE FUNCTION mark_platform_chat_starter_tag();

UPDATE sessions s SET agent_id = a.id, harness_id = generic.id,
    source = CASE WHEN s.tags && ARRAY['chat','global-chat']::text[] THEN 'chat' ELSE s.source END
FROM harnesses old, agents a, harnesses generic
WHERE s.org_id = old.org_id AND s.harness_id = old.id AND s.agent_id IS NULL
  AND old.is_built_in AND old.name IN ('platform-chat', 'platform-chat-v2') AND s.source != 'playground'
  AND a.org_id = s.org_id AND a.name = 'platform-chat' AND a.is_built_in AND a.status = 'active'
  AND generic.org_id = s.org_id AND generic.name = 'generic' AND generic.is_built_in AND generic.status = 'active';
UPDATE sessions SET archived_at = NULL WHERE 'platform-chat-starter' = ANY(tags);
UPDATE harnesses SET intro_markdown = NULL, short_description = NULL, starters = '[]'::jsonb;
