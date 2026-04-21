# Everruns Claude Code plugin

A [Claude Code](https://docs.claude.com/en/docs/claude-code) plugin that wires
the [Everruns](https://everruns.com) agent platform into your terminal over MCP.
Claude Code gets enough context to create and run agents, manage harnesses and
sessions, and drive the full Everruns API through its `discover` / `execute`
tools.

- Product: <https://everruns.com>
- Docs: <https://docs.everruns.com>
- MCP endpoint used: `https://dev.everruns.com/mcp` (temporary; swap in
  `.mcp.json` once the stable URL lands)

## What's in the box

**MCP server** — registers the Everruns MCP endpoint as a remote HTTP server.
Claude Code handles OAuth 2.1 (PKCE) on first use.

**Skill `everruns`** — the reference Claude Code loads whenever you talk about
Everruns: core concepts (Harness, Agent, Capability, Session, Model, App), the
MCP tool surface, and concrete `discover` / `execute` recipes. See
[`skills/everruns/SKILL.md`](skills/everruns/SKILL.md) for the full content.

The intended way to use the plugin is natural language ("create an agent that
does X", "show me sessions from yesterday") — the skill gives Claude the
concepts and tool sequence.

**Slash commands** — thin shortcuts for the few flows worth a key binding:

| Command | Purpose |
|---|---|
| `/everruns:whoami` | Current user + active organization |
| `/everruns:agent-run <agent> <message>` | Start a session and send the first message |
| `/everruns:session-send <session_id> <message>` | Follow-up message |
| `/everruns:session-status <session_id>` | Poll status + recent events |
| `/everruns:discover <query>` / `--all` | Search the Everruns API catalog |
| `/everruns:execute <bash>` | Run a bash script where every Everruns API op is a builtin |

Everything else (listing, creating, configuring) flows through natural
language and `execute`.

## Install

### From GitHub (recommended)

The main repo doubles as a plugin **marketplace**
(`.claude-plugin/marketplace.json` at the root). Inside Claude Code:

```text
/plugin marketplace add everruns/everruns
/plugin install everruns@everruns
```

Pin to a branch or tag:

```text
/plugin marketplace add everruns/everruns#main
```

### From a local clone

```bash
claude plugin install ./integrations/claude-code-plugin
```

### Dev loop

Load the plugin in-place while iterating:

```bash
claude --plugin-dir ./integrations/claude-code-plugin
```

Run `/reload-plugins` inside Claude Code to pick up changes without restarting.

## First run

1. Launch Claude Code with the plugin installed.
2. Run `/everruns:whoami`. Claude Code will detect that the MCP server needs
   OAuth, open your browser, complete the PKCE flow, and cache the token.
3. Ask something in natural language, e.g. *"Create a research agent on
   Everruns and run it against this question: ..."* — the `everruns` skill
   gives Claude the concepts and the right tool sequence.

## Pointing at a different Everruns deployment

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

Any Everruns deployment that exposes `/mcp` plus OAuth discovery at
`/.well-known/oauth-authorization-server` works.

## Uninstall

```bash
claude plugin uninstall everruns
```
