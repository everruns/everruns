-- Source sync reads through a development GitHub account, separate from the
-- Agent's runtime identity: a person's own sign-in (no installation) or an
-- organization link to the shared App. Existing bindings keep the Agent's
-- identity ('agent') until they are reconnected.
ALTER TABLE agent_github_sources
    ALTER COLUMN installation_id DROP NOT NULL,
    ADD COLUMN github_account TEXT NOT NULL DEFAULT 'agent'
        CHECK (github_account IN ('agent', 'personal', 'organization'));
