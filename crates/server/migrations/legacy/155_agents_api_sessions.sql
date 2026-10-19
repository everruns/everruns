-- Durable orchestration state for the opt-in OpenAI Agents API runtime
-- backend (EVE-1123). One row per Everruns session bound to a provider
-- session: the provider session id, stream cursor, item correlations, and the
-- input and tool-result outboxes. The payload holds private tool arguments
-- and results, so it is encrypted. Session deletion owns retention; forks
-- never copy provider identities or ownership.
CREATE TABLE agents_api_sessions (
    -- The secret-rotation registry addresses rows by a single UUID.
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    owner UUID NOT NULL,
    lease_until TIMESTAMPTZ NOT NULL,
    -- Plaintext copy of the provider's opaque session id so lifecycle work
    -- (deletion, retention) can address the remote session without decrypting.
    provider_session_id TEXT,
    payload_encrypted BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
