-- Native-auth account-recovery tokens: password reset + email verification.
--
-- Both tables follow the org_invitations hashed-token model (migration 081):
-- the raw token is surfaced exactly once in an emailed link and is NEVER
-- persisted. Only its SHA-256 hash is stored, so a database leak cannot be
-- replayed to take over an account. Tokens are single-use (claimed via the
-- `used_at` column in one atomic UPDATE ... WHERE used_at IS NULL) and
-- short-lived (TTL enforced via `expires_at`), bounding the window in which a
-- leaked link is useful. Rows are retained after use/expiry for audit; a
-- background sweep can prune them later.

CREATE TABLE password_reset_tokens (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- SHA-256 hash of the raw reset token. The raw token is never persisted.
    token_hash TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    -- Set once when the token is consumed; enforces single use.
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_password_reset_tokens_user_id ON password_reset_tokens(user_id);
CREATE UNIQUE INDEX idx_password_reset_tokens_token_hash ON password_reset_tokens(token_hash);

CREATE TABLE email_verification_tokens (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- SHA-256 hash of the raw verification token. The raw token is never persisted.
    token_hash TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    -- Set once when the token is consumed; enforces single use.
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_email_verification_tokens_user_id ON email_verification_tokens(user_id);
CREATE UNIQUE INDEX idx_email_verification_tokens_token_hash ON email_verification_tokens(token_hash);
