-- Host-owned output state and write-ahead provider intents, scoped by the existing
-- pending delivery's tenant/channel/session relationships. No credentials belong here.
ALTER TABLE channel_pending_deliveries ADD COLUMN checkpoint JSONB;
