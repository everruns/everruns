-- everruns_app_org_installation
--
-- The shared Everruns GitHub App becomes an org-level link (D3 in
-- knowledge/integrations/github-model.md): an org admin links an installation,
-- stored as an organization connection (provider 'github', connection_type
-- 'everruns_app', on the org's hidden organization virtual user), and agents
-- choose it explicitly.
--
-- Decisions:
-- - One installation belongs to at most one org row, across every org: the
--   webhook route finds the org by installation id, so two orgs holding the
--   same installation would make routing ambiguous. Agent rows that point at
--   the installation are owner_scope 'virtual_user' and stay outside the index.
-- - No CHECK change: connection_type has no CHECK (varchar(20) fits
--   'everruns_app'), and owner_scope already allows 'organization'.
CREATE UNIQUE INDEX IF NOT EXISTS virtual_user_connections_github_org_installation
    ON virtual_user_connections (installation_id)
    WHERE provider = 'github' AND owner_scope = 'organization';
