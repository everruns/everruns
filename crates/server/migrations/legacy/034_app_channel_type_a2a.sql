-- Allow `a2a` (Agent2Agent) app invocation channels.
-- See specs/a2a-channel.md.

ALTER TABLE apps
    DROP CONSTRAINT IF EXISTS apps_channel_type_check;

ALTER TABLE apps
    ADD CONSTRAINT apps_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a'));

ALTER TABLE app_channels
    DROP CONSTRAINT IF EXISTS app_channels_channel_type_check;

ALTER TABLE app_channels
    ADD CONSTRAINT app_channels_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a'));
