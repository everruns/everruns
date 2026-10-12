-- Allow `voice` channels: an agent exposed to callers by voice, where a speech
-- model listens and speaks and the agent writes every answer.
-- See knowledge/framework/voice-agents.md.

ALTER TABLE apps
    DROP CONSTRAINT IF EXISTS apps_channel_type_check;

ALTER TABLE apps
    ADD CONSTRAINT apps_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', 'public_chat', 'voice'));

ALTER TABLE agent_channels
    DROP CONSTRAINT IF EXISTS agent_channels_channel_type_check;

ALTER TABLE agent_channels
    ADD CONSTRAINT agent_channels_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', 'public_chat', 'voice'));
