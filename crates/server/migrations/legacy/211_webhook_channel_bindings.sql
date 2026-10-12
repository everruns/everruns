-- Webhook channels now run on the shared channel host, which finds a shared
-- webhook session through channel_bindings (key `webhook:shared`) instead of
-- session tags. Bind each shared-mode webhook channel to the session the tag
-- lookup found: the oldest one carrying the channel's tags and owner that was
-- not a per-invocation session, so no webhook starts over.

INSERT INTO channel_bindings (channel_id, binding_key, session_id)
SELECT DISTINCT ON (channel.id) channel.id, 'webhook:shared', session.id
FROM agent_channels AS channel
JOIN sessions AS session
  ON (session.channel_id = channel.id
      -- An App-era channel's sessions are the App's, as the tag lookup had it.
      OR (channel.app_id IS NOT NULL AND session.app_id = channel.app_id))
 AND session.owner_principal_id = channel.owner_principal_id
WHERE channel.channel_type = 'webhook'
  AND session.tags @> ARRAY['app_channel:' || channel.public_id, 'app_channel_type:webhook']
  AND NOT EXISTS (
      SELECT 1 FROM unnest(session.tags) AS tag WHERE tag LIKE 'app_invocation:%'
  )
ORDER BY channel.id, session.created_at
ON CONFLICT (channel_id, binding_key) DO NOTHING;
