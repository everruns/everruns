---
type: Decision
title: "Personal Access Token Scopes"
description: "Design for enforcing personal access token scopes: a two-scope vocabulary (`*`, `read`), one route classification checked at the auth extractor, fail-closed for unclassified routes, and the MCP, CLI and agent-key boundaries."
tags:
  - everruns
  - security
  - authentication
  - personal-access-tokens
---
# Personal Access Token Scopes

> Status: **Design, awaiting approval.** Nothing here is built. Today scopes are
> stored but not enforced, and creation accepts only `*` (EVE-701).

## Abstract

A personal access token (`evr_pat_`) authenticates as its user with the user's
full authority in every org they belong to
([Authentication](authentication.md#2-personal-access-token)). The
`personal_access_tokens.scopes` column exists, defaults to `["*"]`, and is read
into `ValidatedPersonalAccessToken`, but `BuiltinAuthBackend` drops it when it
builds `AuthUser`, so nothing downstream can see it. To avoid advertising a
control that is not honored,
[`normalize_pat_scopes`](../../crates/server/src/auth/personal_access_token_routes.rs)
rejects anything but `*`.

This concept decides how a narrower token works: which scopes exist, where they
are checked, what "read-only" means for each surface, and how to ship it in
PR-sized steps without loosening anything on the way.

## Goals and non-goals

Goals:

- A user can give CI, a dashboard, or a reporting script a token that can read
  but cannot change anything, so a leak of that token is a disclosure, not an
  account takeover.
- Enforcement is fail-closed: a route nobody classified is out of scope for a
  narrow token.
- `*` tokens, JWTs, cookies and MCP OAuth tokens behave exactly as today.

Non-goals:

- Org-bound tokens. A PAT stays user-scoped; per-request org selection is
  unchanged ([Token scope: user, not organization](authentication.md#token-scope-user-not-organization)).
  Narrowing to one org is a separate decision.
- Service accounts or machine credentials. That concept is the agent key on the
  [Agent Execution API](../integrations/agent-execution-api.md), owned there.
- Per-domain scopes (`agents:write`, `sessions:read`). See
  [Rejected options](#rejected-options).

## Decision 1: vocabulary

Exactly two scope strings:

| Scope | Meaning |
|---|---|
| `*` | Full access, today's behavior. The default when `scopes` is omitted. |
| `read` | Every route classified `Read`, in every org the user belongs to. Nothing else. |

A token carries exactly one of them. Creation accepts `[]`, `["*"]` or
`["read"]`; any other value, including `["*", "read"]`, keeps failing with
`400 unsupported_pat_scopes`.

Stored rows are parsed fail-closed into one value:

- exactly `{"*"}` is full;
- a set that contains `read` and nothing outside the known vocabulary is read;
- anything else (empty, unknown strings, legacy `read`+`write`, `*` mixed with
  another scope) authenticates but is in scope for no route.

The column default `'["*"]'` (`crates/server/migrations/001_base_schema.sql`)
means every token minted since EVE-701 parses as full. Rows minted before
EVE-701 may hold arbitrary strings. Those are dropped rather than honored; see
[Legacy narrow tokens](#legacy-narrow-tokens).

The grammar reserves `<area>:<access>` for later without shipping it.

## Decision 2: route classification

Every route a PAT can reach gets one access class:

| Class | A `read` token | A `*` token |
|---|---|---|
| `Read` | allowed | allowed |
| `Write` | 403 | allowed |
| `Never` | 403 | unchanged from today |

`Never` is equal to `Write` for the two scopes we ship. It exists so that a
future narrower write scope can never reach credential and membership routes
by being given "write". It marks routes no scoped token should ever reach
([Decision 4](#decision-4-what-a-scoped-token-can-never-do)).

### Read-only is a declaration, not the HTTP method

The method is the wrong signal in both directions. POST endpoints that only
read exist (`validate_agent_package`, `diff_agent_package`, search and preview
helpers, and `POST /v1/commands/{name}` for every read command). GET endpoints
with side effects exist too: `GET /v1/auth/cli/callback` binds a CLI login to
the caller, `GET /v1/providers/{id}/oauth/authorize` starts an OAuth grant,
`GET /v1/slack/install` starts an install, and
`GET /mcp/elicitations/connect` starts a connection. So the class is declared,
and the method is only a review hint.

Two sources, one answer per `(method, route template)`:

1. **Command-backed routes** (`#[command(http = ..)]`, served by
   [`command_http`](../../crates/server/src/api/command_http/mod.rs)) derive the
   class from the command: `Read` when `Command::read_only()` holds and the
   command is not in `excluded_from_read_only_query`
   ([`catalog.rs`](../../crates/server/src/services/command_catalog/catalog.rs)),
   else `Write`. This is the same predicate that builds the read-only MCP
   `query` toolset, already reviewed under TM-MCP-002 with its POST allow-list
   in `crates/server/tests/domain/command_policy_enforcement_test.rs`. The
   predicate moves into one shared function, so the MCP `query` toolset and PAT
   scopes cannot drift apart.
2. **Hand-written routes** get an explicit entry in one table, a new
   `crates/server/src/auth/pat_scope/routes.rs`. There is no default from the
   method. A non-GET entry marked `Read` must also appear in a short reviewed
   list in the same file, so making a POST readable takes a deliberate second
   edit that a reviewer sees.

`POST /v1/commands/{name}` is classified `PerCommand`: the route layer lets it
through and
[`execute_command`](../../crates/server/src/api/command_dispatch.rs) applies
source 1 to the named command. An unknown name stays 404. A known command
outside the token's scope is 403, so the 403 does not reveal anything the
command contract (`GET /v1/commands`) does not already list.

### Fail closed

A route with no classification is out of scope for a `read` token. That covers
three cases without special handling: a new route added without an entry,
routes merged in by an embedder (SaaS `extra_routes`, backend-provided
`auth_routes()`), and root routes outside `/v1` (channel ingress, webhooks,
elicitation pages). An embedder that wants its routes readable registers
classifications through a new `ServerAppBuilder` hook. Otherwise its routes
stay denied for narrow tokens, which is the safe default.

## Decision 3: one enforcement point

Authentication happens in extractors, not in a router middleware: every
authenticated handler reaches `extract_auth_user`
([`middleware/mod.rs`](../../crates/server/src/auth/middleware/mod.rs)) through
`AuthUser`, `ResolvedOrg`, `OrgContext`, `AdminUser`, `OrgAdmin` and friends.
The extractor knows the token. It does not know the route. A layer knows the
route but would have to validate the token a second time. The design splits
the two:

1. **Classify in a layer.** One `from_fn` layer on the assembled `/v1` router,
   applied next to `decorate_api_routes` in
   [`app_builder`](../../crates/server/src/app_builder/mod.rs), reads axum's
   `MatchedPath` and the method, strips the configured API prefix, and inserts
   a `RouteAccess` request extension from the two sources above. It does not
   look at credentials.
2. **Enforce in the extractor.** `BuiltinAuthBackend` carries the parsed scope
   onto `AuthUser` (a new field; `None` for every non-PAT method). Right after
   PAT validation, `extract_auth_user` compares that scope with the
   `RouteAccess` extension. A missing extension means unclassified, which is
   denied. A `*` token, a JWT and a cookie skip the check entirely.

This makes the extractor the only place a scope decision is made, before org
resolution or any handler work, and it stays correct when a handler takes
`AuthUser` twice. The PAT cache in `BuiltinAuthBackend` caches the parsed scope
with the user. Scopes are immutable after creation, since there is no update
route, so no invalidation is needed.

Code that validates a PAT without the extractor is listed in
[Decision 5](#decision-5-other-surfaces-and-credentials) and handled at its own
call site: `extract_mcp_auth_user` and the execution API's `org_member`. The
runtime sweep test in [Tests](#tests) catches any other.

**Defense in depth (optional, last step).** `ResolvedOrg` → `Caller` carries a
`read_only` flag, and `Command::run` refuses a non-read-only command for a
read-only caller. That also covers a hand-written handler that dispatches a
write command behind a route someone misclassified as `Read`. `Caller` lives in
`everruns-core` and is built in many places, so this is a separate PR and not
a prerequisite.

## Decision 4: what a scoped token can never do

Classified `Never`, whatever the method:

- Token management: all of `/v1/auth/personal-access-tokens*`, including list.
  A PAT already cannot mint a PAT
  (`create_personal_access_token` rejects `AuthMethod::PersonalAccessToken`);
  that rule stays for `*` tokens too.
- The auth surface, `/v1/auth/*` (login, register, refresh, recovery, OAuth,
  CLI login start, callback and exchange), except `GET /v1/auth/me`, which is
  `Read` so `everruns whoami` works.
- Membership and org lifecycle: invitations (create, revoke, accept), member
  role changes and removal, org create and delete, `switch-org`, account
  delete.
- Credential minting or binding: agent keys (create, rotate, revoke), eval-run
  share creation, provider and user-connection OAuth authorize and callback,
  GitHub App setup, Slack install, connected MCP client revocation, and session
  and agent secret writes.
- The full personal data export, `GET /v1/users/me/export`. It is a read, but
  it hands over everything the user owns across every org in one response, so
  a leaked `read` token would turn into a bulk exfiltration path. Single
  resources stay readable; the bundle does not.
- Any future read that returns plaintext credential material. No route does
  today: tokens and keys are shown once in the POST that mints them, and secret
  listings return names only. The class exists so such a route is classified
  `Never`, not `Read`.

A `*` token keeps today's authority exactly: every route it reaches now, it
reaches afterwards. Its only restriction stays the existing ban on minting
PATs.

## Decision 5: other surfaces and credentials

| Surface or credential | Behavior |
|---|---|
| REST `/v1` | Decisions 2 and 3. |
| `/v1/commands` (CLI) | `GET` lists the commands the token can run (the `query` toolset for `read`), each still marked `read_only`. `POST /v1/commands/{name}` checks per command. |
| `/mcp` with a PAT | `extract_mcp_auth_user` carries the scope. For `read`, `tools/list` returns only tools annotated read-only (`me`, `list_organizations`, `discover`, `query`, `session_get_status`, card and app views), and `tools/call` refuses `execute`, `agent_run`, `session_send_message`, `session_set_secret`, `connect` and mutating app actions. The tool classification is an explicit list in `tool_registry.rs`, kept equal to its `readOnlyHint` annotations by a test, so a hint is never the only control. |
| MCP OAuth tokens | Unchanged: resource-bound, full user authority on `/mcp` only (TM-MCP-006). If consent-time scopes come later, they reuse the same tool classification. |
| JWT and cookie sessions | Unchanged. No scope. |
| Share tokens (`evr_share_`) | Unchanged. Not a principal: anonymous read of one eval run. A `read` PAT cannot create one (`Never`). |
| Agent keys (`evr_ak_`) | Owned by the [Agent Execution API](../integrations/agent-execution-api.md); this design does not touch them. Boundary: a `read` PAT cannot create, rotate or revoke one, and it is not accepted as an "org members" caller on an API channel, because running an agent is a write. Only a `*` PAT qualifies in `agent_api_auth.rs::org_member`. |
| CLI (`everruns`) | `everruns login` keeps minting `*` through `/v1/auth/cli/exchange`. `login --token` accepts a `read` token. `whoami` shows the scope, from a new `token_scope` field on `GET /v1/auth/me` (null for non-PAT). The CLI maps `insufficient_scope` to one line naming the scope the token lacks. A `login --read-only` flag that passes `scopes` to the exchange is optional. |

## Decision 6: error shape and logging

An out-of-scope call returns:

- `403 Forbidden`, rendered through `AuthError` as RFC 9457 problem+json, with
  `code: "insufficient_scope"`. The code is RFC 6750's error name.
- `WWW-Authenticate: Bearer error="insufficient_scope"` (RFC 6750 §3.1).
- A fixed message: "This personal access token is read-only; this operation
  needs a full-access token." It includes no route or permission detail.

Ordering: authentication (401) comes first, then scope (403), then org
resolution and handler authorization. A `read` token therefore gets 403 on a
write route before org membership is checked. That reveals nothing, because
classification depends only on the route template, never on the resource.

On MCP, the refusal is a tool error result whose `classify_mcp_execute_error`
envelope kind is `insufficient_scope`, matching how other tool refusals are
reported.

Logging: every denial emits one `tracing::warn!` with `token_id`, `user_id`,
method, matched route template and class, never the token, and increments a
`pat_scope_denied_total{class}` counter. Denials are not written to the org
audit log: a PAT is user-scoped and often has no resolved org at that point,
and a misconfigured CI loop would flood it. The existing
`auth.personal_access_token.created` audit event gains `scopes` in its
details, so an org admin can see when a full token was minted.

## Threat model changes

Recorded in [Threat Model](threat-model.md) by the PR that enforces each item,
not by this design:

- **New TM-AUTH-037**, scoped PAT exceeds its scope through a misclassified,
  unclassified or alternate-surface route (High). Mitigation: Decisions 2, 3
  and 5. It is MITIGATED only when creation of `read` is enabled (last gate).
  The entry names the data export as `Never`, so a leaked `read` token reads
  resources one by one but never pulls the whole account in one request.
- **New TM-AUTH-038**, a leaked `read` token (Medium, ACCEPTED). It still reads
  every org the user belongs to, including session transcripts and files. That
  is the documented blast radius, and the reason the UI says "read access to
  all your organizations", not "safe to share".
- **TM-AUTH-005** (PAT exposure): the mitigation adds "issue `read` tokens for
  read-only automation".
- **TM-MCP-002**: the same read-only predicate now also gates `read` PATs on
  REST and `/v1/commands`, so its inventory test protects both.
- **TM-MCP-006**: notes that a PAT on `/mcp` honors its scope.
- **TM-TENANT-004** is unchanged: scopes narrow actions, not orgs.

## Tests

Each test fails if the property it names breaks:

1. **Scope parsing** (unit): `*`, `read`, empty, unknown, and legacy mixes
   parse to full, read, read, none, none as Decision 1 says. Creation accepts
   only the three allowed request shapes.
2. **Route coverage** (`crates/server/tests/domain/pat_scope_coverage_test.rs`):
   the universe is the OpenAPI document (`ApiDoc` plus `add_command_paths`),
   which `openapi_coverage_test.rs` already keeps equal to the annotated
   handlers. Every `(method, path)` must resolve to a class, from the command
   inventory or the table. Every table entry must name a route in that
   universe, so there are no stale rows. Every non-GET `Read` entry must be in
   the reviewed list. A new route without a classification fails this test in
   CI. If it shipped anyway, it would be denied, not opened.
3. **Runtime sweep** (integration, real router): for every route in the
   universe, a request with a `read` PAT and a placeholder path. Each route
   classified `Write` or `Never` must answer exactly `403 insufficient_scope`.
   This catches a handler that validates tokens itself and skips the
   extractor. The same sweep with a `*` PAT must never return
   `insufficient_scope`.
4. **Extractor** (middleware tests): `read` on a `Read` route passes, on
   `Write` and on a route with no `RouteAccess` gets 403 with the header. A
   JWT, a cookie and a `*` PAT on an unclassified route pass as today.
5. **Predicate parity**: the classification for command-backed routes equals
   membership in the MCP `ToolsetMode::ReadOnly` toolset.
6. **Surfaces**: `/v1/commands` lists and runs only read commands for `read`.
   MCP `tools/list` and `tools/call` filter as Decision 5 says, and the tool
   list equals its annotations. The execution API rejects a `read` PAT as an
   org member. `GET /v1/auth/me` reports `token_scope`.
7. **End to end**: the CLI with a `read` token lists agents and fails
   `agents create` with the scope message.

## Rollout

One PR per step. Creation of `read` opens last, so no user can hold a narrow
token while any surface still ignores it.

0. **Data check (no PR).** Count production rows whose `scopes` is not
   `["*"]`, so the step 2 deletion has a known size.
1. **Classification, no enforcement.** Scope parsing, the scope field on
   `AuthUser`, the `RouteAccess` layer, the shared read-only predicate, the
   route table, and tests 1, 2 and 5. Nothing changes on the wire. SaaS's
   `AuthUser` struct literals need the new field when SaaS adopts this.
2. **Enforce on REST and `/v1/commands`.** Extractor check, error shape,
   logging and metric, per-command check, and tests 3 and 4. The same PR
   carries the migration that deletes legacy non-`*` rows, so no token is
   narrowed by surprise.
3. **Enforce on MCP and the execution API.** Tool classification and filtering,
   the `org_member` boundary, and test 6.
4. **Open creation.** `normalize_pat_scopes` accepts `["read"]`. Add a scope
   picker to the token UI, `token_scope` on `/v1/auth/me`, `scopes` in the
   creation audit event, public docs in `docs/`, and the
   [Authentication](authentication.md) update. TM-AUTH-037 and TM-AUTH-038
   land here.
5. **CLI.** `whoami` scope display, the `insufficient_scope` message, and
   optionally `login --read-only`. Test 7.
6. **Optional defense in depth.** `Caller.read_only` checked in
   `Command::run`.

## Rejected options

- **Classify by HTTP method alone.** Wrong in both directions (see
  [Decision 2](#read-only-is-a-declaration-not-the-http-method)), and silently
  opens any new side-effecting GET.
- **Per-route `route_layer` annotations at each router.** The declarations
  would be scattered over about 490 `.route(` calls with no single place to
  review or test, and a merged router that forgets the layer fails open unless
  the extractor still denies a missing class. That extractor check is the core
  of this design anyway. A central table plus inventory derivation keeps the
  review surface in one file.
- **Check scopes only in `Command::run`.** Many routes are hand-written and
  never dispatch a command, so this alone fails open. It stays as optional
  defense in depth.
- **Per-domain scopes now.** Cheap for command-backed routes (`CommandMeta`
  has a category) but not for the hand-written ones, which would each need an
  area as well as a class. Nobody has asked for it yet. The `<area>:<access>`
  grammar is reserved.
- **Org-bound PATs.** A different credential model; see
  [Non-goals](#goals-and-non-goals).

## Legacy narrow tokens

Rows minted before EVE-701 may store scopes other than `["*"]` (for example
`["read"]` or `["read", "write"]`). Nobody chose those with this design in
mind, so neither reading them as `read` nor widening them to `*` matches what
their owner asked for. Step 2 drops them: a migration deletes every
`personal_access_tokens` row whose `scopes` is not exactly `["*"]`. Their
owners get a 401 and mint a new token, choosing full or read access
deliberately. Step 0 counts the rows first so the number is known before the
migration ships.

## See also

- [Authentication](authentication.md), PAT semantics and the CLI login flow
- [Permissions Model](permissions.md), role and policy checks that run after
  the scope check
- [Command Tree](../execution/command-tree.md), the command surface the CLI and
  MCP share
- [Agent Execution API](../integrations/agent-execution-api.md), agent keys and
  the org-members opt-in
- [Threat Model](threat-model.md)
