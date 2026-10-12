-- Machine payments foundation.
-- See specs/machine-payments.md.

CREATE TABLE payment_accounts (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    owner_type TEXT NOT NULL CHECK (owner_type IN ('user', 'agent_identity', 'organization')),
    owner_id TEXT NOT NULL,
    rail TEXT NOT NULL CHECK (rail IN ('mpp_tempo', 'x402_base')),
    label TEXT NOT NULL,
    public_address TEXT,
    credential_encrypted BYTEA,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_payment_accounts_org_owner
    ON payment_accounts(org_id, owner_type, owner_id);
CREATE INDEX idx_payment_accounts_org_status
    ON payment_accounts(org_id, status);

CREATE TRIGGER update_payment_accounts_updated_at BEFORE UPDATE ON payment_accounts
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE payment_policies (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    payment_account_id UUID NOT NULL REFERENCES payment_accounts(id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL CHECK (subject_type IN ('user', 'agent_identity', 'agent', 'app', 'session', 'org')),
    subject_id TEXT NOT NULL,
    allowed_capabilities TEXT[] NOT NULL DEFAULT '{}',
    allowed_hosts TEXT[] NOT NULL DEFAULT '{}',
    rail_preference TEXT[] NOT NULL DEFAULT '{}',
    max_amount_usd_per_request DOUBLE PRECISION,
    max_amount_usd_per_turn DOUBLE PRECISION,
    max_amount_usd_per_day DOUBLE PRECISION,
    require_approval_above_usd DOUBLE PRECISION,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT payment_policy_positive_limits CHECK (
        (max_amount_usd_per_request IS NULL OR max_amount_usd_per_request > 0)
        AND (max_amount_usd_per_turn IS NULL OR max_amount_usd_per_turn > 0)
        AND (max_amount_usd_per_day IS NULL OR max_amount_usd_per_day > 0)
        AND (require_approval_above_usd IS NULL OR require_approval_above_usd > 0)
    )
);

CREATE INDEX idx_payment_policies_org_subject
    ON payment_policies(org_id, subject_type, subject_id);
CREATE INDEX idx_payment_policies_account
    ON payment_policies(payment_account_id);

CREATE TRIGGER update_payment_policies_updated_at BEFORE UPDATE ON payment_policies
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE payment_attempts (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    payment_account_id UUID REFERENCES payment_accounts(id) ON DELETE SET NULL,
    session_id UUID REFERENCES sessions(id) ON DELETE SET NULL,
    capability TEXT NOT NULL,
    operation TEXT NOT NULL,
    rail TEXT CHECK (rail IN ('mpp_tempo', 'x402_base')),
    amount_usd DOUBLE PRECISION NOT NULL DEFAULT 0,
    currency TEXT NOT NULL DEFAULT 'usd',
    target_url TEXT NOT NULL,
    request_hash TEXT,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'succeeded', 'failed', 'released')),
    error_message TEXT,
    receipt JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_payment_attempts_org_created
    ON payment_attempts(org_id, created_at DESC);
CREATE INDEX idx_payment_attempts_session_created
    ON payment_attempts(session_id, created_at DESC)
    WHERE session_id IS NOT NULL;

CREATE TRIGGER update_payment_attempts_updated_at BEFORE UPDATE ON payment_attempts
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
