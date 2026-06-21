# App API Keys (execution-scoped)

## Abstract

An **App API key** is an app-scoped, execution-only credential that lets an
external integrator drive an App's agent over Everruns' native session API
(`/v1/sessions/*`) without holding any management access. It is the
programmatic counterpart to a Personal Access Token (PAT): where a PAT is the
**management plane** (user-scoped, full account access — see
[`authentication.md`](authentication.md)), an App API key is the **execution
plane** (one App, run sessions only).

This is the "new, separate concept" that `authentication.md` anticipates: a
narrow, non-human credential. It is deliberately **not** a PAT variant — PATs
stay user-scoped and full-access, and their dormant `scopes` column is not the
mechanism here.

The App is the boundary. The key inherits exactly what the App can do
(its Harness + Agent), and nothing else. It cannot create or modify agents,
harnesses, apps, providers, settings, or other keys.

## Goals

1. Hand an integrator a credential that can **run sessions for one App** and
   read their results, with no path to management APIs.
2. Reuse the existing native session surface (`/v1/sessions/*`) rather than
   forking a parallel public API — one surface, projected through the App
   boundary.
3. Extend the existing shared App-endpoint `api_key` auth scheme
   ([`app-endpoint-auth.md`](app-endpoint-auth.md)) rather than inventing new
   auth machinery.
4. Confine exposure: external callers see the App's public projection of
   session activity (the AG-UI `tool_visibility` model), not raw internal tool
   names, arguments, or results, unless the App opts in.
5. Make keys first-class: listable, rotatable, revocable, hashed at rest.

## Non-Goals

1. Org-scoped "service account that can run any agent." That contradicts the
   apps-as-boundary model; if ever needed it is a separate concept again.
2. Replacing A2A. A2A ([`a2a-channel.md`](a2a-channel.md)) exposes the App over
   the A2A JSON-RPC protocol for agent-to-agent callers. App API keys expose the
   **native** session API for first-party integrators. Both are app-scoped,
   execution-only ingress; they differ only in protocol shape and may coexist on
   the same App.
3. Granting management permissions through scopes. There is intentionally no
   "management scope" on this key. Management stays on PATs / interactive users.
4. Token issuance for third-party IdPs — that is the OIDC/OAuth side of
   [`app-endpoint-auth.md`](app-endpoint-auth.md).

## Concept

App API keys are modeled as an **App channel** of a new
`ChannelType::ApiEndpoint` (`"api_endpoint"`), already reserved as a future
channel type in [`apps.md`](apps.md). Modeling it as a channel keeps it uniform
with `a2a` / `ag_ui` (own enabled flag, own config, own lifecycle, published-app
gate) and lets one App expose several keys with independent settings.

Channel config carries:

- The generated key material — `api_key_hash` (SHA-256 hex) and non-secret
  `api_key_prefix` for display. Plaintext returned **once** at create/rotate.
  Mirrors A2A key handling in [`a2a-channel.md`](a2a-channel.md). The default
  auth policy is the generated key; `channel_config.auth` may instead attach any
  shared App-endpoint auth mode (OIDC/OAuth2/HTTP Basic/mTLS).
- `session_mode: InvocationSessionMode` — `shared_session` vs
  `session_per_invocation`, reusing the app-channel routing semantics.
- An **exposure policy** mirroring AG-UI: `tool_visibility`
  (`none` | `generic` | `narrated`) and `generic_tool_text`. Default `generic`
  so raw internal tool detail is never exposed by default.
- Optional `rate_limit_per_minute`, reusing the shared `ChannelRateLimiter`
  primitive (same as A2A / AG-UI), namespaced `apikey`.

Key format: `evr_app_<64 hex chars>` (32 random bytes, 256-bit entropy),
prefix-scoped so secret scanners target it distinctly from `evr_pat_` and
`evra2a_`.

## Caller and authorization

A request authenticated by an App API key resolves to a non-human, app-bound
`Caller` (see [`permissions.md`](permissions.md)):

- `org_id` is the App's org. `user_id` is `None`. The caller is **not**
  `is_internal` (it must still pass policy) and **not** `is_platform_user`.
- A new principal/caller kind marks it as an **app execution** caller carrying
  the `app_id` (and `app_channel_id`) it is bound to.

Authorization is the existing `Command::run` + `PermissionResolver` contract —
no new enforcement point. Two pieces:

1. **Execution-only permission set.** Introduce a narrow
   `OrgSessionsExecute` permission, distinct from the existing
   `OrgSessionsManage`. The resolver grants an app-execution caller *only*
   `OrgSessionsExecute` (plus session read), and nothing else. Because every
   mutating command declares a policy and the resolver fails closed, an App key
   automatically cannot satisfy `OrgAgentsManage`, `OrgHarnessesManage`,
   app/provider/settings/key-management policies, etc. — they 403 without any
   per-endpoint special-casing.
2. **App confinement (a `Rule`).** `OrgSessionsExecute` alone is org-wide;
   the App boundary is enforced by a `Rule` requiring that the target session
   belong to the caller's App. On create, the session's Harness/Agent are taken
   from the App — client-supplied `harness_id` / `agent_id` / capability
   overrides are rejected, not honored. On read/message/cancel, the session must
   carry the caller's `app:{app_id}` routing tag, so one App's key cannot touch
   another App's sessions.

This keeps the split crisp: **PAT satisfies management policies; App key
satisfies only execution policies, only within its App.**

## Endpoints (reuse `/v1/sessions/*`, app-projected)

No new session API surface. The same endpoints serve App-key callers, projected
through the App boundary and exposure policy. Relative to the management
behavior in [`apis.md`](apis.md):

| Endpoint | App-key behavior |
|----------|------------------|
| `POST /v1/sessions` | Creates a session **bound to the caller's App**; Harness/Agent come from the App, management-only fields rejected. Honors `session_mode`. |
| `POST /v1/sessions/chat` | Not available to App keys (global chat is a management/UI surface). |
| `GET /v1/sessions` | Lists only the App's sessions. |
| `GET /v1/sessions/{id}` | Allowed only for the App's sessions; else 404 (not 403, to avoid cross-App existence probing). |
| `POST /v1/sessions/{id}/messages` | Allowed for the App's sessions. |
| `POST /v1/sessions/{id}/cancel` | Allowed for the App's sessions. |
| `GET /v1/sessions/{id}/events`, `GET /v1/sessions/{id}/sse` | Allowed for the App's sessions, **projected through the channel exposure policy** — raw tool names/args/results suppressed per `tool_visibility`, matching the public AG-UI contract. |

Auth runs through the shared App-endpoint verifier
([`app-endpoint-auth.md`](app-endpoint-auth.md)) before session lookup or
dispatch. Failures are generic 401/403 so callers cannot distinguish
misconfiguration from probing. Only **published** Apps with an **enabled**
`api_endpoint` channel accept traffic; unpublish/disable stops new work, existing
sessions remain.

## Management surfaces

Key lifecycle lives under the App, like other channels:

- `POST /v1/apps/{id}/api-endpoint-channels` — create channel, return plaintext
  key once.
- `POST /v1/apps/{id}/api-endpoint-channels/{channel_id}/regenerate-key` —
  rotate; invalidates the previous key.
- `PATCH /v1/apps/{id}/channels/{channel_id}` — update non-secret fields
  (session mode, exposure, rate limit, `auth`).
- `DELETE /v1/apps/{id}/channels/{channel_id}` — remove.

These management operations are gated by the App-management policy (a human /
PAT), **not** by the App key itself — a key cannot mint, rotate, or read sibling
keys.

## Audit logging

Reuse the shared app-channel invocation audit path
([`a2a-channel.md`](a2a-channel.md) "Audit Logging"): on session dispatch emit
`agent` / `agent.app_invocation.started` with `source = "app_api_key"`,
`app_id`, `app_channel_id`, `app_channel_type = "api_endpoint"`, `session_id`,
`created_session`, and the App owner principal id. Actor is none (external
key-holder, not an Everruns user).

## Threat model

New `specs/threat-model.md` entries to add (TM-APIKEY-*):

- **Privilege confinement** — an App key must never satisfy a management policy.
  Covered by the execution-only resolver grant + the inventory test in
  `command_policy_enforcement_test.rs` (every mutating command has a policy).
- **Cross-App isolation** — an App key must not read or drive another App's
  sessions. Covered by the App-confinement `Rule` and 404-on-foreign-session.
- **Exposure leakage** — internal tool detail must not leak to external callers.
  Covered by routing events/SSE through the channel exposure policy
  (`tool_visibility`), reusing the AG-UI projection (`TM-LLM-020` neighborhood).
- **Key storage / rotation** — hashed at rest, plaintext once, rotation
  invalidates prior keys. Mirrors A2A (`TM-A2A-*`).

## Open questions

1. **Confinement mechanism.** App confinement can be a dedicated `Rule`
   evaluated against the resolved session's app tag, or folded into a
   resource-ownership check once `created_by`/owner lineage lands
   (`permissions.md` Phase 2). Prefer the explicit `Rule` for the first cut.
2. **Session listing default.** Whether `GET /v1/sessions` for an App key
   should require an explicit `app_id`-implied filter (it is implied by the key)
   or also accept paging the same way the management list does.
3. **Reusing A2A's session routing tags** verbatim vs. a distinct
   `app_channel_type:api_endpoint` tag — the latter keeps per-channel-type
   metrics and rate-limit buckets disjoint.

## References

- [`apps.md`](apps.md) — App entity, channels, lifecycle
- [`app-endpoint-auth.md`](app-endpoint-auth.md) — shared inbound auth verifier
- [`a2a-channel.md`](a2a-channel.md) — sibling execution-only channel (A2A protocol)
- [`authentication.md`](authentication.md) — PATs and the management plane
- [`permissions.md`](permissions.md) — `Permission`, `Rule`, `Policy`, `Caller`
- [`apis.md`](apis.md) — native session API surface
