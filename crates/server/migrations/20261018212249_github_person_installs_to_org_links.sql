-- github_person_installs_to_org_links
--
-- Step 3b of knowledge/integrations/github-model.md: shared Everruns GitHub
-- App installations stored on a virtual user's own `github` row (the pre-D3
-- shape: connection_type 'oauth' with an installation_id) move to the org.
--
-- Decisions:
-- - Each installation becomes an org link (an organization connection with
--   connection_type 'everruns_app' on its own hidden organization virtual user,
--   as `create_organization_connection` writes it) in the org of the oldest row
--   that holds it. The person proved access when they installed it, and
--   installations belong to the org (open question 1, decided 2026-10-12).
-- - One installation belongs to at most one org (the 3a partial unique index),
--   so an installation another org already links, or one held in several
--   orgs, goes to the first org only. Rows in other orgs are removed.
-- - An agent's legacy row becomes the agent's explicit choice of that link,
--   exactly as `UseAgentEverrunsApp` writes it, so its tools and GitHub
--   triggers keep working. An agent whose installation went to another org
--   loses the row: nothing falls back to the shared App.
-- - A person's legacy row is removed. It made their turns act as the shared
--   App's bot (the finding in the design); a person's own GitHub sign-in is
--   step 4, and holds the person's token, not an installation.
-- - Rows on deleted or archived virtual users are migrated the same way: the
--   link is the org's either way, and nothing reads the removed rows.

DO $$
DECLARE
    legacy record;
    link_id uuid;
    owner_id uuid;
BEGIN
    -- Oldest holder first, across orgs, so the first org keeps it.
    FOR legacy IN
        WITH holders AS (
            SELECT DISTINCT ON (c.installation_id, v.org_id)
                   c.installation_id, v.org_id, c.provider_user_id, c.provider_username,
                   c.scopes, c.provider_metadata, c.created_at
            FROM virtual_user_connections c
            JOIN virtual_users v ON v.id = c.virtual_user_id
            WHERE c.provider = 'github' AND c.connection_type = 'oauth'
              AND c.owner_scope = 'virtual_user' AND c.installation_id IS NOT NULL
            ORDER BY c.installation_id, v.org_id, c.created_at
        )
        SELECT * FROM holders ORDER BY created_at
    LOOP
        SELECT c.id INTO link_id
        FROM virtual_user_connections c
        WHERE c.provider = 'github' AND c.owner_scope = 'organization'
          AND c.installation_id = legacy.installation_id;
        IF link_id IS NOT NULL THEN
            CONTINUE;
        END IF;

        owner_id := uuidv7();
        INSERT INTO virtual_users (org_id, id, name, description, status, usage)
        VALUES (
            legacy.org_id, owner_id,
            'Everruns GitHub App: '
                || COALESCE(NULLIF(btrim(legacy.provider_username), ''),
                            'installation ' || legacy.installation_id),
            'Hidden owner for an organization connection', 'active', 'organization'
        );
        INSERT INTO virtual_user_connections
            (virtual_user_id, provider, connection_type, owner_scope, name,
             provider_user_id, provider_username, scopes, installation_id,
             provider_metadata)
        VALUES (
            owner_id, 'github', 'everruns_app', 'organization',
            COALESCE(NULLIF(btrim(legacy.provider_username), ''),
                     'installation ' || legacy.installation_id),
            legacy.provider_user_id, legacy.provider_username, legacy.scopes,
            legacy.installation_id,
            CASE WHEN legacy.provider_metadata ? 'account_type'
                 THEN jsonb_build_object('account_type', legacy.provider_metadata -> 'account_type')
                 ELSE '{}'::jsonb END
        );
    END LOOP;

    -- Agents' legacy rows become their choice of the link in their own org.
    UPDATE virtual_user_connections c
    SET connection_type = 'everruns_app',
        provider_metadata = COALESCE(c.provider_metadata, '{}'::jsonb)
            || jsonb_build_object('org_connection_id', link.id)
    FROM virtual_users v, virtual_user_connections link, virtual_users link_owner
    WHERE v.id = c.virtual_user_id AND v.usage = 'service'
      AND c.provider = 'github' AND c.connection_type = 'oauth'
      AND c.owner_scope = 'virtual_user' AND c.installation_id IS NOT NULL
      AND link.provider = 'github' AND link.owner_scope = 'organization'
      AND link.installation_id = c.installation_id
      AND link_owner.id = link.virtual_user_id AND link_owner.org_id = v.org_id;

    -- Everything left: people's rows, and agents whose installation another
    -- org holds.
    DELETE FROM virtual_user_connections c
    WHERE c.provider = 'github' AND c.connection_type = 'oauth'
      AND c.owner_scope = 'virtual_user' AND c.installation_id IS NOT NULL;
END $$;
