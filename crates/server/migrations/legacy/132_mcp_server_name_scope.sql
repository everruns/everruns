-- EVE-964: scope MCP server name uniqueness to the organization and to live rows.
--
-- 001_base_schema.sql created `idx_mcp_servers_name` as a UNIQUE index on
-- `name` alone. That was wrong twice over:
--
--   1. Not org-scoped. One organization creating a server called `github`
--      stopped every other organization from ever using that name, and the
--      resulting conflict disclosed that some other tenant held it.
--   2. Counted dead rows. `delete_mcp_server` archives rather than removes
--      (`status = 'archived'`), so a deleted server held its name forever.
--
-- `active` and `disabled` are the live states: a disabled server is
-- configuration a user can re-enable, so reusing its name would create an
-- ambiguous pair. `archived` and `deleted` release the name.
--
-- Any duplicate rows that the old global index made impossible cannot exist, so
-- this needs no data cleanup — the new index is strictly more permissive.

DROP INDEX IF EXISTS idx_mcp_servers_name;

CREATE UNIQUE INDEX idx_mcp_servers_org_name_live
    ON mcp_servers (org_id, name)
    WHERE status IN ('active', 'disabled');
