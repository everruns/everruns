# Plugins

Plugins kept in this repository, one directory per plugin. First-party plugins
for coding agents (the `everruns` plugin for Claude Code, Codex, Cursor and
Gemini CLI) live in [everruns/plugins](https://github.com/everruns/plugins),
which is also the default marketplace every Everruns organization starts with.

## Layout

| Directory | Plugin | Hosts |
|---|---|---|
| `plugins/resend/` | `resend`: send email via Resend over MCP (`skills/resend/`) | Everruns (inbound fixture and example), Claude, Codex, Cursor |

## Rules

- The plugin `name` and `version` must match across the root `plugin.json` and
  all three host manifests (`.claude-plugin/`, `.codex-plugin/`, `.cursor-plugin/`).
- The skill directory (`skills/<name>/SKILL.md`) must declare matching
  frontmatter `name` and `description`.
- Do not add a root marketplace or a coding-agent plugin here; add it to
  everruns/plugins.

## Verification

Run `bash scripts/test-plugins.sh`. See `knowledge/integrations/plugins.md`
for the plugin contract.
