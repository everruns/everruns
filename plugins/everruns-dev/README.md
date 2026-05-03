# Everruns(Dev) Plugin

This directory is the shared source of truth for the Everruns(Dev) plugin in both
Claude Code and Codex.

It wires the [Everruns](https://everruns.com) dev managed harnesses platform into
local agent tools. The shared payload is the Everruns(Dev) MCP server config, the
`everruns-dev` skill, and the slash commands for common session workflows.

- Product: <https://everruns.com>
- Docs: <https://docs.everruns.com>
- MCP endpoint used: `https://dev.everruns.com/mcp`

## What It Includes

- `.claude-plugin/plugin.json` for Claude Code metadata
- `.codex-plugin/plugin.json` for Codex metadata
- `.mcp.json` for the Everruns(Dev) MCP server
- `skills/everruns-dev/SKILL.md` for Everruns(Dev) concepts and workflows
- `commands/` for the main Everruns(Dev) slash commands
- `assets/` for Codex UI metadata

## Install

### Claude Code

From GitHub marketplace:

```text
/plugin marketplace add everruns/everruns
/plugin install everruns-dev@everruns-dev
```

From a local clone:

```bash
claude plugin install ./plugins/everruns-dev
```

Dev loop:

```bash
claude --plugin-dir ./plugins/everruns-dev
```

### Codex

Codex discovers the plugin through the workspace marketplace at
`.agents/plugins/marketplace.json`, which points at `./plugins/everruns-dev`.

## First Run

1. Install the plugin in Claude Code or expose it through the Codex marketplace.
2. Run `/everruns-dev:whoami`.
3. Complete the OAuth flow in the browser if prompted.
4. Ask for an Everruns(Dev) task in natural language.

## Pointing At Another Everruns(Dev) Deployment

Edit `plugins/everruns-dev/.mcp.json` and replace the `url` value with your
deployment's `/mcp` endpoint.
