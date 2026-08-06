-- Allow `fcp` (Free Communication Protocol) app channels.
-- See knowledge/integrations/fcp-channel.md.

ALTER TABLE apps
    DROP CONSTRAINT IF EXISTS apps_channel_type_check;

ALTER TABLE apps
    ADD CONSTRAINT apps_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp'));

ALTER TABLE app_channels
    DROP CONSTRAINT IF EXISTS app_channels_channel_type_check;

ALTER TABLE app_channels
    ADD CONSTRAINT app_channels_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp'));
