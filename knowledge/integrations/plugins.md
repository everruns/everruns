---
type: Specification
title: "Plugins"
description: "Plugin host: marketplaces and cross-host plugin packages installed as capabilities."
tags:
  - everruns
  - integrations
---
# Plugins

## Abstract

A Plugin is an installable package. Everruns accepts the portable
[Agent Plugins v1.0.0](https://agent-plugins.org/specification) format and the
legacy cross-host directory formats used by Claude Code, Codex, and Cursor.
Installing either dialect into an organization produces a capability with
reference `plugin:{install_public_id}` that agents and harnesses enable like any other
capability.

The control plane owns hydration of installed definitions. Durable execution
transports the complete hydrated capability configs for agents and harnesses,
not only their references, so the worker evaluates the same definition that
passed install-time compilation. Older workers remain readable through the
reference-only compatibility path, but data-backed contributions require the
full config contract.

Portable packages use root `plugin.json`, fixed `skills/`, and optional root
`mcp.json`. Legacy packages continue to use their host-specific hidden manifest,
component path overrides, `commands/`, `agents/`, and `.mcp.json`. A canonical
root manifest takes precedence when both forms are present; Everruns-only
behavior belongs under the `com.everruns` extension namespace.

A Marketplace is an org-registered catalog in the `marketplace.json` format.
A first-party default marketplace ships preconfigured for every organization,
but it is a regular marketplace row, nothing about it is special-cased in
code.

Direction matters relative to existing specs:

- [everruns-dev-plugin.md](everruns-dev-plugin.md) is **outbound**: Everruns
  packaged as a plugin for Claude Code/Codex/Cursor (`plugins/everruns*`).
- This spec is **inbound**: Everruns consuming plugins in the same format.
  The outbound plugins double as the first dogfood content: `everruns` and
  `everruns-dev` must install cleanly into Everruns itself.

## Why not just declarative capabilities

Persisted declarative capabilities ([capabilities.md](../execution/capabilities.md)) are
the right *runtime* model: data-only contributions (system prompt, skills,
text file mounts, scoped MCP config) with no arbitrary server-side code. A
plugin install **compiles into the same declarative definition shape** and
reuses its hydration, validation, size limits, and worker execution path
unchanged.

What plugins add is the layer declarative capabilities deliberately lack:

- **Interchange format**: consume packages authored for Claude Code, Codex,
  or Cursor without conversion, at least partially. Unsupported components
  degrade to install warnings, mirroring Claude Code's own tolerant loading
  policy for unrecognized fields.
- **Distribution**: marketplaces, sources (git repo, subdirectory, URL),
  versioning, and pinning.
- **Provenance and lifecycle**: where a capability came from, which version
  is installed, update/uninstall semantics. A declarative capability is
  org-local and hand-edited; an installed plugin is a managed artifact whose
  compiled definition is regenerated on update, never edited in place.

## Component mapping

One plugin installs as one capability. Manifest fields and components map to
capability contributions:

| Plugin component                  | Capability contribution                                       |
| --------------------------------- | ------------------------------------------------------------- |
| `name`                            | capability display/discovery name                             |
| `icon`                            | bundled relative SVG, validated and embedded in capability metadata |
| `displayName`                     | `display_name`                                                |
| `description`                     | `description`                                                 |
| `version`, `author`, `homepage`, `repository`, `license`, `keywords` | install provenance metadata (not runtime) |
| `agents/*.md`                     | system prompt contribution: each agent file rendered as a named persona/instructions section |
| `skills/<name>/SKILL.md` + files  | skill packages (same shape as `DeclarativeCapabilitySkill`)   |
| `commands/*.md`                   | user-invocable skills (`user_invocable: true`); frontmatter `name`/`description` carried over |
| `mcp.json` or legacy `.mcp.json` / `mcpServers` | scoped MCP servers; portable Streamable HTTP and legacy HTTP entries are SSRF-validated like all scoped MCP config |
| `userConfig`                      | *(phase 2)* capability `config_schema`                        |
| `hooks`, `lspServers`, `monitors`, `themes`, `outputStyles` | ignored; surfaced as install warnings |

Notes:

- `agents/*.md` are subagent definitions on other hosts. Everruns has no
  per-plugin subagent runtime, so v1 folds them into the capability system
  prompt (the user-visible contract: "this plugin teaches the agent these
  personas/behaviors"). Mapping them to agent blueprints or subagent
  definitions is a possible later phase; the install pipeline keeps the raw
  files so re-compilation can change the mapping.
- Hook components are shell commands on the authoring host and cannot run
  server-side. If a future phase maps them to `user_hooks`
  ([user-hooks.md](../runtime-resources/user-hooks.md)), the compiled capability must take
  `risk_level: high` and the admin assignment gate, same as the planned
  declarative `user_hooks` field.
- Portable names follow the Agent Plugins schema; legacy names retain the host
  convention. Names are unique per organization across installed plugins. The
  server capability identity is the install public ID; standalone runtimes
  without install rows use the manifest name in the `plugin:` namespace, whose
  persisted columns accommodate the portable 64-character maximum plus the
  prefix.
- Icon paths must stay inside the package and resolve to a bounded UTF-8 SVG.
  Active content, external references, data URLs, and remote URLs are rejected
  with an install warning and use the neutral plugin fallback instead.

## OAuth-authenticated MCP servers

A legacy plugin's `.mcp.json` server may declare `"auth": "oauth"` (alias
`"auth_mode": "oauth"`) to require a user-scoped OAuth connection, the pattern
used by remote MCP servers like Resend (`https://mcp.resend.com/mcp`). The
compiler maps this to `auth_mode = oauth` on the compiled scoped server. Two
fields plugin content can **not** set are enforced at compile time (dropped
with a warning): `oauth_provider_id` (the host assigns it) and any `api_key`
(a package cannot carry key material). Only `"none"` and `"oauth"` are accepted
auth modes; anything else warns and degrades to `none`. Literal `headers` are
preserved and only ever sent to the plugin's own server URL.

**Install-time anchoring.** An OAuth MCP server needs a stable per-org provider
id and somewhere to persist discovered authorization-server metadata plus the
dynamically registered OAuth client. Rather than a parallel store, install
creates one linked org `mcp_servers` row per OAuth server, the *anchor*, held
in `status = disabled` so it never becomes a runtime capability and never
resolves by tool prefix. The compiled definition's `oauth_provider_id` is set
to the anchor's `mcp_oauth_{uuid}`. This reuses the existing MCP-OAuth
machinery unchanged:

- The anchor surfaces in `GET /v1/user/connections/providers` as an OAuth
  provider, so the standard authorize/callback flow
  (`/v1/user/connections/{provider}/authorize`), dynamic client registration,
  and encrypted token storage all work against it.
- At execution, the worker resolves the session's connection token for the
  server's `oauth_provider_id` and injects it as a `Bearer` header. When no
  token is connected, the MCP executor returns a `connection_required` tool
  result, rendering the inline "connect" prompt instead of a raw 401.

Connection providers use the plugin's display name (and the server name when a
plugin contributes more than one OAuth server), so package identifiers do not
leak into the user-facing Connections list. Connection resolution and live
tool discovery run again for each reasoning turn. A connection completed while
a session is open is therefore available on that session's next turn; starting
a new session is not required.

Anchors are reused across plugin **updates** while the server URL is unchanged.
A changed URL replaces the anchor and rotates its provider id, requiring users
to reconnect rather than risking refresh of an old grant against a new OAuth
authority. Anchors are removed when a server is dropped from the plugin and
deleted on **uninstall**.
Because the provider id is always assigned server-side from a host-created row,
plugin content can never bind to another provider (e.g. `github`) and read
tokens connected for it (`TM-PLUGIN-004`).

**Token refresh.** The connection resolver refreshes short-lived OAuth access
tokens lazily when they are expired or within 60 seconds of expiry. Refreshes
use the anchor's registered OAuth client through the host egress boundary;
concurrent resolution of one grant is coalesced, and rotated access/refresh
tokens are persisted atomically for both session-scoped and user-scoped
connections. A rejected refresh fails closed to the standard
`connection_required` reconnect flow. This applies equally to plugin anchors
such as Resend (whose access tokens last about 15 minutes) and org-managed
OAuth MCP servers.

Agent Plugins deliberately leaves authentication to the client. Portable
packages request the same behavior through
`extensions.com.everruns.mcpServers.<server>.auth`; `oauth` and `none` are the
supported values. Authentication data never enters portable `mcp.json`.

## Marketplaces

Org-scoped registry of plugin catalogs (`plugin_marketplaces`):

- `name`: unique per org, used in install references (`my-tool@my-marketplace`).
- `source`: where `marketplace.json` lives, GitHub repo, git URL, or direct
  HTTPS URL. Fetched through the egress boundary ([egress.md](../operations/egress.md)).
- Cached catalog: the validated `marketplace.json` content plus sync
  metadata (`last_synced_at`, resolved commit SHA when the source is git).

Sync is explicit (`POST .../sync`) and on a periodic schedule. The catalog
schema is the marketplace.json schema: top-level `name`/`owner`/`plugins`,
plugin entries with `name`, `source` (relative path, `github`, `url`,
`git-subdir`; `npm` deferred), and optional metadata/component overrides.
Unknown fields are preserved and ignored.

The **default marketplace** is seeded for every organization. It is named
`everruns`, uses `source_type: github`, and points at `everruns/everruns`
(the repo's own `.claude-plugin/marketplace.json`). Seeding happens at org
creation via `org_init::seed_default_plugin_marketplace`; existing orgs
received it via the one-time backfill in `058_backfill_default_marketplace.sql`.
The marketplace is deletable/disableable like any other marketplace; "default"
means seeded, not privileged. Deletion is permanent, the marketplace is
never re-seeded lazily on read.

## Lifecycle

- **Install**: resolve the plugin entry's source relative to the marketplace,
  fetch the plugin directory at a concrete commit SHA (GitHub tarball / git
  archive, no server-side clones in v1), validate against the manifest
  schema and declarative size/count limits, compile to the declarative
  definition shape, persist with provenance (`marketplace`, `source`,
  `version`, `pinned_sha`, raw manifest, install warnings).
- **Update**: explicit re-install at the marketplace's current entry;
  recompiles the definition. Version semantics follow the host convention:
  a manifest `version` pins until bumped, otherwise the commit SHA is the
  effective version. The installation public ID is preserved, so assignments
  retain the same identity. Auto-update is deferred.
- **Uninstall**: removes the capability; agents referencing its
  `plugin:{install_public_id}` surface an actionable dangling-ref error. A
  reinstall gets a new public ID and cannot silently rebind those agents.
- **Enable/disable**: installed plugins follow the standard building-block
  lifecycle and are assigned to agents/harnesses by capability ref.

## API sketch

```
GET/POST          /v1/plugin_marketplaces
GET/PATCH/DELETE  /v1/plugin_marketplaces/{id}
POST              /v1/plugin_marketplaces/{id}/sync
GET               /v1/plugin_marketplaces/{id}/plugins     # catalog
GET/POST          /v1/plugins                              # installed; install
GET/PATCH/DELETE  /v1/plugins/{id}
POST              /v1/plugins/{id}/update
```

Resources follow the dual-ID pattern ([id-schema.md](../foundations/id-schema.md)).
Installed plugins appear in `GET /v1/capabilities` as a distinct kind next to
built-in, MCP, skill, and declarative refs.

## UI

Marketplaces and plugins are managed through the UI on top of the same CRUD
API, full management parity with the API, not a read-only view:

- **Marketplaces**: list registered marketplaces with sync status; add by
  GitHub repo or URL (admin-gated); manual re-sync; remove/disable.
- **Catalog**: browse a synced marketplace's plugin entries (name, display
  name, description, version, author) and install from there.
- **Installed plugins**: list with provenance (marketplace, version, pinned
  SHA) and install warnings; enable/disable; uninstall; **Update** action
  shown when the synced catalog resolves to a newer version or SHA than the
  installed pin. Update is always explicit user action in v1.
- Installed plugins also surface in the existing capability picker as
  `plugin:{install_public_id}` entries, so assignment to agents/harnesses reuses the
  capability UI unchanged.

## Runtime mode

The plugins subsystem must work in the in-process runtime
([runtime.md](../foundations/runtime.md)), not only in the server/control-plane deployment:

- The **plugin compiler** (directory → dialect-specific manifest validation → declarative
  definition shape) lives in `everruns-core`, not in the server crate, so
  server, worker, and embedded runtime share one implementation.
- `InProcessRuntimeBuilder` accepts local plugins: load a plugin directory
  from disk (the equivalent of a local marketplace path source) or a
  pre-compiled definition. No PostgreSQL and no marketplace sync are
  involved; marketplace registration, catalog cache, and remote fetch are
  control-plane concerns only.
- Because workers and the runtime receive fully hydrated capability config
  (same property declarative capabilities rely on), a plugin compiled by the
  control plane executes identically in dev mode, durable worker mode, and
  embedded runtime.
- Plugin-declared MCP servers execute through the runtime MCP client
  ([runtime-mcp.md](runtime-mcp.md)). HTTP transport everywhere in v1;
  stdio-transport plugin servers are a possible embedded-runtime-only
  extension, rejected elsewhere.

## Test fixture

`testdata/plugins/` is a local marketplace fixture used by server and runtime
tests:

- `testdata/plugins/.claude-plugin/marketplace.json`, valid marketplace
  manifest with relative-path plugin sources.
- `testdata/plugins/microsoft-docs/`, an Everruns-authored variant of the
  public Microsoft Docs plugin (`MicrosoftDocs/mcp`), pointing at the same
  public MCP server (`https://learn.microsoft.com/api/mcp`). It exercises
  every v1 mapping: manifest metadata, `skills/`, `commands/`, `agents/`,
  and `.mcp.json`, plus an `interface` block that v1 ignores with a warning.
- `testdata/plugins/oauth-mail/`, minimal fixture whose `.mcp.json` sets
  `"auth": "oauth"`. It exercises the OAuth-anchor install path: install
  creates a disabled anchor row, assigns a host-owned `mcp_oauth_*` provider,
  and lists it in the connections API; uninstall removes it. The URL is a
  non-routable `.test` host and is never contacted.

Tests cover: marketplace sync from a local path, install/compile of the
fixture, the compiled capability's prompt/skill/MCP contributions, and
loading the same plugin directory through `InProcessRuntimeBuilder`. The MCP
server is public and unauthenticated, so manual smoke tests can call it live;
automated tests must not depend on network.

## Security

Plugins are third-party remote content compiled into agent context, a
supply-chain and prompt-injection surface on top of the declarative
capability threat model:

- Adding a marketplace and installing plugins are admin-gated org actions.
- Installs pin a commit SHA; an upstream force-push cannot silently change
  an installed plugin. Updates are explicit and re-validated.
- The compiled definition passes the full declarative validation: size/count
  limits, text-only files, path traversal rejection, skill name validation,
  SSRF-safe scoped-MCP URL validation.
- No code-execution components are compiled in v1 (hooks/LSP/monitors are
  dropped with warnings).
- MCP servers declared by a plugin use existing scoped-MCP auth (OAuth /
  API key) with per-org consent, analogous to Claude Code's connector
  enablement step.
- Install/update/uninstall and marketplace registration stay admin-gated;
  the member-level escape hatch is deliberate, a member can manually copy
  plugin content into a declarative capability or an agent prompt, which is
  self-limiting because there is no automated remote fetch and no upstream
  update channel.
- Threat model entries: see `TM-PLUGIN-*` in
  [threat-model.md](../security/threat-model.md) § 26.

## Phasing

1. **Shipped**: marketplaces CRUD + sync (GitHub + direct URL sources), install
   with relative-path and `github` plugin sources, compile
   skills/commands/agents/MCP, stable installation refs, capability registry and
   picker integration, marketplace/plugin management UI, core-owned compiler
   with `InProcessRuntimeBuilder` local-directory loading, dogfood by
   installing `everruns`/`everruns-dev` and the `microsoft-docs` fixture.
2. **Shipped**: Agent Plugins v1 portable manifest, skills, Streamable HTTP
   MCP, version matching, narrow failure isolation, and `com.everruns` auth
   extension, alongside the legacy host dialects.
3. **Next**: `userConfig` → capability `config_schema`, `git-subdir` and git
   URL sources, update UX with version diffing, install warnings surfaced in
   UI.
4. **Later**: publishing (export a declarative capability as a plugin
   directory), org-to-org sharing, `agents/*.md` → blueprints/subagents,
   hooks → `user_hooks` (high-risk gated), npm source.
