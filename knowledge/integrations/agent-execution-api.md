---
type: Proposal
title: "Agent Execution API"
description: "Expose one agent and its execution to code: API channel, agent keys, customer OAuth, session and run routes, a management vs execution split shared with serve, and the SDK as an agent client."
tags:
  - everruns
  - integrations
  - api
  - serve
  - sdk
---
# Agent Execution API

> Status: **Proposal**, not built. Owner decisions recorded under [Decisions](#decisions).

## The scenario

1. A user builds an agent in Everruns that does some job.
2. They want to call that agent from their own code or a script.
3. They need a way to expose exactly that agent, securely, without handing out their account.

Requirements from the ask: a full session API plus a simpler task style API, token auth, custom OAuth, ideally several agents behind one credential, an agent-only SDK, and a clean split between the management API and the execution API that lines up with `serve`.

## What already exists (and why it is close, but not enough)

| Piece | Where | What it gives us | Gap |
|---|---|---|---|
| Agent channels | `crates/server/src/records/agent_channel.rs`, `knowledge/integrations/agent-exposure.md` | Agent-owned "doors": status draft/live/disabled, owner principal, service virtual user, `auth`, budget subject, per-channel publish, agent-level suspend | none, this is the right home |
| `api_endpoint` channel | `crates/server/src/api/channel_api.rs`, `knowledge/integrations/app-api-keys.md` | `POST /v1/channels/{id}/sessions`, get, message, cancel. Execution-only key `evr_app_…`, sessions confined by routing tags | **Frozen**: no way to create or rotate keys. Polling only, text only, no SSE, no questions/approvals, reads show only completed assistant messages |
| Channel auth verifier | `crates/server/src/api/channel_auth.rs`, `knowledge/integrations/channel-auth.md` | `shared_secret`, `api_key`, `google_oidc`, `oidc` (JWKS), `oauth2_introspection`, `http_basic`, `mtls`, with audience/scope/claim requirements; AgentID preset | One mode per channel; no Everruns-managed keys |
| Caller → virtual user | `storage/runtime_identity.rs`, `knowledge/runtime-resources/virtual-users.md` | Verified (provider, realm, subject) becomes an end-user virtual user; MCP `actsAs: user` then spends that user's grants | Shared keys identify an application, not a person; no way for a trusted backend to say "acting for customer 42" |
| Runtime auth exchange | `POST /v1/channels/{id}/runtime-auth` (`api/runtime_auth.rs`) | Swaps a verified channel identity for a 15 minute end-user JWT bound to one channel | Only AG-UI accepts it today |
| A2A and AG-UI channels | `api/channel_a2a*`, `api/ag_ui.rs` | Protocol doors with blocking send, tasks, streaming, agent card | Protocol-shaped; fine for agents and CopilotKit, awkward for "my Python script" |
| `/v1` sessions API | `api/sessions.rs`, `api/events.rs` | Full session surface: messages, SSE with `since_id`/`after_sequence`, cancel, question-answers, tool-approvals, tool-results | Needs a PAT, which is the user's whole account in every org (`knowledge/security/authentication.md`, PAT scopes not enforced) |
| MCP `agent_run` | `api/mcp_endpoint/mod.rs` | One-shot "create session + send" | Management credential, not an exposure |
| serve | `crates/serve`, [`crates/serve/docs/wire-api.md`](../../crates/serve/docs/wire-api.md) | A declared subset of server `/v1` sessions with the same JSON; several agents per app; AG-UI and A2A already at `/v1/channels/{agent}/ag-ui` and `/a2a` | No auth at all; shapes are ad hoc `json!`, not shared types |
| SDK | `everruns/sdk` (`everruns-sdk` on crates.io/PyPI, `@everruns/sdk`) | Org-wide management client plus a basic session loop | PAT only (`Authorization: <token>`, no Bearer), no OAuth/token refresh, no notion of "one agent" |

So the building blocks are almost all here. The one honest gap is that the native API door was frozen after Apps were retired, and nothing replaced it. A previous option, letting a scoped key into `/v1/sessions`, was dismissed for blast radius (`knowledge/project/dismissed-options.md`, "App API key: project through the global /v1/sessions surface"). This design keeps that decision: execution credentials never reach the management routes; instead the execution routes get the full surface.

## The core idea

**Two APIs, one contract each.**

- **Management API**: everything under `/api/v1/*` you use to build and operate agents (agents, harnesses, channels, keys, budgets, workspaces, `/v1/commands`, MCP `execute`). Authenticated as an Everruns user (cookie, JWT, PAT). Served only by the Everruns server. Client: today's `everruns-sdk` and the CLI.
- **Execution API**: everything a caller needs to *talk to one agent*: its card, sessions, messages, events, runs, answers to questions and approvals. Authenticated as a caller of that agent (agent key, the customer's OAuth token, or a short-lived runtime token). Never reaches management. Served by the Everruns server **and** by `serve` (and therefore AgentCore and celld) with the same wire shapes. Client: a new agent client.

The execution API is rooted at an **agent base URL**, and every route is relative to it:

| Host | Agent base URL |
|---|---|
| Everruns server | `https://app.everruns.com/api/v1/channels/{channel_id}` (the existing `/v1/e/{channel_id}` short alias stays only for compatibility and is not documented) |
| serve app | `http://localhost:3000/v1/channels/{agent_name}` |
| serve on AgentCore | the AgentCore endpoint, forwarding to the same paths |

This is not a new address scheme. serve already mounts AG-UI and A2A at `/v1/channels/{agent}/…` and the server already mounts every door at `/v1/channels/{channel_id}/…`; the agent id slot is a channel id on the server and the agent name in serve. A caller only ever holds one URL and one credential.

## The model

### API channel

A new channel type `api` (the successor of the frozen `api_endpoint`, which stays served for existing keys and is migrated later). It is an ordinary `agent_channels` row, so it inherits everything that already works: draft/live/disabled, agent suspend, owner principal, service virtual user, `agent_channel` budget subject, Exposures view, audit.

Channel config adds only what is specific to code callers:

- `session_binding`: `Requester` by default (each caller sees its own sessions), or `Ephemeral` (a new session per run). `Channel` (one shared session) allowed but warned about.
- `visibility`: what the event stream shows. `messages` (assistant text only, today's api_endpoint), `activity` (plus tool start/finish with the channel's public tool activity text, the policy Slack and AG-UI already share), `full` (raw canonical events: tool names, arguments, results, reasoning, usage). Default `activity`. `full` is for trusted callers who own both ends.
- `errors`: `public` (the `PublicError` four codes, required when the channel allows anonymous or end-user tokens) or `detailed` (problem+json with internal codes, only for key-authenticated developer callers).
- `client_tools`: allow the caller to declare client-side tools (the existing `tool-results` flow). Off by default.
- `cors_origins`: needed only when the browser calls with a runtime token.
- `limits`: rate limit (existing channel rate limiter), max concurrent sessions per caller, max run wait.

### Callers

Every request resolves to a **caller** before anything else, the same way ingress already resolves identity before rate limiting and session lookup:

| Credential | Who the caller is | Session ownership / visibility |
|---|---|---|
| Agent key (`evr_ak_…`) | the application holding the key (`key:{key_id}`) | sessions tagged with the key; a rotated key keeps its id so sessions survive rotation |
| Agent key + `End-User` assertion | a customer of that application: end-user virtual user via provider binding (provider `agent_key`, realm = the key's id, subject = asserted id) | sessions tagged with that end user; the backend can list its customers' sessions only by naming them |
| Customer OAuth token (OIDC JWT or introspected opaque token) | the token's (issuer, `sub`): end-user virtual user via provider binding, exactly as AG-UI does today | sessions tagged with that end user |
| Runtime token (from `/runtime-auth`) | the end user it was minted for, bound to this channel only | same as above; for browsers and mobile apps |
| Anonymous (opt-in) | a bounded visitor, as Public Chat does | ephemeral only |

Sessions stay owned by the channel's `owner_principal_id` (an invariant from agent-exposure.md); the caller tag is what confines reads, and lookups keep tag containment keyed on org and owner. A caller can never name a session it did not create: wrong caller and missing session return the same 404.

End-user identity matters beyond confinement: it decides whose connections the agent may use. With `actsAs: user`, a turn started by "customer 42" uses customer 42's grants, never the key holder's. That is the virtual-users rule "shared endpoint tokens identify an application, not an individual end user" applied directly.

## Execution API surface

All paths are relative to the agent base URL. JSON shapes are the server's existing `Session`, `Message`, canonical event envelope, SSE framing (`connected`, `id:` on durable events, `since_id` / `after_sequence`, heartbeat, `disconnecting`), and RFC 9457 errors. Anything added is added to both hosts.

### Discovery

| Method | Path | Notes |
|---|---|---|
| GET | `/` | Agent card: name, description, avatar, conversation starters, accepted input (text, images, files), `run` support, streaming, accepted auth schemes, links to sibling AG-UI / A2A doors of the same agent if they are live. serve's existing `GET /v1/agent` becomes this per agent. |

### Session style (full conversation)

| Method | Path | Notes |
|---|---|---|
| POST | `/sessions` | `{title?, metadata?, hints?, idempotency_key?}`. No `agent_id`, no `harness_id`: the URL is the agent. |
| GET | `/sessions` | This caller's sessions only, paginated. |
| GET | `/sessions/{id}` | Status (`idle`, `running`, `waiting_for_input`, `failed`), pending questions and approvals. |
| POST | `/sessions/{id}/messages` | Same body as `/v1/sessions/{id}/messages` (content parts). Starts a turn if idle, steers if running. Returns the message and the `turn_id`. |
| GET | `/sessions/{id}/messages` | Conversation as the channel's `visibility` allows. |
| GET | `/sessions/{id}/sse` | Live events, filtered by `visibility`, resumable. |
| GET | `/sessions/{id}/events` | Paged events, same filter. |
| POST | `/sessions/{id}/cancel` | Cancel the running turn. |
| POST | `/sessions/{id}/question-answers` | Answers `ask_user`. |
| POST | `/sessions/{id}/tool-approvals` | Approve or deny a gated tool call. serve's `/approvals/{tool_call_id}` becomes an alias of this. |
| POST | `/sessions/{id}/tool-results` | Results for client-side tools, only when `client_tools` is on. |
| POST | `/sessions/{id}/files` | Upload inputs (images and files), bounded by the channel. |
| DELETE | `/sessions/{id}` | Caller-side delete (archive). |

### Task style (one input, one result)

A **run** is one turn, addressed on its own. It is the "call a function" shape scripts want, and it maps onto what already exists: A2A blocking send, AgentCore `/invocations`, MCP `agent_run`.

| Method | Path | Notes |
|---|---|---|
| POST | `/runs` | `{input, session_id?, wait?, output_schema?, metadata?}` plus `Idempotency-Key` header. Without `session_id` it creates a session per the channel binding (`Ephemeral` gives a fresh one each time). `wait` (seconds, capped by the channel) holds the response until the run finishes; otherwise `202` with the run. `Accept: text/event-stream` streams it instead. |
| GET | `/runs/{id}` | `{id, session_id, status, output: {text, data?}, pending_input?, usage?, error?}`. `data` is present when `output_schema` was given (the existing JSON Schema `response_format` support, EVE-1116). |
| GET | `/runs/{id}/sse` | Events of that turn only. |
| POST | `/runs/{id}/cancel` | |

Run status is derived from the turn lifecycle the way A2A derives task state today (`turn.started` → `running`, `turn.completed` → `completed`, parked `ask_user` or approval → `input_required`), so there is no new table: run id is the turn id. An `input_required` run is continued through the session routes (answer the question, then `GET /runs/{id}` again).

The word is **runs**, not tasks, because "task" already means two other things here: background session tasks (`/v1/tasks`, `knowledge/runtime-resources/session-tasks.md`) and A2A tasks.

### What the execution API deliberately does not have

Choosing the agent or harness, changing instructions, tools, models or capabilities, reading other callers' sessions, org listings, budgets, keys. Those are management. A caller who needs them is a member and uses the management API.

## Auth

### Agent keys (token auth)

Managed, Everruns-issued keys replace the frozen `evr_app_` keys:

- Prefix `evr_ak_`, shown once, stored as SHA-256 with a display prefix, constant-time compare (same as PATs and A2A keys).
- Owned by the org, created by a member with permission to manage the agent's channels; `created_by` kept for audit. Not tied to a person, so they survive the creator leaving. This is the "org-scoped machine credential" authentication.md says would have to be a new concept; it is, and it reaches only execution routes.
- Granted to one API channel (several agents per key is deferred, see below).
- Name, optional expiry, `last_used_at`, revoke, and rotate with an overlap window (old and new both valid for N hours).
- Permissions on the key, small and enforced: `sessions` (session routes), `runs` (run routes), `end_user` (may send `End-User` assertions). Default: `sessions` + `runs`.
- Header: `Authorization: Bearer evr_ak_…`. The agent client never sends a bare token (today's SDK sends `Authorization: <token>` with no scheme).

### Custom OAuth (the customer's identity provider)

Reuse the channel auth verifier as is. The customer configures their issuer on the API channel (`oidc` with discovery and JWKS, `oauth2_introspection` for opaque tokens, `google_oidc`, or the AgentID preset) with requirements such as audience `api://support-agent` and scope `agent:invoke`. Everruns acts only as a resource server: it validates tokens and never issues them, which matches the channel-auth non-goal. Their users call the agent with their own access tokens, and each `sub` becomes its own end-user virtual user, so each person gets their own sessions and their own connections.

One change to the auth config: a channel accepts a **list** of methods, not one mode. The common setup is "backend uses an agent key, browser users use our OIDC tokens" on the same agent. Methods are tried by credential shape (`evr_ak_` prefix, then JWT, then introspection), so there is no guessing order.

### Browsers and mobile

A key never goes to a browser. The customer's backend calls `POST /runtime-auth` with its key and an `End-User` id (or the browser presents the customer's OIDC token directly) and gets the existing 15 minute runtime JWT for that channel and that end user. The API channel accepts that token, with `cors_origins` from the channel config.

### Org members

A PAT is not accepted on the execution API by default. A channel may opt in to "org members" so the builder can test with their own login; such calls run as the member's console virtual user. This keeps the dismissed-option boundary (no PAT confinement logic) while giving builders a zero-setup try-out.

### Security checklist

- Execution routes have no handler path to management state; keys are rejected by every management extractor (the same structural property api_endpoint keys have today, TM-APIKEY-002).
- Liveness first (channel live, agent active, exposures not suspended), then auth, then rate limit, then session lookup.
- Caller confinement by tags, generic 404 across callers and channels; new tag prefixes (`api:`, `key:`) reserved before anything writes them.
- `End-User` only honoured from a key with `end_user`; the binding realm is the key, so two applications cannot collide on "customer 42", and an OAuth caller cannot assert anyone.
- `visibility` and `errors` decide what leaves the platform; anything reachable by end-user or anonymous callers is forced to `public` errors (`knowledge/execution/public-endpoints.md`).
- Budgets: the `agent_channel` subject already caps spend per door; add an optional per-caller daily cap so one leaked key or one noisy customer cannot drain the agent.
- Idempotency on `POST /runs` and `POST /sessions` (the `/v1/commands` Idempotency-Key machinery, encrypted stored responses).
- Audit: key create/rotate/revoke, auth failures (rate-limited logging), end-user bindings created.
- Threat model entries to add: leaked agent key, end-user assertion spoofing, cross-caller session probing, CORS misuse with runtime tokens.

## Several agents behind one credential (deferred)

Decided 2026-10-08: tabled for now. A v1 agent key is granted to exactly one API channel, so one key means one agent. A grouping resource with its own base URL is ruled out: it would be the retired App under a new name.

If it comes back, the cheap shape is still available without a new entity: let a key list several API channels, plus `GET /api/v1/channels` returning the channels a key may call. Keys store their grants as a list from day one so that change is not a migration.

## Alignment with serve

- **One contract, two hosts.** The execution API becomes the contract `serve` implements per agent at `/v1/channels/{agent}/…` (card, sessions, runs). serve's root `/v1/sessions` (pick agent by `agent_name`) stays as its dev/whole-app surface; it is the serve equivalent of the management session routes.
- **Shared wire types.** Move the execution API request/response types out of serve's ad hoc `json!` and the server's handler structs into `everruns-contracts` (module `execution_api`, no new crate). Server, serve and the Rust agent client all use them, so drift becomes a compile error.
- **One conformance suite.** Like the durable backend conformance suite: a set of tests that drive "an agent base URL" and run against both a server API channel and a serve app. The existing `the_everruns_sdk_drives_a_serve_app` test is the seed.
- **Auth in serve.** serve gets an auth hook on `ServerBuilder` that takes the same method list: static keys from config or env, and OIDC/JWKS. To avoid two verifiers, the token-verification half of `channel_auth.rs` (JWT/JWKS/introspection/requirements, no database) moves into `everruns-core` behind a feature; the server keeps the parts that read channel rows. No new crate.
- **Hosting targets.** AgentCore `/invocations` is a run with streaming; it maps onto `POST /runs` with `Accept: text/event-stream`. celld already forwards `/v1`, so it gets the API for free.
- **Route name drift to fix while doing this:** serve `POST /approvals/{tool_call_id}` vs server `tool-approvals`; serve-only `GET /v1/agent` vs the per-agent card.

## Agent-only SDK

Today's SDK is a management client with a session loop attached: one `Everruns` client, org-scoped, PAT only. The agent client is the opposite: one agent, one URL, the caller's credential, nothing else.

```python
from everruns_sdk import Agent

agent = Agent(
    url="https://app.everruns.com/api/v1/channels/chan_01HX...",
    api_key=os.environ["EVERRUNS_AGENT_KEY"],   # or token_provider=get_access_token
)

# Task style
run = await agent.run("Summarize ticket 4812", wait=60)
print(run.output.text)

# Structured output
run = await agent.run("Triage ticket 4812", output_schema=Triage)
triage = run.output.data

# Session style
session = await agent.sessions.create()
async for event in session.send("What changed since yesterday?"):
    if event.type == "output.message.delta":
        print(event.delta, end="")
    elif event.type == "ask_user":
        await session.answer(event.question_id, "yes")

# Acting for one of my customers
customer_agent = agent.for_end_user("customer-42")
```

```ts
const agent = new Agent({ url, apiKey: process.env.EVERRUNS_AGENT_KEY });
const { output } = await agent.run("Summarize ticket 4812", { wait: 60 });
```

```rust
let agent = everruns_sdk::Agent::new(url).api_key(key);
let run = agent.run("Summarize ticket 4812").wait(60).await?;
```

What it has that the management client does not:

- `Authorization: Bearer`, `api_key=` or `token_provider=` (a callback returning a fresh access token, for customer OAuth with refresh).
- `for_end_user(id)` sends the `End-User` header.
- Runs with wait, stream and typed output.
- The SSE reconnect rules already specified in `sdk/specs/sse-streaming.md`.
- Works unchanged against a serve app URL (`Agent(url="http://localhost:3000/v1/channels/support")`).

Packaging (decided 2026-10-08): the `Agent` client goes into the existing `everruns-sdk` / `@everruns/sdk` packages and becomes what the SDK is for. The SDK's management clients (agents, harnesses, capabilities, workspaces, memories, budgets and the rest of `Everruns`) are deprecated: they stay for one release with deprecation warnings, then are removed. Management from code goes through the CLI and `/v1/commands` (the command contract the CLI already uses), and the SDK specs (`specs/api-surface.md`, `specs/auth.md`) are rewritten around the execution API.

Framework protocols remain available for people who already speak them: CopilotKit and other AG-UI front ends use the agent's AG-UI channel, other agents use A2A. The agent card links them.

## Management side

- Channels: `POST /v1/agents/{agent_id}/channels` with `type: "api"`, publish/unpublish as today.
- Keys: `GET|POST /v1/agents/{agent_id}/keys`, `POST …/keys/{key_id}/rotate`, `DELETE …/keys/{key_id}`. Org-wide read in the existing Credentials view. Command contract entries (`everruns agents keys create`) come for free from `/v1/commands`.
- UI: Agent → Integrations → "API" channel with three panels: base URL and copyable snippets (curl, Python, TypeScript), keys, sign-in methods (keys, OAuth issuer, members). The Exposures view lists API channels like any other door.

## Phases

Each is a separate PR and useful on its own.

1. **Contract.** Write the execution API spec into `knowledge/` and add shared types in `everruns-contracts`; serve implements per-agent routes and the card over them. Conformance suite running against serve.
2. **API channel + agent keys.** New channel type, `agent_api_keys` table, management routes, Integrations UI. Session routes with SSE, questions, approvals, `visibility`, `errors`. Conformance suite also against the server.
3. **Runs.** `/runs` with wait, stream, idempotency, output schema; AgentCore `/invocations` mapped onto it.
4. **Identity.** Method list in channel auth, OIDC/introspection on API channels, `End-User` assertion, `/runtime-auth` accepted on API channels, CORS. Per-caller budget cap.
5. **Agent client** in the SDK repo (Python, TypeScript, Rust), docs page "Call your agent from code", cookbook against a local server and a serve app. Mark the management clients deprecated in the same release; remove them in the next.
6. **serve auth hook** with the verifier moved into core.
7. **Migrate `api_endpoint`** rows to `api` (existing `evr_app_` keys keep working as imported keys) and drop the frozen handler.
8. Later: several agents per key and a key directory route (deferred), separate OpenAPI document for the execution API (`/api-doc/execution.json`), optional custom domain per agent.

## Decisions

Settled by the owner on 2026-10-08:

1. **Several agents per key:** tabled. One key, one agent for now. No grouping resource; definitely not Apps again.
2. **SDK:** the agent client lives in the existing SDK packages, and the SDK's management endpoints are deprecated.
3. **OAuth:** Everruns only validates the customer's tokens; it does not issue tokens.
4. **Agent base URL:** `/api/v1/channels/{channel_id}`, not the `/v1/e/` short alias.

Still open, with the default this design uses:

- **Task style name:** `runs` (default) or `tasks`.

Smaller defaults, easy to flip: PATs rejected unless the channel opts in to members; default visibility `activity`; default binding `Requester`; AG-UI and A2A stay separate channel types rather than becoming switches on the API channel.
