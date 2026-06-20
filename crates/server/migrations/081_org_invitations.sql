-- OSS-owned organization invitations (EVE-602).
--
-- Everruns owns the full org membership lifecycle, including pending invites and
-- acceptance. Invite tokens are never stored in raw form: only a SHA-256 hash is
-- persisted, and the raw token is surfaced once at creation as an invite URL.
-- Email delivery is optional and handled above this table; delivery failure must
-- not corrupt invite state, so this table has no email-delivery columns.

CREATE TABLE org_invitations (
    id BIGSERIAL PRIMARY KEY,
    -- Stable public identifier returned to clients (orginv_<32-hex>).
    public_id TEXT UNIQUE NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    -- Invited email, normalized (trimmed + lowercased) for comparison.
    email TEXT NOT NULL,
    -- Invited role: must match org membership roles.
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
    -- User who created the invite.
    invited_by UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- SHA-256 hash of the raw token. The raw token is never persisted.
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    -- User who accepted the invite (NULL until accepted).
    accepted_by UUID REFERENCES users(id) ON DELETE SET NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT org_invitations_public_id_format
        CHECK (public_id ~ '^orginv_[0-9a-f]{32}$')
);

CREATE INDEX idx_org_invitations_org_id ON org_invitations(org_id);
CREATE UNIQUE INDEX idx_org_invitations_token_hash ON org_invitations(token_hash);

-- At most one outstanding (pending, not accepted, not revoked) invite per
-- (org, email). Accepted/revoked rows are retained for audit and are excluded
-- from this constraint so a fresh invite can be issued after they resolve.
CREATE UNIQUE INDEX idx_org_invitations_pending_email
    ON org_invitations(org_id, email)
    WHERE accepted_at IS NULL AND revoked_at IS NULL;

CREATE TRIGGER update_org_invitations_updated_at BEFORE UPDATE ON org_invitations
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
