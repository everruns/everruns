-- Links a platform message (a Slack `channel:ts`) to the Everruns message it
-- carries, so a reaction made on the platform reaches the right message and
-- the agent's own reactions reach the platform. First link wins: a key names
-- one message for good.
CREATE TABLE channel_message_links (
    channel_id uuid NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    message_key text NOT NULL,
    session_id uuid NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    message_id text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, message_key)
);

-- The reverse lookup: the platform message an agent reaction lands on.
CREATE INDEX idx_channel_message_links_message
    ON channel_message_links (channel_id, session_id, message_id);
