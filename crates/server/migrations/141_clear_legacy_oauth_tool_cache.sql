UPDATE mcp_servers
SET cached_tools = '[]'::jsonb,
    tools_cached_at = NULL
WHERE (
        settings ->> 'auth_mode' IN ('o_auth', 'oauth')
        OR (
            settings -> 'auth_mode' IS NULL
            AND settings -> 'oauth' IS NOT NULL
        )
    )
  AND (
      cached_tools <> '[]'::jsonb
      OR tools_cached_at IS NOT NULL
  );
