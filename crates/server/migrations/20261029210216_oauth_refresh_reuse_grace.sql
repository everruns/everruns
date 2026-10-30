-- MCP OAuth refresh tokens: keep a rotated token instead of deleting it, so a
-- replay inside the reuse grace window (a lost response, or two sessions of one
-- host refreshing together) is honoured once instead of forcing a reconnect, and
-- a replay after the window is detected as reuse and kills its token family.
-- See storage::repositories::auth::REFRESH_REUSE_GRACE_SECS.
ALTER TABLE oauth_refresh_tokens
    ADD COLUMN rotated_at timestamp with time zone,
    ADD COLUMN grace_used boolean DEFAULT false NOT NULL,
    ADD COLUMN family_id uuid;

CREATE INDEX idx_oauth_refresh_tokens_family_id
    ON oauth_refresh_tokens (family_id)
    WHERE family_id IS NOT NULL;
