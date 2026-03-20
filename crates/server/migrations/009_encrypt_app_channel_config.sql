-- Encrypt app channel credentials (EVE-158)
--
-- channel_config JSONB contains secrets (e.g. Slack signing_secret, bot_token).
-- Move to an encrypted column following the envelope encryption pattern.
-- Keep the old column temporarily for migration reads; new writes go only to
-- channel_config_encrypted.

ALTER TABLE apps ADD COLUMN channel_config_encrypted BYTEA;

-- Backfill: mark existing rows as needing migration by leaving
-- channel_config_encrypted NULL. The app service reads from encrypted first,
-- falls back to plaintext, and writes encrypted on any update.
