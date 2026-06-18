-- Make a harness's base system prompt optional.
--
-- A harness bundles many configuration dimensions (capabilities, MCP servers,
-- model default, network access, initial files). A perfectly valid harness —
-- especially a child harness whose only job is to add capabilities or MCP
-- servers — may contribute no base prompt of its own and rely entirely on the
-- parent harness, agent, session, and capability layers. Requiring a base
-- prompt forced callers to invent filler text or duplicate the parent prompt.
--
-- The prompt-composition layer already treats each layer's contribution as
-- optional (empty/whitespace-only fragments are trimmed to "no contribution"),
-- so dropping the NOT NULL constraint simply lets storage represent that.
-- Existing rows are unaffected; no backfill is required.
ALTER TABLE harnesses
ALTER COLUMN system_prompt DROP NOT NULL;
