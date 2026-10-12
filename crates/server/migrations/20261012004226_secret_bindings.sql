-- secret_bindings
-- Bound secrets (knowledge/security/bound-secrets.md): a secret stored with the
-- URL and auth form it may be sent with. Bound values are write-only; the
-- shell gets a placeholder and Bashkit swaps the value into request headers
-- only for the bound URL.

ALTER TABLE session_secrets
    ADD COLUMN binding JSONB;

COMMENT ON COLUMN session_secrets.binding IS
    'SecretBinding {url, auth}. NULL: an ordinary secret the model may read. Set: write-only, injected only into requests to url.';

-- Agent-scoped bound secrets live with the agent's other write-only
-- credentials (the Credentials tab), as a second credential kind: an HTTP
-- binding names an env var and a SecretBinding instead of an MCP tool
-- parameter. The table keeps its name; its MCP columns are now set only for
-- the mcp_parameter kind.
ALTER TABLE agent_mcp_secret_bindings
    ADD COLUMN kind VARCHAR(32) NOT NULL DEFAULT 'mcp_parameter',
    ADD COLUMN env_name VARCHAR(128),
    ADD COLUMN binding JSONB,
    ALTER COLUMN mcp_server_name DROP NOT NULL,
    ALTER COLUMN mcp_server_url DROP NOT NULL,
    ALTER COLUMN tool_name DROP NOT NULL,
    ALTER COLUMN parameter_name DROP NOT NULL,
    ADD CONSTRAINT agent_mcp_secret_bindings_kind_check CHECK (
        (kind = 'mcp_parameter'
            AND mcp_server_name IS NOT NULL AND mcp_server_url IS NOT NULL
            AND tool_name IS NOT NULL AND parameter_name IS NOT NULL
            AND env_name IS NULL AND binding IS NULL)
        OR (kind = 'http'
            AND env_name IS NOT NULL AND binding IS NOT NULL
            AND mcp_server_name IS NULL AND mcp_server_url IS NULL
            AND tool_name IS NULL AND parameter_name IS NULL)
    );

CREATE UNIQUE INDEX agent_mcp_secret_bindings_http_env_name_key
    ON agent_mcp_secret_bindings (agent_id, env_name)
    WHERE kind = 'http';

COMMENT ON TABLE agent_mcp_secret_bindings IS
    'Encrypted Agent-scoped write-only credentials. kind mcp_parameter: injected into one MCP tool argument after tool-call persistence. kind http: the shell sees env_name as a placeholder, replaced in requests to binding.url only.';
