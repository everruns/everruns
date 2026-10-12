-- Slack now runs on the shared channel host, which finds a conversation's
-- session through channel_bindings instead of session tags. Bind every
-- existing Slack conversation under the key the host now uses, to the session
-- the tag lookup found (the oldest), so no thread starts over.
--
-- The key is the routing tag the session already carries: `slack:thread:{ts}`,
-- `slack:channel:{id}` or `slack:user:{id}`, the same strings the host builds.
--
-- A session is a channel's under the tag lookup's own rule: a native
-- channel's session has that channel's ingress and owner; an App-era
-- channel's session belongs to its App. Either way it carries the channel's
-- `slack:endpoint:` tag, which kept two Slack channels of one App apart.

WITH channel_sessions AS (
    SELECT channel.id AS channel_id, session.id AS session_id,
           session.tags, session.created_at
    FROM agent_channels AS channel
    JOIN sessions AS session
      ON (session.channel_id = channel.id
          AND session.owner_principal_id = channel.owner_principal_id)
      OR (session.channel_id IS NULL
          AND channel.app_id IS NOT NULL
          AND session.app_id = channel.app_id)
    WHERE channel.channel_type = 'slack'
      AND session.tags @> ARRAY['slack:endpoint:' || channel.public_id]
),
keyed AS (
    SELECT channel_id, session_id, created_at, route.tag AS binding_key
    FROM channel_sessions
    CROSS JOIN LATERAL (
        SELECT tag FROM unnest(tags) AS tag
        WHERE tag LIKE 'slack:thread:%'
           OR tag LIKE 'slack:channel:%'
           OR tag LIKE 'slack:user:%'
    ) AS route
)
INSERT INTO channel_bindings (channel_id, binding_key, session_id)
SELECT DISTINCT ON (channel_id, binding_key) channel_id, binding_key, session_id
FROM keyed
ORDER BY channel_id, binding_key, created_at
ON CONFLICT (channel_id, binding_key) DO NOTHING;
