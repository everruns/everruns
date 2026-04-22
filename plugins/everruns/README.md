# Everruns Plugin

This directory is the shared source of truth for the Everruns plugin in both
Claude Code and Codex.

It wires the [Everruns](https://everruns.com) agent platform into local agent
tools over MCP. The shared payload is the Everruns MCP server config, the
`everruns` skill, and the slash commands for common session workflows.
Use `query` for read-only inspection and `execute` for workflows that create,
update, delete, or otherwise have side effects.

- Product: <https://everruns.com>
- Docs: <https://docs.everruns.com>
- MCP endpoint used: `https://dev.everruns.com/mcp`

## What It Includes

- `.claude-plugin/plugin.json` for Claude Code metadata
- `.codex-plugin/plugin.json` for Codex metadata
- `.mcp.json` for the Everruns MCP server
- `skills/everruns/SKILL.md` for Everruns concepts and workflows
- `commands/` for the main Everruns slash commands
- `assets/` for Codex UI metadata

## Install

### Claude Code

From GitHub marketplace:

```text
/plugin marketplace add everruns/everruns
/plugin install everruns@everruns
```

From a local clone:

```bash
claude plugin install ./plugins/everruns
```

Dev loop:

```bash
claude --plugin-dir ./plugins/everruns
```

### Codex

Codex discovers the plugin through the workspace marketplace at
`.agents/plugins/marketplace.json`, which points at `./plugins/everruns`.

## First Run

1. Install the plugin in Claude Code or expose it through the Codex marketplace.
2. Run `/everruns:whoami`.
3. Complete the OAuth flow in the browser if prompted.
4. Ask for an Everruns task in natural language.

## Pointing At Another Everruns Deployment

Edit `plugins/everruns/.mcp.json` and replace the `url` value with your
deployment's `/mcp` endpoint.
