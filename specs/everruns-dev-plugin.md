# Everruns(Dev) Plugin

## Abstract

Single-source plugin under `plugins/everruns-dev/` packages the Everruns(Dev)
MCP server, the `everruns-dev` skill, and shared slash commands for both
Claude Code and Codex. The two host manifests
(`.claude-plugin/plugin.json`, `.codex-plugin/plugin.json`) MUST stay in sync
on shared metadata and on the shared payload they expose. Drift between the
two hosts is a release-blocking bug.

`scripts/test-everruns-dev-plugin.sh` is the authoritative gate. CI invokes it
through `just pre-push`. Any change to the plugin layout, manifests, or
marketplace registrations MUST keep that script green.

## Layout

```
plugins/everruns-dev/
├── .claude-plugin/plugin.json   # Claude Code manifest
├── .codex-plugin/plugin.json    # Codex manifest
├── .mcp.json                    # Shared MCP server config
├── README.md
├── assets/                      # Codex UI metadata (icon, logo)
├── commands/                    # Shared slash commands
└── skills/everruns-dev/SKILL.md # Shared skill
```

Marketplace registrations live outside the plugin directory:

- `.claude-plugin/marketplace.json` — Claude Code marketplace
- `.agents/plugins/marketplace.json` — Codex marketplace

## Sync Contract

The two manifests describe the same plugin to two different hosts. Some fields
are shared verbatim; some are host-specific.

### Fields That MUST Match Verbatim

| Field        | Source of truth                                                  |
| ------------ | ---------------------------------------------------------------- |
| `name`       | Always `everruns-dev`                                            |
| `version`    | Bumped together; `claude-marketplace` entry must match           |
| `author`     | Identical object in both manifests                               |
| `homepage`   | `https://everruns.com`                                           |
| `repository` | `https://github.com/everruns/everruns`                           |
| `license`    | `MIT`                                                            |
| `keywords`   | Identical, identical order                                       |

### Description

Both descriptions describe the same product, but the Codex variant is allowed
to add `from Codex` to disambiguate the host. Aside from the optional
`from Codex` insertion, the wording MUST match.

- Claude:
  `Interact with the Everruns(Dev) managed harnesses platform. Manage
  harnesses, agents, and capabilities. Run agentic sessions. Create and deploy
  agentic applications.`
- Codex:
  `Interact with the Everruns(Dev) managed harnesses platform from Codex.
  Manage harnesses, agents, and capabilities. Run agentic sessions. Create and
  deploy agentic applications.`

### Component Pointers

Both manifests MUST declare the shared payload explicitly so each host loads
the same skill set and MCP config:

- `skills`: `"./skills/"`
- `mcpServers`: `"./.mcp.json"`

These paths point at files at the plugin root, not inside `.claude-plugin/`.
Claude Code rejects components living inside `.claude-plugin/`.

### Host-Specific Fields

| Field       | Host   | Notes                                                                                                                |
| ----------- | ------ | -------------------------------------------------------------------------------------------------------------------- |
| `interface` | Codex  | UI metadata: `displayName`, `shortDescription`, `longDescription`, `category`, `capabilities`, icons, screenshots    |
| `category`  | Marketplace entry only | Claude Code's `plugin.json` schema does NOT accept `category`. Put it on the marketplace plugin entry instead. |

Adding `interface` to the Claude manifest breaks loading (`Invalid manifest
file`). Codex tolerates the extra fields it does not understand, but
new host-specific keys SHOULD live on the matching host's manifest only.

### Marketplace Registrations

- Claude Code: `.claude-plugin/marketplace.json` — top-level `description` is
  required for the plugin browser to render. The plugin entry's `version` MUST
  match `plugin.json`.
- Codex: `.agents/plugins/marketplace.json` — uses `source: { source: local,
  path: ./plugins/everruns-dev }` and a `policy` block
  (`installation: AVAILABLE`, `authentication: ON_INSTALL`).

Both marketplaces MUST point at `./plugins/everruns-dev`.

### MCP Server Endpoint

`.mcp.json` MUST declare a single server named `everruns-dev` pointing at
`https://dev.everruns.com/mcp`, with `oauth_resource` set to the same URL
(RFC 8707) and no `scopes` (PropelAuth rejects scopes on this resource). See
`specs/mcp.md` for the MCP server's auth contract.

### Skill Content

`skills/everruns-dev/SKILL.md` is the shared skill body. It MUST NOT mention
`switch_organization` (removed) and MUST contain the multi-org guidance
phrases enforced by the validator. MCP is stateless: callers route to a
specific organization by passing `organization_id` per call, not by switching
context. See `specs/mcp.md`.

## Sync Workflow

When changing the plugin:

1. Update the shared payload (`.mcp.json`, `commands/`, `skills/`, README) once
   — both hosts pick it up.
2. Update both `plugin.json` files together for any shared metadata change.
3. Bump `version` in both `plugin.json` files and in
   `.claude-plugin/marketplace.json` together. Codex marketplace does not
   pin a version; Claude marketplace does.
4. Run `bash scripts/test-everruns-dev-plugin.sh` and `just pre-push` before
   pushing.
5. Smoke test:
   - Claude Code: `/plugin install everruns-dev@everruns-dev`, then
     `/everruns-dev:whoami`.
   - Codex: workspace marketplace install, then the same skill command.

## Connector Install UX (Claude vs Codex)

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

If Claude Code adds an auto-install policy in the future, this spec and the
validator should be updated to require the equivalent declaration so both
hosts behave the same.

## Validation

`scripts/test-everruns-dev-plugin.sh` enforces:

- `.mcp.json` `url` and `oauth_resource` equal `https://dev.everruns.com/mcp`,
  no `scopes`.
- Both manifests declare `name: everruns-dev`.
- `version` is identical across both manifests and the Claude marketplace
  entry.
- Claude marketplace `source` is `./plugins/everruns-dev`; Codex marketplace
  `source` is local at the same path.
- Claude `plugin.json` does NOT contain `category`.
- Claude `plugin.json` does NOT contain `interface` (Codex-only field;
  including it breaks Claude Code plugin loading).
- Both manifests declare `skills: "./skills/"` and
  `mcpServers: "./.mcp.json"`.
- Shared metadata (`author`, `homepage`, `repository`, `license`, `keywords`)
  matches between the two manifests.
- Both manifests declare a non-empty `description`. The Codex description
  equals the Claude description, optionally with the `from Codex` host
  marker inserted exactly once; any other deviation fails the check.
- `marketplace.json` declares a top-level `description`.
- `SKILL.md` has no `switch_organization` references and contains the
  required multi-org phrases.

Adding new sync requirements means extending this script in the same PR.
