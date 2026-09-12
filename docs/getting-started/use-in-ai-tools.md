---
title: Use in AI Tools
description: Set up Everruns in AI tools through the Everruns plugin.
---

# Use in AI Tools (Everruns Plugin)

The `everruns` plugin connects Claude Code, Codex, and Cursor to any Everruns deployment over MCP. It ships skills, slash commands, and agents; Codex setup additionally uses the plugin's `defaultPrompt` and `description`.

## Quickstart (local Everruns)

```bash
just up
just agent-auth PROVIDER=cursor # or vscode, claude, codex, gemini, droid, opencode
```

This scaffolds a plugin that talks to your local deployment. Point the plugin at a different deployment by setting `EVERUNS_MCP_URL` (defaults to `https://app.everruns.com/mcp`).

## Codex (ChatGPT + CLI)

Use the plugin's `defaultPrompt` and `description` so you do not have to type OAuth scopes and MCP labels by hand:

```jsonc
{
  "title": "Everruns",
  "text": "...",
  "images": ["docs/getting-started/codex-everruns-plugin.png"],
  "skill": "everruns",
  "commands": ["commands"],
  "mcp": "everruns",
  "defaultPrompt": "You are using Everruns at ${EVERUNS_MCP_URL:-https://app.everruns.com/mcp}. Use the everruns skill...",
  "description": "Connects Codex to Everruns over MCP with skills, slash commands, and agents."
}
```

![Everruns Codex plugin](codex-everruns-plugin.png)

To install the published plugin, open the `everruns` plugin page in Codex and choose **Add to Codex**.

## Plugin layout

The portable plugin lives in `plugins/everruns/`:

- `plugin.json` / `mcp.json` — marketplace registration (name `everruns`, version, MCP server URL).
- `.claude-plugin/plugin.json` — Claude Code manifest.
- `.codex-plugin/plugin.json` — Codex manifest (`defaultPrompt`, `description`).
- `.cursor-plugin/plugin.json` — Cursor manifest.
- `skills/everruns/SKILL.md` — the agent skill (frontmatter `name: everruns`).
- `commands/` — slash commands.

`EVERUNS_MCP_URL` selects the deployment the plugin talks to; it defaults to `https://app.everruns.com/mcp`.
