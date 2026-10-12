-- User MCP servers: a personal sign-in to a catalog server puts it on the list.
--
-- Decision (knowledge/integrations/user-mcp-servers.md, D8): from now on the
-- OAuth callback adds a user-owned row pointing at the catalog preset when a
-- person signs in to it (`Database::list_catalog_server_for_owner`). This
-- backfills the same rows for sign-ins made before, so every personal grant on
-- a catalog server shows up in My MCP servers and reaches agents with the User
-- MCP servers capability.
--
-- Scope matches the callback: grants keyed `mcp_oauth_<preset uuid>` held by an
-- active end-user virtual user, on an active catalog preset (owner NULL) of the
-- same organization, with no listed row for that preset yet. Agent grants
-- (service virtual users), archived or deleted presets, custom servers' own
-- grants and lists already at the 50-server cap are left alone.
--
-- Name rule (same as `free_user_server_name` in Rust): the preset's name, or
-- `<name>-2`, `<name>-3`, ... when one of the person's listed servers already
-- produces the same tool prefix (lowercase, non-alphanumerics as `_`). Grants
-- are processed per person in the order they were connected, then by preset
-- id, so the outcome is deterministic. Done row by row so names picked earlier
-- in the backfill count as taken.

DO $$
DECLARE
    g RECORD;
    taken TEXT[];
    candidate TEXT;
    suffix TEXT;
    n INT;
BEGIN
    FOR g IN
        SELECT * FROM (
            SELECT DISTINCT ON (v.id, p.id)
                v.org_id, v.id AS owner, p.id AS preset_id, p.name, p.description,
                p.url, p.transport_type, c.created_at AS connected_at
            FROM virtual_user_connections c
            JOIN virtual_users v ON v.id = c.virtual_user_id
            JOIN mcp_servers p
              ON p.org_id = v.org_id
             AND 'mcp_oauth_' || p.id::text = c.provider
            WHERE c.provider LIKE 'mcp\_oauth\_%'
              AND v.usage = 'end_user'
              AND v.status = 'active'
              AND p.owner_virtual_user_id IS NULL
              AND p.status = 'active'
              AND NOT EXISTS (
                  SELECT 1 FROM mcp_servers u
                  WHERE u.org_id = v.org_id
                    AND u.owner_virtual_user_id = v.id
                    AND u.catalog_mcp_server_id = p.id
                    AND u.status IN ('active', 'disabled')
              )
            ORDER BY v.id, p.id, c.created_at
        ) grants
        ORDER BY owner, connected_at, preset_id
    LOOP
        SELECT COALESCE(array_agg(lower(regexp_replace(name, '[^[:alnum:]]', '_', 'g'))), '{}')
          INTO taken
          FROM mcp_servers
         WHERE org_id = g.org_id
           AND owner_virtual_user_id = g.owner
           AND status IN ('active', 'disabled');

        CONTINUE WHEN cardinality(taken) >= 50;

        candidate := NULL;
        IF NOT (lower(regexp_replace(g.name, '[^[:alnum:]]', '_', 'g')) = ANY (taken)) THEN
            candidate := g.name;
        ELSE
            FOR n IN 2..51 LOOP
                suffix := '-' || n;
                candidate := rtrim(left(g.name, 64 - length(suffix)), '-_') || suffix;
                EXIT WHEN NOT (lower(regexp_replace(candidate, '[^[:alnum:]]', '_', 'g')) = ANY (taken));
                candidate := NULL;
            END LOOP;
        END IF;

        CONTINUE WHEN candidate IS NULL;

        INSERT INTO mcp_servers (
            org_id, owner_virtual_user_id, name, description, url, transport_type,
            api_key_set, headers, settings, catalog_mcp_server_id, deferred
        )
        VALUES (
            g.org_id, g.owner, candidate, g.description, g.url, g.transport_type,
            FALSE, '{}'::jsonb, '{"auth_mode": "none"}'::jsonb, g.preset_id, TRUE
        );
    END LOOP;
END
$$;
