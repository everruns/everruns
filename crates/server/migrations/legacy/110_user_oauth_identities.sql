-- Allow one user account to authenticate through multiple OAuth providers.
-- The provider subject is globally unique, and one account may bind at most
-- one subject from each provider.
CREATE TABLE user_oauth_identities (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, provider_id),
    UNIQUE (user_id, provider)
);

INSERT INTO user_oauth_identities (user_id, provider, provider_id)
SELECT id, auth_provider, auth_provider_id
FROM users
WHERE auth_provider IS NOT NULL
  AND auth_provider_id IS NOT NULL;

-- Keep the legacy user columns as the account's original/display provider
-- while ensuring new OAuth-created users also get a durable identity row.
CREATE FUNCTION sync_user_oauth_identity()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.auth_provider IS NOT NULL AND NEW.auth_provider_id IS NOT NULL THEN
        INSERT INTO user_oauth_identities (user_id, provider, provider_id)
        VALUES (NEW.id, NEW.auth_provider, NEW.auth_provider_id)
        ON CONFLICT DO NOTHING;

        IF NOT EXISTS (
            SELECT 1
            FROM user_oauth_identities
            WHERE user_id = NEW.id
              AND provider = NEW.auth_provider
              AND provider_id = NEW.auth_provider_id
        ) THEN
            RAISE EXCEPTION 'OAuth identity conflicts with an existing binding'
                USING ERRCODE = 'unique_violation';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sync_user_oauth_identity_after_write
    AFTER INSERT OR UPDATE OF auth_provider, auth_provider_id ON users
    FOR EACH ROW EXECUTE FUNCTION sync_user_oauth_identity();
