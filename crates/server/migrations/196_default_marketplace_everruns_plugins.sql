-- Repoint the seeded default plugin marketplace from everruns/everruns to
-- everruns/plugins, where first-party plugins now live.
--
-- Only rows still pointing at the old repository move, so a marketplace an org
-- renamed, deleted or pointed elsewhere is left alone. The cached catalog and
-- sync markers are cleared so the next sync reads the new repository instead
-- of serving the old catalog. Installed plugins keep their pinned commit until
-- someone updates them explicitly.
--
-- New orgs get the new source from org_init::seed_default_plugin_marketplace.

UPDATE plugin_marketplaces
SET source = jsonb_set(source, '{repo}', '"everruns/plugins"'),
    catalog = NULL,
    last_synced_at = NULL,
    last_synced_sha = NULL
WHERE name = 'everruns'
  AND source_type = 'github'
  AND source ->> 'repo' = 'everruns/everruns';
