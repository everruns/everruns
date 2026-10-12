-- How an agent talks: `direct` (its assistant text is the reply) or `explicit`
-- (assistant text is working notes; it talks through `send_message`).
-- See knowledge/integrations/explicit-communication.md.
--
-- The agent owns this setting. It replaces Slack's per-channel reply mode,
-- which stored `reply_mode: tool_only` in the channel config and mirrored it
-- onto each Slack session as a `slack:reply_mode:*` / `channel:reply_mode:*`
-- tag that the runtime read.
ALTER TABLE agents
    ADD COLUMN communication TEXT NOT NULL DEFAULT 'direct'
    CHECK (communication IN ('direct', 'explicit'));

-- Carry the old Slack setting onto its agent. Two sources, because encrypted
-- channel configs cannot be read here: any session that carries the tool-only
-- tag (every Slack session that received a message under that mode has one),
-- and any plaintext Slack channel config that asks for it.
UPDATE agents
SET communication = 'explicit'
WHERE id IN (
    SELECT DISTINCT s.agent_id
    FROM sessions s
    WHERE s.agent_id IS NOT NULL
      AND s.tags && ARRAY[
          'slack:reply_mode:tool_only',
          'channel:reply_mode:tool_only',
          'slack:reply_mode:report_progress_only',
          'channel:reply_mode:report_progress_only'
      ]::TEXT[]
    UNION
    SELECT c.agent_id
    FROM agent_channels c
    WHERE c.channel_type = 'slack'
      AND c.channel_config->>'reply_mode' IN ('tool_only', 'report_progress_only')
);

-- The tags and the plaintext config key no longer mean anything. Encrypted
-- configs drop the key the next time they are saved; reads ignore it.
UPDATE sessions
SET tags = ARRAY(
    SELECT t
    FROM unnest(tags) WITH ORDINALITY AS u(t, o)
    WHERE t NOT LIKE 'slack:reply_mode:%'
      AND t NOT LIKE 'channel:reply_mode:%'
    ORDER BY o
)
WHERE tags && ARRAY[
    'slack:reply_mode:tool_only',
    'channel:reply_mode:tool_only',
    'slack:reply_mode:report_progress_only',
    'channel:reply_mode:report_progress_only'
]::TEXT[];

UPDATE agent_channels
SET channel_config = channel_config - 'reply_mode'
WHERE channel_type = 'slack'
  AND channel_config ? 'reply_mode';
