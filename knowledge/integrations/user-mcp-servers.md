---
type: Decision
title: "User MCP servers and agent MCP auth modes"
description: "Virtual users own MCP servers; agents opt in to use or manage them; agent-level servers choose service, user, or user-with-service-fallback auth; connect from chat; one mechanism shared with yolop."
tags:
  - everruns
  - integrations
  - mcp
  - identity
---

# User MCP servers and agent MCP auth modes

> Status: **Accepted 2026-10-06, being implemented**: steps 1 to 7 are built (steps in [Plan](#plan)).
> Public docs: `docs/features/user-mcp-servers.md`, `docs/capabilities/user-mcp-servers.md`.
> [Agent MCP attachments](agent-mcp-attachments.md),
> [MCP servers](mcp-servers.md), and [virtual users](../runtime-resources/virtual-users.md)
> stay authoritative for what exists today. This proposal extends them; where it
> changes an existing behavior it says so under [What changes](#what-changes).

## Summary

Everruns already has two of the three places an MCP server can live: the
**organization catalog** (presets an admin registers) and the **agent** (servers
attached to an agent, each saying who it acts as). The missing third is the
**person**: a virtual user should own a list of MCP servers the way they already
own connections, and an agent should be able to use that list, and optionally
change it, while chatting with them.

```
Organization catalog   presets: URL, OAuth client, era policy. Not used by anything until attached.
Agent servers          attached to one agent; acts as none / service / user / user-or-service.
User servers       owned by one virtual user; always acts as that user. NEW.
Session servers        added during one conversation; gone when it ends (exists today via ARD).
```

![How agents, capabilities, MCP servers, virtual users and connections relate](../../docs/features/user-mcp-servers.svg)

The three use cases map onto this directly:

| Use case | Configuration |
|---|---|
| Platform Chat respects my MCP servers and lets me add and connect them in chat | Platform Chat gets the **User MCP servers** capability with *use* and *manage* on. |
| Git Review Robot uses GitHub as itself | Agent server `github`, acts as **service**, authorized once on the agent (OAuth or the agent's GitHub App connection). |
| GitHub Personal Review Agent uses GitHub as whoever is talking to it | Agent server `github`, acts as **user**; the first call without a grant shows a Connect card in chat. |

## What exists today

Verified against `origin/main` at `0e0150f` (catalog row updated with step 8).

| Piece | State | Where |
|---|---|---|
| Org catalog with OAuth client registration per preset | Implemented | `mcp_servers` table, `crates/server/src/domains/mcp_servers/` |
| `actsAs: none / service / user` on every attachment, fail-closed, no fallback | Implemented | `McpServerActsAs` in `crates/contracts/src/runtime/mcp_server.rs`, `crates/server/src/storage/connection_resolver/mod.rs` |
| `use: catalog:<name>` references | Implemented | same contract file |
| Agent MCP side sheet (source, acts as, state, Connect/Authorize/Ask admin) | Implemented | `apps/ui/src/components/agents/agent-mcp-panel.tsx`, `/v1/agents/{id}/mcp-attachments` |
| Org MCP catalog in **Settings > Organization > MCP catalog** (step 8; was a main-navigation MCP page with **Catalog** and **My connections** tabs) | Implemented | `apps/ui/src/app/(main)/settings/mcp-catalog/page.tsx` |
| Virtual-user connections, including MCP OAuth grants keyed `mcp_oauth_<server uuid>` | Implemented | `crates/server/src/api/user_connections/mod.rs`, `virtual_user_connections.rs` |
| OAuth from chat: a missing grant becomes an inline Connect card (`setup_connection` hint) | Implemented | [client hints](../runtime-resources/client-hints.md) |
| URL and form elicitation from servers | Implemented | [mcp-servers.md](mcp-servers.md), [form elicitation](mcp-form-elicitation.md) |
| Adding an MCP server mid-session | Implemented for ARD only (`attach_resource` → session resource → session `mcpServers` on the next turn) | `crates/ard`, test case `ard_discovery/TC001` |
| Deferred tool schemas | Implemented per tool (`tool_search`), not per server | [tool search](../execution/tool-search.md) |
| MCP server owned by a person | **Missing** | |
| Agent setting to use or manage someone's user servers | **Missing** | |
| "Use the user's login if they have one, else the agent's" | **Missing**, and today's rule forbids it (no fallback) | |

yolop (on everruns `=0.41.0`) uses the everruns-core MCP
client, OAuth protocol, `ScopedMcpServer` and `merge_scoped_mcp_servers`
unchanged. It owns its own config store (`src/config/mcp.rs`: global
`settings.toml`, profile, workspace `.mcp.json`), a `/mcp add|remove|enable|
disable|login|reload` command, token storage in `connections.toml`, and a
loopback OAuth callback. Its model-facing MCP capability is read-only; writes go
through `/mcp` in the terminal.

## Decisions

### D1. A user MCP server is an MCP server record owned by a virtual user

Reuse the `mcp_servers` table and service with a nullable owner:
`owner_virtual_user_id IS NULL` is a catalog preset, otherwise it is that
person's server. Name uniqueness becomes per (org, owner).

Why one table rather than a new one: the OAuth client registration, discovery,
URL validation (SSRF), protocol era, elicitation policy and tool cache already
key off this row. A user server needs every one of them. A second table
would duplicate that code; an owner column reuses it.

A user server is either:

- **from the catalog**: the row points at its preset
  (`catalog_mcp_server_id`) and copies nothing that matters; the catalog's
  OAuth client is reused, the grant is the person's own (same as an agent
  attachment acting as `user` today), or
- **custom**: name, URL, optional literal headers. If it speaks OAuth, its
  dynamically registered client is stored on the user-owned row, exactly as a
  preset stores it.

Its grant is an ordinary virtual-user connection with provider
`mcp_oauth_<server uuid>` (the preset's uuid for a catalog server), so it shows up, refreshes and revokes like every other
connection. User servers always act as their owner. There is no way to
make them act as an agent, and nobody else's session can resolve them.

Concrete implication: a person's servers follow the virtual user, which is
org-scoped. The same person in two orgs has two lists, matching how their
connections already work.

### D2. One capability, two switches: *use* and *manage*

Add a built-in capability **User MCP servers** (`user_mcp`) with two
settings:

| Setting | Effect |
|---|---|
| `use` (default on) | The current user's enabled user servers join the agent's MCP servers for that turn. |
| `manage` (default off) | The agent gets tools to list, add, remove, enable/disable and connect the current user's servers. |
| `allow_custom_urls` (default off) | With `manage`, the agent may add servers that are not in the catalog. |

Why a capability and not a new agent field: capabilities already contribute MCP
servers with a declared `actsAs` (`collect_capability_mcp_servers`), already
appear in the agent editor with settings, and already lose to explicit agent
servers by name. Nothing new is needed in agent history, export or the
blueprint format.

Which user: the turn's verified **initiating virtual user**, never the session
owner or the agent's service account. Consequences:

- **Unattended runs get nothing.** A trigger or schedule has no initiating
  person, so `use` contributes no servers. This matches the existing rule that
  `user` attachments do not run unattended.
- **Shared conversations** (Slack channels, multi-participant sessions) are off
  by default: tools would change with every speaker, and one person's servers
  would be visible to the others' turns. A capability setting
  `in_shared_sessions` can turn it on later if there is a real need.
- **Name clashes**: an agent server or a capability server with the same name
  wins; the user one is skipped and reported in the agent MCP sheet and the
  manage tool's list output.

### D3. Management tools act on the person's own resources, not on the org

The `manage` tools use the same virtual-user self-service operations the
Settings page uses (`/v1/virtual-users/me/mcp-servers`, implemented in
`crates/server/src/domains/mcp_servers/user_servers/mod.rs`). They do not need, and never receive,
Everruns-user management authority. This keeps them working after Platform
Chat stops having org write access, and makes them available to external
consumers using a published agent.

Guardrails, because adding an MCP server is how a prompt injection would
exfiltrate data:

- `add` and `enable` require tool approval by default (the user confirms the
  server name and host in chat). `remove`, `disable` and `list` do not.
- Custom URLs are refused unless `allow_custom_urls` is on, and always pass the
  same URL validation as catalog presets.
- `connect` never handles a credential. It starts the existing in-chat Connect
  card; the user signs in in their own browser.
- A server added in a turn becomes usable on the next turn, the same as ARD.

### D4. Agent servers get a fourth auth mode: use the user's login if they have one

`actsAs` becomes:

| Value | Meaning | Missing login |
|---|---|---|
| `none` | No credential; literal headers only. | n/a |
| `service` | The agent's own account, authorized once by an admin. | Admin authorizes on the agent. |
| `user` | The person talking to the agent; required. | Connect card in chat. |
| `user_or_service` **(new)** | The person's own login if they connected one, otherwise the agent's. | Falls back to the agent's; if that is missing too, the admin authorizes. |

This is the "allow user-level auth" case. It deliberately reverses the current
"no fallback" rule for this one value, so it has to be loud:

- It is only ever chosen explicitly; nothing migrates to it.
- Every tool call records which account it used (`user` or `service`) in the
  event, and the agent MCP sheet labels the row "Acts as: the user, or the
  agent when the user has not connected".
- Unattended runs always use the service login.
- The agent can offer the person to connect their own account (D6), which
  switches the next call to their login.

Service credentials can come from three places, all existing: an OAuth grant on
the agent's service virtual user, the preset's org API key, or **an existing
connection on the service virtual user** named by the preset (for GitHub, the
agent's GitHub App installation from [per-agent GitHub Apps](github-apps.md)).
Only capability-contributed servers can name a connection today
(`oauth_provider_id`); this proposal lets a catalog preset name one too, so
the Git Review Robot does not need a second GitHub login just for MCP.

### D5. Connecting from chat is per server, with an agent-wide default

The in-chat Connect card already exists for missing grants. Two additions:

- **Per-attachment `connectInChat`** (`ask` default, `never`). `never` returns the
  error with a link to settings instead of pausing the turn: right for agents
  behind channels that cannot render a card.
- **A `connect_mcp_server` tool** (part of `user_mcp` with `manage`, and also
  available to agents with any `user` or `user_or_service` server) lets the agent
  offer the connection *before* a tool call fails, for example "connect GitHub
  so I can review your PRs".

Service logins can be authorized from chat only by someone holding MCP
management permission, and the card says it is authorizing the agent, not the
person. Otherwise the card says "ask an admin". This is the existing
`Authorize` / `AskAdmin` split from the agent MCP sheet.

### D6. Load MCP servers lazily and allow changes mid-session

User servers multiply the number of servers a turn might touch, and each
`tools/list` is a network round trip at turn start.

- **Per-server deferral.** A server can be marked `deferred` (default for
  user servers, which the owner can turn off per server; opt-in for agent
  servers). Its tools are not listed at turn
  start. The model sees one line per deferred server (name and description) and
  reveals a server's tools through `tool_search`, which then fetches
  `tools/list` once and uses the existing identity-scoped cache.
- **Session servers** generalize ARD's session resource: `attach_resource`,
  `user_mcp` *add* and a future "add for this chat only" all write the same
  session record, folded into session `mcpServers` on the next turn. Removing
  it drops the tools on the next turn.
- **Prompt cache.** Changing the tool set mid-session currently rewrites the
  tools array and misses the provider cache. The planned cache-stable tool
  reveals (inline tool additions) fix this for reveals and list MCP attach as
  a later step. This proposal depends on that work for cost, not for
  correctness, and does not duplicate it.

### D7. yolop uses the same code

yolop already has user MCP servers: its global `settings.toml` list is a
single user's list. The proposal makes the two hosts share the management layer,
not just the protocol, without new crates:

| Piece | Lives in | Server implementation | yolop implementation |
|---|---|---|---|
| Server entry shape `{ enabled, ScopedMcpServer }` | `everruns-core` (`mcp` module) | DB row | `settings.toml` (already `McpServerEntry`) |
| `UserMcpStore` trait: list, upsert, remove, set_enabled | `everruns-core` | virtual-user rows | `McpConfigStore` (already has these four methods) |
| `McpTokenStore` | `everruns-core` (exists) | virtual-user connections | `connections.toml` (exists) |
| `McpLoginPrompter` trait: start a login, report when done | `everruns-core` | in-chat Connect card | loopback callback + browser (exists) |
| `user_mcp` capability (use/manage tools, approval policy) | `everruns-capabilities` | registered | registered; replaces yolop's read-only MCP capability, so the model can add and log in without `/mcp` |
| Deferred servers | `tool_search` in `everruns-core` | yes | yes |

`yolop mcp …` and `/mcp …` stay as the human commands and call the same store.
Workspace `.mcp.json` remains yolop-only (there is no workspace in the server).

## UI

- **Main navigation "MCP" moves to Settings > Organization > MCP catalog.** It is
  an admin registry: presets with "used by N agents", and a line at the top
  saying a preset does nothing until an agent or a person adds it. People who
  cannot manage the catalog no longer see it in the main navigation.
- **Settings > My agent experience** gets a **My MCP servers** section next to
  Connections: one row per server (name, host, from catalog / custom, connected
  as, enabled), Add (search the catalog, or custom URL), Connect, Remove. The
  old *My connections* tab on the MCP page redirects here; MCP grants for
  agent servers acting as you are listed here as well, so "what have I
  authorized" has one answer.
- **Agent > MCP servers sheet** shows a group "User servers of the person
  chatting" when `user_mcp` is on, with the switches, and the new
  `user_or_service` label. Adding a server keeps the single "Who does this act
  as?" question with a third option, "Each user, or the agent if they have not
  connected".
- **Chat**: Connect cards unchanged; approval cards for `add`/`enable`.
- **Chat header**: an **MCP** button, shown only when the conversation has
  servers added for it alone, lists them (name, host, signed in or not) with
  Remove.

## What changes

Nothing is removed. These existing behaviors change:

| Existing feature | Change |
|---|---|
| MCP page in main navigation | Moves to Settings > Organization. Its *My connections* tab moves to Settings > My agent experience (old URLs redirect). |
| "No fallback between user and service logins" ([agent MCP attachments](agent-mcp-attachments.md) D2) | Still true for `user` and `service`. The new `user_or_service` value falls back by design, visibly. |
| "OAuth needs a catalog preset" (same doc, D4) | Relaxed for user custom servers, whose own row stores the OAuth client. Inline agent and session servers stay credential-free. |
| `mcp_servers` table and API | Gain an owner. Org list endpoints exclude user-owned rows; existing rows are catalog rows with no migration of data. |
| Platform Chat | Gets `user_mcp` (use + manage). Its org-level MCP management through Platform commands is unaffected by this proposal; the read-only Platform Chat work decides that separately. |
| ARD `attach_resource` | Unchanged behavior; an MCP target also writes the general session-server record, which is what the turn folds. |
| URL/form elicitation (EVE-1068), MCP Events triggers, agent tool-parameter credentials, connectors | Unchanged. User servers use the default `url` elicitation policy. MCP Events triggers keep requiring service servers (user servers never fire triggers). |
| yolop's read-only MCP capability | Replaced by `user_mcp`, which can write. `/mcp` commands unchanged. |

## Plan

Each step is one PR, shippable alone.

1. **User servers backend.** Owner column on `mcp_servers`, `/v1/virtual-users/{id|me}/mcp-servers` self-service routes (the same authority as preferences; the command catalog is management-only), OAuth authorize/callback for a user-owned row, tests for cross-user and cross-org refusal.
2. **My MCP servers in settings.** Section in My agent experience; MCP page *My connections* tab redirects there.
3. **`user_mcp` capability, *use*.** Resolve the initiating virtual user's enabled servers into the turn; unattended and shared sessions get none; Platform Chat turns *use* on. Built: `crates/server/src/domains/mcp_servers/user_layer/mod.rs`, which the worker turn context, MCP prefix resolution and the tool-call token check all share; channel consumers may sign in to their own servers (`RuntimeAccount::allowed_mcp_providers`). Name-clash reporting in the agent MCP sheet moves to step 4, with the list tool.
4. **`user_mcp` *manage* and `connect_mcp_server`.** Shared store and prompter traits in `everruns-core`; approval defaults; Platform Chat turns it on; an eval case in `evals/platform-capability` for "add Linear and connect it". Built: `UserMcpStore` and `McpLoginPrompter` in `crates/core/src/mcp/user_store.rs`; the tools in `crates/capabilities/src/capabilities/user_mcp/`, which hold `add` and `enable` behind a capability-owned durable approval gate; the control-plane store in `crates/server/src/domains/mcp_servers/user_manage/mod.rs`, which re-derives `manage`, `allow_custom_urls` and the initiating person itself and is reached in process or over the `InvokeUserMcpStore` worker RPC. Name clashes are reported by the list tool and, for the viewer's own servers, in the agent MCP sheet. Eval case `user-mcp-add-linear-and-connect`.
5. **`user_or_service` and connection-backed presets.** New `actsAs` value with per-call recorded identity; preset field naming a connection provider; GitHub preset backed by the agent's GitHub App. Built: `McpServerActsAs::UserOrService` and its `resolution_order` in `crates/contracts/src/runtime/mcp_server.rs`, composed by the default `UserConnectionResolver::get_mcp_connection_credential` so the worker RPC stays per identity; the identity a call used travels as `McpConnection::acted_as` to `ToolCompletedData::acted_as` through the per-call `McpCallIdentity` slot (`crates/contracts/src/runtime/mcp_proxy.rs`). Presets name a provider in their settings (`service_connection_provider`), pinned to the hosts that accept its tokens on save and on every resolution (`crates/server/src/domains/mcp_servers/connection_backed.rs`); the seeded `github` preset uses it. `connect_mcp_server` alone is derived for agents with a `user` or `user_or_service` server (`imply_mcp_connect_tool` in `crates/core/src/runtime_context.rs`), and the control-plane store routes a service server's card to the agent's sheet (`agent_server_login` in `user_manage.rs`). MCP Events triggers resolve the same way without a session: `DbConnectionResolver::agent_service_mcp_token` (`crates/server/src/storage/connection_resolver/mod.rs`) answers a connection-backed preset from the agent's named connection, with the same host pin, and an ordinary preset from the agent's MCP OAuth grant.
6. **`connectInChat` per attachment.** Built: `McpConnectInChat` on `ScopedMcpServer` in `crates/contracts/src/runtime/mcp_server.rs` (`connectInChat`, omitted when `ask`), carried by the resolved descriptor (`McpServerResolved`, the worker `McpServerInfo` and its proto field) onto `McpConnection::connect_in_chat`; `connection_required_result` in `crates/core/src/mcp/executor.rs` turns a missing grant under `never` into a tool error with the setup URL and no `connection_required`, so nothing pauses. `connect_mcp_server` gets the setting through `McpLogin::Pending::connect_in_chat` (`agent_server_login` in `user_manage.rs`) and returns the link instead of a card. The agent MCP sheet adds an **Ask to connect in chat** switch and reports the value on `AgentMcpAttachment::connect_in_chat`. A person's own servers have no attachment and always `ask`. OpenAI-hosted MCP calls already fail the turn with the link, unchanged.
7. **Deferred servers and session servers.** Per-server deferral through `tool_search`; ARD's session record generalized. Two PRs:
   - **7a, deferred servers.** Built: `ScopedMcpServer::deferred` (`deferred`, omitted when off) in `crates/contracts/src/runtime/mcp_server.rs`; the user layer copies it from the person's server row (`mcp_servers.deferred`, default true, set through the user MCP server API and the **Load tools on demand** switch in Settings > My MCP servers; chat-only servers are always deferred) in `user_layer.rs`, agent attachments opt in. `crates/contracts/src/runtime/mcp_deferred.rs` owns the rest: a deferred server not yet revealed is held out of discovery and stands in the turn as one never-deferred placeholder tool `mcp_<prefix>` (name and description; a real MCP tool always has `__`), and a reveal is a session KV record `mcp_reveal:<prefix>` (reserved from `kv_store`). `tool_search` in `crates/core/src/builtins/tool_search.rs` reveals a server when its query matches the placeholder and marks `mcp_<prefix>__*` as revealed, so the listed tools come with full schemas; calling the placeholder (`DeferredMcpServerTool`, registered by `build_mcp_proxy_tools`) writes the same record, which is the only way in on models whose tool search is provider-hosted. Turn-context loaders list revealed servers through the usual discovery and identity-scoped cache (`build_turn_mcp_tool_definitions` in `crates/server/src/domains/mcp_servers/deferred/mod.rs`, `discover_turn_tool_definitions` in `crates/core/src/host/mcp.rs`); the worker drops its kept turn reads on a reveal write (`crates/worker/src/reveal_storage.rs`) so the tools arrive on the turn's next step. A reveal lasts for the session. The agent MCP sheet adds a **Load tools on demand** switch and reports `AgentMcpAttachment::deferred`.
   - **7b, session servers.** Built: one session KV record per server, `session_mcp:<name>` (`SessionMcpServer` with its source, ARD or `user_mcp`, in `crates/core/src/session_mcp_servers.rs`, reserved from `kv_store`). ARD `attach_resource` writes it for an MCP target next to its `ard_attach:` record (which still drives idempotency, the attachment cap, `list_attached_resources` and external agents); `add_user_mcp_server` with `scope: "chat"` writes it through the control-plane store (`UserMcpStoreCall::AddToChat`, `add_to_chat` in `user_manage.rs`), which builds the definition itself: catalog servers sign in as the person as they do from the list, custom servers need `allow_custom_urls` and cannot ask for OAuth (the sign-in needs the list row), and a name the agent or an ARD attachment already uses is refused, since a session server wins over the agent's own. `remove_user_mcp_server` and `connect_mcp_server` find chat-only servers before the list. `apply_session_attachments` folds the records into session `mcpServers` on every path that builds the MCP surface: the gRPC turn context and prefix resolution, and the in-process worker (`fold_session_records` in `crates/server/src/domains/mcp_servers/session_servers/mod.rs`). The in-process worker and gRPC prefix resolution did not fold ARD attachments before, so ARD MCP tools did not reach in-process turns and could not resolve for execution over gRPC. ARD MCP attachments made before this change have no session record and stop contributing tools (ARD is dev-only; re-attach). Chat-only servers are listed and removed outside the chat by `GET`/`DELETE /v1/sessions/{session_id}/mcp-servers[/{name}]` (`ListChatMcpServers`, `RemoveChatMcpServer` in `session_servers.rs`; `SESSION_VIEW` / `SESSION_MANAGE`, session in the caller's org; ARD records are neither listed nor removed there, since their `ard_attach:` record would be left behind), with the viewer's sign-in state, and the Chat header shows an **MCP** button for them once a conversation has one (`apps/ui/src/components/chat/chat-mcp-servers.tsx`).
8. **MCP catalog moves to Settings > Organization.** Built: the catalog page is `apps/ui/src/app/(main)/settings/mcp-catalog/page.tsx` (dialogs in `apps/ui/src/components/mcp/mcp-catalog-dialogs.tsx`), with the "used by N agents" column it already had (`used_by_agents` on the catalog entry) and the line that a preset does nothing until an agent or a person adds it. The main-navigation entry is gone for everyone; the Settings entry (`apps/ui/src/lib/settings-navigation.ts`) carries `policy: "mcp_server.manage"`, which `visibleNavigationSections` checks through `useNavigationPolicy` for Settings and command search, so only people who manage the catalog see it. Viewers without manage still reach the page read-only by URL. `/mcp-servers` and `/mcp-servers/*` redirect permanently to `/settings/mcp-catalog` (`apps/ui/next.config.ts`). The *My connections* tab, which had no URL of its own, became **MCP sign-ins for agent servers** in My agent experience (`apps/ui/src/components/connections/mcp-grants-panel.tsx`); `/settings/connections` still redirects there. Server setup links already pointed at `/settings/connections`, so none changed.
9. **yolop adopts** the store and prompter traits and the `user_mcp` capability after the next everruns release.

Steps 1 to 4 deliver use case 1. Use case 3 works on today's code once a GitHub
preset exists (agent server acting as `user`, in-chat Connect card); step 5
makes use case 2 not need a separate MCP login.

## Open questions

1. **User servers in scheduled work.** "Every morning, check my Linear" needs a
   standing permission for one agent to use one person's login unattended. Out of
   scope here; it should be an explicit, revocable delegation, never a default.
2. **GitHub's remote MCP server and GitHub App installation tokens.** Settled
   from source, not yet tried against the hosted endpoint: github/github-mcp-server
   lists installation tokens (`ghs_`) as a supported Authorization type
   (`pkg/utils/token.go`) and its auth middleware rejects no token type, so the
   agent's installation token is accepted. The token acts as the App, so it
   reaches only repositories the App is installed on, with the App's
   permissions, and person-centric tools (`get_me`, notifications, starred
   repositories, "my" issues and PRs) fail because GitHub's `/user` endpoints
   reject installation tokens. Those need the server attached acting as `user`.
3. **Admin policy on user servers.** Should an org be able to forbid custom
   URLs or limit people to catalog presets? Proposed: an org setting, off means
   catalog-only, on by default for self-hosted and off for hosted.
