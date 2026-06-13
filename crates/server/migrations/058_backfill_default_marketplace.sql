-- Backfill the default plugin marketplace for every existing organization.
--
-- For each org that does not already have a marketplace named "everruns",
-- insert a row: name "everruns", source_type "github",
-- source {"repo": "everruns/everruns"}, status "active", catalog NULL
-- (unsynced until first sync).
--
-- Going forward, new orgs receive this row at org-creation time via
-- org_init::seed_default_plugin_marketplace (see crates/server/src/org_init.rs).
-- This migration is the one-time backfill for orgs created before that code
-- shipped.
--
-- Idempotent: safe to run on a fresh database or one where some orgs already
-- have the marketplace (the WHERE NOT EXISTS guard prevents duplicate names).

INSERT INTO plugin_marketplaces (
    org_id,
    public_id,
    name,
    source_type,
    source
)
SELECT
    o.org_id,
    'plgmkt_' || replace(uuidv7()::text, '-', ''),
    'everruns',
    'github',
    '{"repo": "everruns/everruns"}'::jsonb
FROM organizations o
WHERE NOT EXISTS (
    SELECT 1
    FROM plugin_marketplaces pm
    WHERE pm.org_id = o.org_id
      AND pm.name = 'everruns'
);
