-- Read-only share links for eval runs (specs/evals.md, specs/public-endpoints.md).
--
-- A share token unlocks an anonymous, read-only view of exactly one eval run via
-- `GET /v1/public/eval-runs/{token}`. Like personal access tokens, the raw token
-- is surfaced once at creation and stored only as a SHA-256 hash. Org-scoped for
-- management (mint/revoke), but the public read resolves the run from the token
-- row itself (no auth). `revoked_at`/`expires_at` disable a link without deleting
-- the row (audit); deleting the run cascades its share tokens away.
CREATE TABLE eval_run_share_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL,
    public_id TEXT NOT NULL,
    eval_run_id UUID NOT NULL,
    -- SHA-256 hash of the token; the raw token is never stored.
    token_hash TEXT NOT NULL,
    -- Display prefix (e.g. `evr_share_abc12345...`), safe to show in the UI.
    token_prefix TEXT NOT NULL,
    created_by UUID,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(org_id, public_id),
    CONSTRAINT eval_run_share_tokens_public_id_format
        CHECK (public_id ~ '^evalshare_[0-9a-f]{32}$'),
    CONSTRAINT eval_run_share_tokens_org_id_fk
        FOREIGN KEY (org_id) REFERENCES organizations(org_id),
    CONSTRAINT eval_run_share_tokens_eval_run_id_fk
        FOREIGN KEY (eval_run_id) REFERENCES eval_runs(id) ON DELETE CASCADE,
    CONSTRAINT eval_run_share_tokens_created_by_fk
        FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE SET NULL
);

-- Public read hashes the presented token and looks it up here.
CREATE UNIQUE INDEX idx_eval_run_share_tokens_token_hash
    ON eval_run_share_tokens(token_hash);
-- "Does this run have an active share?" / revoke-all-for-run.
CREATE INDEX idx_eval_run_share_tokens_run
    ON eval_run_share_tokens(eval_run_id);
CREATE INDEX idx_eval_run_share_tokens_org_id
    ON eval_run_share_tokens(org_id);

CREATE TRIGGER update_eval_run_share_tokens_updated_at
    BEFORE UPDATE ON eval_run_share_tokens
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
