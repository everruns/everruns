-- Lifecycle of provider-held state for the OpenAI Agents API runtime backend
-- (EVE-1126). Deleting an Everruns session must also delete the OpenAI
-- session it drove, and a provider session the driver stops using (a new
-- agent definition, other provider credentials, a retention release) must not
-- outlive Everruns' reference to it.
--
-- The provider call cannot run inside the delete transaction, so every path
-- that drops a provider session id records a tombstone in the same
-- transaction: the session delete itself, any cascade that reaches
-- agents_api_sessions (agent, harness, or organization deletion), and a
-- checkpoint save that replaces or clears the id. A server background task
-- deletes the remote session through the API and removes the tombstone; a
-- failed call stays queued with backoff. Tombstones hold no credentials: the
-- task resolves the current key of `provider_key` at delete time, so a key
-- rotated on the same provider still works.

-- The Everruns LLM provider (its id) whose credentials own the provider
-- session. Plaintext, like provider_session_id, so lifecycle work never
-- decrypts the checkpoint.
ALTER TABLE agents_api_sessions ADD COLUMN provider_key TEXT;

CREATE TABLE agents_api_provider_deletions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- No foreign keys: the session, its organization, and even the provider
    -- may already be gone when the tombstone is processed.
    org_id BIGINT NOT NULL,
    session_id UUID NOT NULL,
    provider_key TEXT,
    provider_session_id TEXT NOT NULL UNIQUE,
    -- session_deleted | replaced | released
    reason TEXT NOT NULL,
    -- pending: queued or retrying; failed: gave up, kept for operators.
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Stable code of the last failure (never a provider response body).
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_agents_api_provider_deletions_due
    ON agents_api_provider_deletions (next_attempt_at)
    WHERE state = 'pending';

-- Retention sweeps look for idle checkpoints that still hold a provider session.
CREATE INDEX idx_agents_api_sessions_idle_provider
    ON agents_api_sessions (updated_at)
    WHERE provider_session_id IS NOT NULL;

CREATE FUNCTION agents_api_tombstone_provider_session() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        INSERT INTO agents_api_provider_deletions
            (org_id, session_id, provider_key, provider_session_id, reason)
        VALUES (OLD.org_id, OLD.session_id, OLD.provider_key, OLD.provider_session_id,
                'session_deleted')
        ON CONFLICT (provider_session_id) DO NOTHING;
        RETURN OLD;
    END IF;
    INSERT INTO agents_api_provider_deletions
        (org_id, session_id, provider_key, provider_session_id, reason)
    VALUES (OLD.org_id, OLD.session_id, OLD.provider_key, OLD.provider_session_id,
            CASE WHEN NEW.provider_session_id IS NULL THEN 'released' ELSE 'replaced' END)
    ON CONFLICT (provider_session_id) DO NOTHING;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER agents_api_sessions_tombstone_on_delete
    AFTER DELETE ON agents_api_sessions
    FOR EACH ROW
    WHEN (OLD.provider_session_id IS NOT NULL)
    EXECUTE FUNCTION agents_api_tombstone_provider_session();

CREATE TRIGGER agents_api_sessions_tombstone_on_replace
    AFTER UPDATE OF provider_session_id ON agents_api_sessions
    FOR EACH ROW
    WHEN (OLD.provider_session_id IS NOT NULL
          AND OLD.provider_session_id IS DISTINCT FROM NEW.provider_session_id)
    EXECUTE FUNCTION agents_api_tombstone_provider_session();
