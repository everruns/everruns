# Everruns Claude Code plugin

A [Claude Code](https://docs.claude.com/en/docs/claude-code) plugin that wires
the Everruns MCP endpoint into your terminal. Use it to create and run agents,
manage harnesses, poll sessions, and explore the Everruns API catalog without
leaving Claude Code.

The plugin targets the hosted dev environment
(`https://dev.everruns.com/mcp`). Point it at a different deployment by
editing `.mcp.json`.

## What's in the box

- **MCP server** — adds `https://dev.everruns.com/mcp` as a remote MCP server.
  Claude Code handles OAuth 2.1 (with PKCE) automatically: on first use it opens
  a browser so you can sign in to Everruns, then caches the refresh token.
- **Slash commands** (prefixed `/everruns:`):

  | Command | Purpose |
  |---|---|
  | `/everruns:whoami` | Current user + active organization |
  | `/everruns:switch-org <id>` | Switch the active organization |
  | `/everruns:list-agents [query]` | List agents |
  | `/everruns:get-agent <id>` | Agent detail |
  | `/everruns:create-agent <name> [flags]` | Create an agent |
  | `/everruns:agent-run <agent_id> <message>` | Start a session and send the first message |
  | `/everruns:session-send <session_id> <message>` | Follow-up message |
  | `/everruns:session-status <session_id>` | Poll status + recent events |
  | `/everruns:list-sessions [flags]` | Recent sessions |
  | `/everruns:list-harnesses` | List harnesses |
  | `/everruns:get-harness <id>` | Harness detail |
  | `/everruns:create-harness <name> [flags]` | Create a harness |
  | `/everruns:list-models` | Available LLM models |
  | `/everruns:discover <query>` / `--all` | Search the Everruns API catalog |
  | `/everruns:execute <bash>` | Run a bash script where every Everruns API op is a builtin |

  The commands are thin shells over the Everruns MCP tools
  (`me`, `agent_run`, `session_send_message`, `session_get_status`, `discover`,
  `execute`, plus the ~50 API builtins surfaced via `execute`). The model fills
  in parameters from natural language and paginates sensibly.

## Install

### From a local clone

```bash
claude plugin install ./integrations/claude-code-plugin
```

or, from anywhere:

```bash
claude plugin install /path/to/everruns/integrations/claude-code-plugin
```

### Dev loop

While iterating on the plugin, either re-install after each change or use
`--plugin-dir` to load it in-place:

```bash
claude --plugin-dir ./integrations/claude-code-plugin
```

Run `/reload-plugins` inside Claude Code to pick up changes without restarting.

## First run

1. Launch Claude Code with the plugin installed.
2. Run `/everruns:whoami`. Claude Code will detect that the MCP server requires
   OAuth, open your browser, complete the authorization code + PKCE flow, and
   store the resulting token.
3. You should see your Everruns user profile and active organization.
4. Try `/everruns:list-agents` and `/everruns:agent-run <agent_id> "hello"`.

## Pointing at a different Everruns instance

Edit `.mcp.json`:

```json
{
  "mcpServers": {
    "everruns": {
      "type": "http",
      "url": "https://your-everruns-host.example.com/mcp"
    }
  }
}
```

Any Everruns deployment that exposes `/mcp` and the OAuth discovery metadata at
`/.well-known/oauth-authorization-server` works — that's the standard Everruns
routing (see `specs/mcp.md` in the main repo).

## Multi-organization accounts

`switch_organization` is advisory because the MCP transport is stateless.
Commands that hit org-scoped tools accept an `--organization_id org_{32-hex}`
flag — pass it explicitly when you want a one-off call against a non-default
org. For the common case, the token's default organization is used.

## Uninstall

```bash
claude plugin uninstall everruns
```

## Troubleshooting

- **OAuth loop / 401 responses** — sign out of the Everruns web UI, then retry
  the command so Claude Code re-registers a fresh OAuth client.
- **`tool not found`** — run `/reload-plugins` and verify the MCP server shows
  up in `claude mcp list`.
- **Wrong org** — run `/everruns:whoami`, then `/everruns:switch-org <id>` or
  pass `--organization_id` per call.
