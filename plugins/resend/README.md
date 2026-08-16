# Resend Plugin

Send emails through the official [Resend](https://resend.com) remote MCP
server (`https://mcp.resend.com/mcp`) with OAuth.

This plugin is the end-to-end proof for Everruns' inbound plugin support
(`knowledge/integrations/plugins.md`): a cross-host plugin package whose MCP server requires
OAuth, installed into Everruns as a stable `plugin:{install_id}` capability.

## What It Includes

- `plugin.json` and `mcp.json`, portable Agent Plugins v1 metadata
- `.claude-plugin/plugin.json`, Claude Code plugin manifest
- `.mcp.json`, the Resend remote MCP server. The `"auth": "oauth"` field is
  an Everruns extension marking the server as OAuth-authenticated; other
  hosts (Claude Code, Cursor) ignore it and negotiate OAuth at the protocol
  level on 401.
- `skills/resend/SKILL.md`, email-sending guidance for the agent
- `commands/send-email.md`, `/send-email` command

## Install in Everruns

Install `resend` from the default `everruns` marketplace (Settings →
Plugins), assign the Resend capability to an agent, then connect
Resend under **Settings → Connections**. The OAuth client is registered
dynamically against `api.resend.com`; tokens are stored encrypted and
refreshed automatically.

## Install in Claude Code

```text
/plugin marketplace add everruns/everruns
/plugin install resend@everruns-dev
```

Claude Code runs its own OAuth flow on first use of the server.
