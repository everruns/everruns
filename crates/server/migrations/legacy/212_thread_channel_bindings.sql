-- AG-UI, Public Chat and FCP now run on the shared channel host, which finds
-- a conversation's session through channel_bindings instead of session tags.
-- Bind every existing conversation under the key the host now uses, to the
-- session the tag lookup found (the oldest), so no client starts over:
--
-- - AG-UI: `thread:{thread id}`, plus `|virtual_user:{id}` for an identified
--   caller's thread.
-- - Public Chat: `thread:{thread id}|{visitor tag}`.
-- - FCP: the session id, which is what cookies set before the host carry.
--
-- A session is a channel's when its ingress was that channel, or, for an
-- App-era channel, when it was that App's (the tag lookup's own rule).

WITH channel_sessions AS (
    SELECT channel.id AS channel_id, channel.public_id AS channel_public_id,
           channel.channel_type, session.id AS session_id,
           session.tags, session.created_at
    FROM agent_channels AS channel
    JOIN sessions AS session
      ON session.channel_id = channel.id
      OR (session.channel_id IS NULL
          AND channel.app_id IS NOT NULL
          AND session.app_id = channel.app_id)
    WHERE channel.channel_type IN ('ag_ui', 'public_chat', 'fcp')
),
keyed AS (
    SELECT channel_id, session_id, created_at,
           'thread:' || substr(thread.tag, length('ag_ui:thread:') + 1)
               || COALESCE('|virtual_user:' || substr(caller.tag, length('ag_ui:virtual_user:') + 1), '')
               AS binding_key
    FROM channel_sessions
    CROSS JOIN LATERAL (
        SELECT tag FROM unnest(tags) AS tag WHERE tag LIKE 'ag\_ui:thread:%' LIMIT 1
    ) AS thread
    LEFT JOIN LATERAL (
        SELECT tag FROM unnest(tags) AS tag WHERE tag LIKE 'ag\_ui:virtual\_user:%' LIMIT 1
    ) AS caller ON TRUE
    WHERE channel_type = 'ag_ui'

    UNION ALL

    SELECT channel_id, session_id, created_at,
           'thread:' || substr(thread.tag, length('public_chat:thread:') + 1)
               || '|' || visitor.tag AS binding_key
    FROM channel_sessions
    CROSS JOIN LATERAL (
        SELECT tag FROM unnest(tags) AS tag WHERE tag LIKE 'public\_chat:thread:%' LIMIT 1
    ) AS thread
    CROSS JOIN LATERAL (
        SELECT tag FROM unnest(tags) AS tag WHERE tag LIKE 'public\_chat:visitor:%' LIMIT 1
    ) AS visitor
    WHERE channel_type = 'public_chat'

    UNION ALL

    SELECT channel_id, session_id, created_at, session_id::text AS binding_key
    FROM channel_sessions
    WHERE channel_type = 'fcp'
      AND tags @> ARRAY['fcp:endpoint:' || channel_public_id]
)
INSERT INTO channel_bindings (channel_id, binding_key, session_id)
SELECT DISTINCT ON (channel_id, binding_key) channel_id, binding_key, session_id
FROM keyed
ORDER BY channel_id, binding_key, created_at
ON CONFLICT (channel_id, binding_key) DO NOTHING;
