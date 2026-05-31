# Everruns Plugin

This directory is the shared source of truth for the Everruns plugin in
Claude Code, Codex, and Cursor.

It wires the [Everruns](https://everruns.com) managed harnesses platform into
local agent tools. The shared payload is the Everruns MCP server config, the
`everruns` skill, and the slash commands for common session workflows.

- Product: <https://everruns.com>
- Docs: <https://docs.everruns.com>
- MCP endpoint used: `https://app.everruns.com/mcp`

## What It Includes

- `.claude-plugin/plugin.json` for Claude Code metadata
- `.codex-plugin/plugin.json` for Codex metadata
- `.cursor-plugin/plugin.json` for Cursor metadata
- `.mcp.json` for the Everruns MCP server (referenced by all three hosts)
- `skills/everruns/SKILL.md` for Everruns concepts and workflows
- `commands/` for the main Everruns slash commands
- `assets/` for Codex and Cursor marketplace metadata

## Install

### Claude Code

From GitHub marketplace:

```text
/plugin marketplace add everruns/everruns
/plugin install everruns@everruns-dev
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

### Cursor

Cursor discovers the plugin through the marketplace manifest at
`.cursor-plugin/marketplace.json` in the repository root, which points at
`./plugins/everruns`. Submit the repository to the
[Cursor Marketplace](https://cursor.com/marketplace/publish) for public listing,
or load it locally during development by opening the repo in Cursor and
enabling the plugin from the marketplace UI.

## First Run

1. Install the plugin in Claude Code, Codex, or Cursor.
2. Run `/everruns:whoami` (Claude/Codex) or invoke the `whoami` command (Cursor).
3. Complete the OAuth flow in the browser if prompted.
4. Ask for an Everruns task in natural language.

## Pointing At Another Everruns Deployment

Edit `plugins/everruns/.mcp.json` and replace the `url` value with your
deployment's `/mcp` endpoint. All three hosts (Claude Code, Codex, Cursor) read
this file, so a single change keeps them aligned.
