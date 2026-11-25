-- Repoint the seeded default plugin marketplace from everruns/plugins to the
-- Everruns Plugin directory (everruns/plugin-directory): reviewed plugins in
-- the Agent Plugins format, each pinned to its source, license-checked, and
-- tested in Everruns. everruns/plugins is where Everruns writes its own
-- plugins; the directory copies them along with third-party ones.
--
-- Only rows still pointing at the old repository move, so a marketplace an org
-- renamed, deleted or pointed elsewhere is left alone. The cached catalog and
-- sync markers are cleared so the next sync reads the directory. Installed
-- plugins keep their pinned commit until someone updates them explicitly.
--
-- New orgs get the new source from org_init::seed_default_plugin_marketplace.

UPDATE plugin_marketplaces
SET source = jsonb_set(source, '{repo}', '"everruns/plugin-directory"'),
    catalog = NULL,
    last_synced_at = NULL,
    last_synced_sha = NULL
WHERE name = 'everruns'
  AND source_type = 'github'
  AND source ->> 'repo' = 'everruns/plugins';
