# Plugin test fixtures

Local marketplace fixture for the plugins subsystem (`specs/plugins.md`).
Not shipped; consumed by server and runtime tests and by manual smoke tests.

- `.claude-plugin/marketplace.json` — marketplace manifest with
  relative-path plugin sources, so the directory works as a local
  marketplace source end to end.
- `microsoft-docs/` — Everruns-authored variant of the public Microsoft
  Docs plugin (https://github.com/MicrosoftDocs/mcp), pointing at the same
  public, unauthenticated MCP server (`https://learn.microsoft.com/api/mcp`).
  It intentionally exercises every v1 component mapping (manifest metadata,
  `skills/`, `commands/`, `agents/`, `.mcp.json`) and carries an
  `interface` block that the Everruns host must ignore with an install
  warning. The fixture targets the Everruns host only; Claude Code would
  reject the `interface` field in `.claude-plugin/plugin.json`.

- `oauth-mail/` — minimal fixture whose `.mcp.json` marks its MCP server
  `"auth": "oauth"`. It exercises the plugin OAuth-anchor install path
  (`crates/server/.../plugins/oauth_anchor.rs`): install creates a disabled
  anchor `mcp_servers` row, assigns a host-owned `mcp_oauth_*` provider id,
  and surfaces it in the connections API; uninstall removes it. The URL is a
  non-routable `.test` host and is never contacted by automated tests.

Automated tests must not call the live MCP server.
