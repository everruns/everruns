# Plugins

Portable agent plugins live here, one directory per plugin. Each plugin keeps
per-host manifests (Claude `.claude-plugin/`, Codex `.codex-plugin/`, Cursor
`.cursor-plugin/`) in sync with shared marketplace registration (`plugin.json` /
`mcp.json`), one skill, and matching slash commands.

## Layout

| Directory | Plugin | Hosts |
|---|---|---|
| `plugins/everruns/` | `everruns` — connect to Everruns over MCP (`skills/everruns/`) | Claude, Codex, Cursor |
| `plugins/resend/` | `resend` — send email via Resend over MCP (`skills/resend/`) | Claude, Codex, Cursor |

## Rules

- The plugin `name` and `version` must match across the root `plugin.json` and
  all three host manifests (`.claude-plugin/`, `.codex-plugin/`, `.cursor-plugin/`).
- The MCP endpoint lives in `.mcp.json` (`https://app.everruns.com/mcp`), which
  all three hosts read; `mcp.json` pins the same production URL.
- The skill directory (`skills/<name>/SKILL.md`) must declare matching
  frontmatter `name` and `description`.
- Marketplace registration lives in `.claude-plugin/marketplace.json`,
  `.agents/plugins/marketplace.json`, and `.cursor-plugin/marketplace.json`;
  keep the plugin entries pointing at the directories above.

## Verification

Run `bash scripts/test-plugins.sh`. It checks manifest name/version parity,
marketplace registration, and skill frontmatter. See
`knowledge/integrations/plugins.md` for the plugin contract.
