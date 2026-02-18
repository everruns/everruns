-- GitHub App migration: store installation_id instead of OAuth access token.
--
-- GitHub Apps use short-lived installation tokens minted on demand (1h TTL),
-- so we no longer need to store a long-lived access token for GitHub connections.
-- The installation_id identifies which GitHub App installation to mint tokens for.
-- Other providers (GitLab, etc.) still use access_token_encrypted.

ALTER TABLE user_connections
    ALTER COLUMN access_token_encrypted DROP NOT NULL;

ALTER TABLE user_connections
    ADD COLUMN installation_id BIGINT;

-- Partial index for installation-based lookups
CREATE INDEX idx_user_connections_installation
    ON user_connections(installation_id)
    WHERE installation_id IS NOT NULL;
