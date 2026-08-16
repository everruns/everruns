---
type: Specification
title: "Everruns Plugins"
description: "Everruns(Dev) plugin sync contract."
tags:
  - everruns
  - integrations
---
# Everruns Plugins

## Abstract

Single-source plugins under `plugins/everruns-dev/` and `plugins/everruns/`
package the Everruns MCP server, matching skills, and shared slash commands for
Agent Plugins v1, Claude Code, Codex, and Cursor. `everruns-dev` targets
`https://dev.everruns.com`; `everruns` targets `https://app.everruns.com`.
The portable root `plugin.json` and three host manifests
(`.claude-plugin/plugin.json`, `.codex-plugin/plugin.json`, and
`.cursor-plugin/plugin.json`) MUST stay in sync on shared metadata and on the
shared payload they expose. Drift between any two hosts is a release-blocking
bug.

`scripts/test-everruns-dev-plugin.sh` is the authoritative gate. CI invokes it
through `just pre-push`. Any change to the plugin layout, manifests, or
marketplace registrations MUST keep that script green.

## Layout

```
plugins/everruns-dev/
├── plugin.json                   # portable Agent Plugins manifest
├── mcp.json                      # portable Streamable HTTP MCP config
├── .claude-plugin/plugin.json    # Claude Code manifest
├── .codex-plugin/plugin.json     # Codex manifest
├── .cursor-plugin/plugin.json    # Cursor manifest
├── .mcp.json                     # Shared MCP server config
├── README.md
├── assets/                       # Codex/Cursor UI metadata (icon, logo)
├── commands/                     # Shared slash commands
└── skills/everruns-dev/SKILL.md  # Shared skill
```

`plugins/everruns/` has the same structure, with `skills/everruns/SKILL.md`.

Marketplace registrations live outside the plugin directory:

- `.claude-plugin/marketplace.json`, Claude Code marketplace
- `.agents/plugins/marketplace.json`, Codex marketplace
- `.cursor-plugin/marketplace.json`, Cursor marketplace

## Sync Contract

The four manifests describe the same plugin to portable and host-specific clients. Some
fields are shared verbatim; some are host-specific. The Claude manifest is the
canonical source for shared metadata; Codex and Cursor mirror it.

### Fields That MUST Match Verbatim Across All Hosts

| Field        | Source of truth                                                  |
| ------------ | ---------------------------------------------------------------- |
| `name`       | `everruns-dev` for dev, `everruns` for production                |
| `version`    | Bumped together across the portable/host manifests + Claude/Cursor marketplaces |
| `homepage`   | `https://everruns.com`                                           |
| `repository` | `https://github.com/everruns/everruns`                           |
| `license`    | `MIT`                                                            |
| `keywords`   | Identical, identical order                                       |

### Author

Claude and Codex share an identical `author` object including
`{ "name": "Everruns", "url": "https://everruns.com" }`. Cursor's plugin
manifest schema rejects `author.url` (only `name` and `email` are accepted),
so Cursor's `author` is `{ "name": "Everruns", "email": "support@everruns.com" }`.
The validator enforces:

- Claude `author` == Codex `author` (verbatim object equality)
- Cursor `author.name` == Claude `author.name`

### Description

All three descriptions describe the same product. The Codex variant may add
`from Codex` and the Cursor variant may add `from Cursor` to disambiguate the
host. Aside from at-most-one such marker insertion, the wording MUST match.

- Claude:
  `Interact with the Everruns(Dev) managed harnesses platform. Manage
  harnesses, agents, and capabilities. Run agentic sessions. Create and deploy
  agentic applications.`
- Codex:
  `Interact with the Everruns(Dev) managed harnesses platform from Codex.
  Manage harnesses, agents, and capabilities. Run agentic sessions. Create and
  deploy agentic applications.`
- Cursor:
  `Interact with the Everruns(Dev) managed harnesses platform from Cursor.
  Manage harnesses, agents, and capabilities. Run agentic sessions. Create and
  deploy agentic applications.`

The production plugin uses the same wording with `Everruns` in place of
`Everruns(Dev)`.

### Component Pointers

All three manifests MUST declare the shared payload explicitly so each host
loads the same skill set and MCP config:

- `skills`: `"./skills/"`
- `mcpServers`: `"./.mcp.json"`

The Cursor manifest additionally declares `commands: "./commands/"` to make
the slash commands explicit (Cursor would otherwise discover them via the
default folder, but the explicit path keeps the manifest self-describing).

These paths point at files at the plugin root, not inside `.claude-plugin/`,
`.codex-plugin/`, or `.cursor-plugin/`. Each host rejects components living
inside the manifest folder.

The shared `.mcp.json` file (with leading dot) lives at the plugin root.
Cursor's default convention is `mcp.json` (no dot); we override discovery via
the explicit `mcpServers: "./.mcp.json"` path so all three hosts read the
same file. Cursor's official plugin validator emits a "no mcp.json file"
warning, which is informational, the manifest path resolves correctly.

Portable Agent Plugins clients instead discover fixed root `plugin.json` and
`mcp.json`. The portable MCP entry uses `type: "streamable-http"`; OAuth is
requested through `extensions.com.everruns` because Agent Plugins leaves
authentication to the client. See [plugins.md](plugins.md).

### Host-Specific Fields

| Field         | Host   | Notes                                                                                                                |
| ------------- | ------ | -------------------------------------------------------------------------------------------------------------------- |
| `interface`   | Codex  | UI metadata: `displayName`, `shortDescription`, `longDescription`, `category`, `capabilities`, icons, screenshots    |
| `displayName` | Cursor | Human-readable name; rendered in the Cursor marketplace and plugin chrome.                                           |
| `publisher`   | Cursor | Publishing org. Set to `Everruns`.                                                                                   |
| `logo`        | Cursor | Relative path to a logo image (`assets/everruns.png`) committed to the repo.                                         |
| `tags`        | Cursor | Free-form filter tags surfaced in the marketplace UI.                                                                |
| `category`    | Marketplace entry only | Claude Code's `plugin.json` schema does NOT accept `category`. Put it on the marketplace plugin entry instead (Cursor accepts it on either). |

Adding `interface` to the Claude manifest breaks loading (`Invalid manifest
file`). Codex tolerates the extra fields it does not understand, but
new host-specific keys SHOULD live on the matching host's manifest only.

Cursor's manifest schema is `additionalProperties: false`, so unknown fields
are rejected. Slash command files MUST declare both `name` and `description`
in YAML frontmatter for Cursor compatibility, this is a no-op for Claude
Code and Codex, which ignore the extra `name` field.

### Marketplace Registrations

- Claude Code: `.claude-plugin/marketplace.json`, top-level `description` is
  required for the plugin browser to render. The plugin entry's `version` MUST
  match `plugin.json`.
- Codex: `.agents/plugins/marketplace.json`, the top-level marketplace name is
  the neutral `everruns` source because it contains both dev and production
  plugins. Plugin entries use `source: { source: local, path:
  ./plugins/<plugin-name> }` and a `policy` block (`installation: AVAILABLE`,
  `authentication: ON_INSTALL`).
- Cursor: `.cursor-plugin/marketplace.json` at the repo root, uses
  `source: "./plugins/<plugin-name>"`. Both `metadata.version` and the per-plugin
  `version` MUST match `plugin.json`. The marketplace entry's `logo` is
  resolved from the repo root (`plugins/<plugin-name>/assets/everruns.png`),
  while the plugin manifest's `logo` is resolved relative to the plugin
  directory (`assets/everruns.png`).

All marketplaces MUST point at the matching plugin directory:
`./plugins/everruns-dev` for dev and `./plugins/everruns` for production.

### MCP Server Endpoint

`.mcp.json` MUST declare a single server whose name matches the plugin name.
`everruns-dev` points at `https://dev.everruns.com/mcp`; `everruns` points at
`https://app.everruns.com/mcp`. `oauth_resource` must be set to the same URL
(RFC 8707), and `scopes` must be omitted (PropelAuth rejects scopes on this
resource). See `knowledge/integrations/mcp.md` for the MCP server's auth contract.

### Skill Content

`skills/<plugin-name>/SKILL.md` is the shared skill body for each plugin. It
MUST NOT mention `switch_organization` (removed) and MUST contain the multi-org
guidance phrases enforced by the validator. MCP is stateless: callers route to
a specific organization by passing `organization_id` per call, not by switching
context. See `knowledge/integrations/mcp.md`.

## Sync Workflow

When changing the plugin:

1. Update the shared payload (`.mcp.json`, `commands/`, `skills/`, README) once
, all three hosts pick it up.
2. Update the portable and all three host `plugin.json` files together for any shared metadata change.
3. Bump `version` in all four `plugin.json` files, in
   `.claude-plugin/marketplace.json`, and in `.cursor-plugin/marketplace.json`
   (both `metadata.version` and the per-plugin `version`) together. Codex
   marketplace does not pin a version.
4. Run `bash scripts/test-everruns-dev-plugin.sh` and `just pre-push` before
   pushing.
5. Smoke test:
   - Claude Code: `/plugin install everruns-dev@everruns-dev`, then
     `/everruns-dev:whoami`.
   - Claude Code production plugin: `/plugin install everruns@everruns-dev`,
     then `/everruns:whoami`.
   - Codex: workspace marketplace install, then the same skill command.
   - Cursor: load the local marketplace via the plugins UI (or push and
     submit at <https://cursor.com/marketplace/publish>), install the plugin,
     and run the `whoami` slash command.

## Connector Install UX (Claude vs Codex vs Cursor)

Codex installs the plugin's MCP server during plugin install, gated on the
marketplace `authentication: ON_INSTALL` policy. The OAuth flow runs
inline at install time and the connector is wired up automatically.

Claude Code surfaces every plugin-declared MCP server as a **connector**
that the user MUST explicitly enable from the plugin's Connectors tab. The
plugin manifest cannot opt out of this prompt: Claude Code treats remote MCP
servers as user-granted capabilities, so OAuth and tool exposure require an
explicit click. This is a host-side product decision, not a plugin
configuration. The plugin's role is limited to declaring `mcpServers` in
`plugin.json` and shipping a valid `.mcp.json` with `oauth_resource`; the
rest is up to the host.

Cursor reads the `mcpServers` path from `plugin.json` and registers the
remote MCP server during plugin install. OAuth runs in the user's browser on
first use; there is no plugin-side switch to skip the consent flow. The
manifest shape is the same as Claude/Codex, the differences are purely in
how each host renders consent and lifetime of the granted credential.

If any host adds or changes its auto-install policy, this spec and the
validator should be updated to require the equivalent declaration so all
three hosts behave the same.

## Validation

`scripts/test-everruns-dev-plugin.sh` enforces:

- `.mcp.json` `url` and `oauth_resource` equal `https://dev.everruns.com/mcp`,
  or `https://app.everruns.com/mcp` for the production plugin, with no
  `scopes`.
- Portable `plugin.json` and `mcp.json` use the Agent Plugins v1 schemas,
  Streamable HTTP URL, and `com.everruns` OAuth extension.
- All three manifests declare the expected plugin name: `everruns-dev` or
  `everruns`.
- `version` is identical across the portable and all three host manifests, the Claude marketplace
  entry, and both `metadata.version` and the per-plugin `version` of the
  Cursor marketplace.
- Claude marketplace `source` points at the matching plugin directory; Codex
  marketplace `source` is local at the same path; Cursor marketplace `source`
  matches that path.
- Codex marketplace top-level `name` is `everruns` and `interface.displayName`
  is `Everruns`, so the shared source label does not misidentify the production
  plugin as `everruns-dev`.
- Claude `plugin.json` does NOT contain `category` or `interface`.
- All three manifests declare `skills: "./skills/"` and
  `mcpServers: "./.mcp.json"`.
- Shared metadata (`author`, `homepage`, `repository`, `license`, `keywords`)
  matches between Claude and Codex manifests verbatim.
- Cursor manifest's `homepage`, `repository`, `license`, and `keywords` match
  Claude verbatim. `author.name` matches Claude (the rest of `author` may
  differ because Cursor's schema rejects `author.url`).
- All three manifests declare a non-empty `description`. The Codex
  description equals the Claude description with at most one ` from Codex`
  insertion; the Cursor description equals the Claude description with at
  most one ` from Cursor` insertion.
- Cursor manifest declares a non-empty `displayName` and a `logo` whose path
  resolves to a file on disk (or an absolute URL).
- Cursor manifest's `name` matches the Cursor marketplace name pattern
  `^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$`.
- Every command file under `commands/*.md` declares `name: <filename-stem>`
  and a non-empty `description` in YAML frontmatter (required by Cursor;
  ignored by Claude/Codex).
- `marketplace.json` declares a top-level `description`.
- `SKILL.md` has no `switch_organization` references and contains the
  required multi-org phrases.

Adding new sync requirements means extending this script in the same PR.
