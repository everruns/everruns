-- A2A channels now run on the shared channel host. A `shared_session`
-- channel's one conversation is found through channel_bindings (key
-- `a2a:shared`) instead of session tags. Bind each A2A channel to the session
-- the tag lookup found: the oldest one carrying the channel's tags and owner
-- that was not a per-invocation (or per-caller) session, so no shared
-- conversation starts over. Channels in another mode never read the binding.

INSERT INTO channel_bindings (channel_id, binding_key, session_id)
SELECT DISTINCT ON (channel.id) channel.id, 'a2a:shared', session.id
FROM agent_channels AS channel
JOIN sessions AS session
  ON (session.channel_id = channel.id
      -- An App-era channel's sessions are the App's, as the tag lookup had it.
      OR (channel.app_id IS NOT NULL AND session.app_id = channel.app_id))
 AND session.owner_principal_id = channel.owner_principal_id
WHERE channel.channel_type = 'a2a'
  AND session.tags @> ARRAY['app_channel:' || channel.public_id, 'app_channel_type:a2a']
  AND NOT EXISTS (
      SELECT 1 FROM unnest(session.tags) AS tag WHERE tag LIKE 'app_invocation:%'
  )
ORDER BY channel.id, session.created_at
ON CONFLICT (channel_id, binding_key) DO NOTHING;
