---
type: Specification
title: "FCP (Free Communication Protocol) channel"
description: "FCP inbound channel."
tags:
  - everruns
  - integrations
---
# FCP (Free Communication Protocol) channel

## Abstract

FCP is a minimal text-first ingress channel. The endpoint accepts a
`POST` whose body is plain text (or `{"message": "..."}`) and returns the
agent's reply as `text/markdown`. A `GET` to the same URL returns a
Markdown handshake that describes what the endpoint can do and how to
authenticate.

FCP is deliberately schema-free at the wire layer. Capability discovery,
parameter collection, and error guidance happen in natural language, the
same way a person would ask a service what it does. See the upstream FCP
specification at `https://github.com/everruns/fcp/blob/main/SPEC.md`.

This spec captures the *channel*, how the protocol is exposed by an App
in this repo, the isolation invariants it must keep, and how operators
configure it.

## Goals

1. Provide an unattended, anonymous-by-default text-in / text-out
   endpoint for any published App.
2. Make every response, success or error, readable and actionable
   without parsing.
3. Keep FCP's auth, rate-limit, and error surface isolated from every
   other channel and from the platform's user-session auth.

## Non-Goals

1. Structured envelopes, JSON-RPC, schemas, code generation.
2. Streaming. FCP returns a single reply per `POST`. A future revision
   may negotiate `text/event-stream` via the handshake; today it does not.
3. Inline OIDC/HTTP-Basic/mTLS auth modes. Operators that need IdP-backed
   auth in front of FCP terminate it at the edge (reverse proxy, IAP,
   mTLS at the LB).

## Endpoints

```
GET  /v1/apps/{app_id}/fcp
POST /v1/apps/{app_id}/fcp
```

Both routes always respond with `Content-Type: text/markdown; charset=utf-8`.

- `GET` returns the configured handshake (or a generated one).
- `POST` runs one turn through the app's agent and returns the assistant's
  final Markdown reply.

## Configuration

`channel_config` for an `fcp` channel deserializes to
`everruns_core::FcpChannelConfig`:

| Field                       | Default      | Notes                                                                                       |
| --------------------------- | ------------ | ------------------------------------------------------------------------------------------- |
| `anonymous`                 | `true`       | When `false`, a non-empty `token` must authenticate every `POST`.                           |
| `token`                     | none         | Shared bearer secret; validated by constant-time comparison.                                |
| `handshake`                 | generated    | Optional Markdown override for the `GET` body.                                              |
| `session_expiration_seconds`| `21600` (6h) | Cookie lifetime; `0` disables expiration.                                                   |
| `rate_limit_per_minute`     | none         | Per-app, per-IP cap counted in the FCP-specific limiter namespace.                          |
| `response_timeout_seconds`  | `120`        | Maximum seconds the endpoint waits for the agent's reply before returning `504`.            |

`auth` (the inline IdP/Basic/mTLS verifier used by AG-UI and A2A) is
**deliberately not accepted** on FCP channels, see the isolation
invariants below.

## Isolation invariants

The user-facing requirement was clear: FCP must not share infrastructure
with anything else.

1. **Auth stack is FCP-only.** Token verification lives inside
   `crates/server/src/api/fcp.rs::check_token` and never delegates to
   `AppEndpointAuthVerifier`. Adding new auth modes is intentionally a
   breaking design decision, not a config flag.
2. **Rate limiter is FCP-only.** `app_builder` constructs a dedicated
   `ChannelRateLimiter` with namespace `"fcp"`. Buckets cannot collide
   with `"agui"`/`"a2a"` or with the global API limiter.
3. **No platform-user auth.** FCP requests never carry an Everruns user
   session, API token, or cookie. The platform's auth middleware is not
   on the FCP route path.
4. **No internal-state leaks.** Every error path collapses through a
   small set of sanitized responses (`not_found_response`,
   `unauthorized_response`, `turn_error_response`, etc.). A caller cannot
   distinguish "no such app" from "app exists but unpublished" from
   "channel disabled". `turn.failed` causes pass through `PublicError` so
   provider details (OpenAI/Anthropic vocabulary, stack traces, internal
   codes) never reach the wire.

> The same invariants are good architecture for AG-UI as well. This is
> noted but is not part of this spec, refactoring AG-UI to match should
> ship as its own change. New channels should follow the FCP shape, not
> the AG-UI shape.

## Actionable error bodies

Every non-2xx response is `text/markdown` with at least:

- a one-line statement of what happened, and
- a concrete next step the caller can take without contacting support.

Examples:

- `401 Unauthorized` body includes `Authorization: Bearer <token>` and a
  pointer to the `GET` handshake.
- `429 Too Many Requests` body includes the configured limit and tells
  the caller to wait roughly 60 seconds. The header `Retry-After: 60` is
  set alongside.
- `504 Gateway Timeout` body names the configured response-timeout
  budget so the operator can tune it.
- `410 Gone` body tells the caller to drop the `fcp_session` cookie.

The handshake itself is the single source of truth for "how do I use
this endpoint": all error bodies point back at it rather than restating
its contents.

## Request shape

`POST` body handling (`extract_user_text`):

- `Content-Type: application/json` → parse as `{"message": "..."}`. Any
  other JSON shape is `400 Bad Request`.
- Otherwise → if the body parses as JSON-with-a-`message`-field, take
  that. Else treat the body as raw UTF-8 text. Non-UTF-8 is `400`.
- Bodies above 256 KiB are rejected with `413` before any agent work
  runs.

## Session reuse

FCP sessions are reused via the `fcp_session` cookie:

- First `POST` creates a session tagged `fcp:app:{app_id}`. The response
  sets `fcp_session=<uuid>; Path=/; HttpOnly; SameSite=Lax`, plus
  `Max-Age` when `session_expiration_seconds > 0`.
- Subsequent `POST`s with the cookie resume that session if it still
  matches the tag and has not expired.
- A stale or unknown cookie is silently ignored, the next `POST`
  creates a fresh session. This avoids stranding clients on the wrong
  side of an operator config change.
- Expired sessions return `410 Gone` with body instructing the client to
  drop the cookie.

All sessions adopt the App's owner principal (`session.owner_principal_id
= app.owner_principal_id`), matching the invariants in
`knowledge/integrations/app-invocation-channels.md` so reuse, budgets, and audit logs
remain consistent across all app channels.

## Migration

`043_app_channel_type_fcp.sql` extends the `channel_type` CHECK
constraints on `apps` and `app_channels` to include `'fcp'`. No data
migration is needed, existing rows are unaffected.

## Testing

Coverage in `crates/server/src/api/fcp.rs::tests` includes:

1. Handshake renderer: name + description, token-advertised, anonymous,
   expiration-disabled, rate-limit-advertised, custom override.
2. Cookie parsing and emission with and without `Max-Age`.
3. Body extraction: plain text, JSON via `application/json`, JSON via
   unknown content type, invalid JSON rejection.
4. Token check: anonymous accept, required reject-on-missing,
   required reject-on-mismatch, required accept-on-bearer, misconfigured
   `anonymous: false` without token.
5. Error-body sanitization: 404 leaks no operator state; turn errors
   leak no provider/internal vocabulary; 401 names the right headers and
   points at the handshake; 429 includes the configured limit.

Core coverage in `crates/platform/src/app.rs::tests` ensures `ChannelType`
serde/display round-trips include `fcp` and that `App::fcp_channel()` /
`AppChannel::fcp_config()` work end-to-end.

## Related

- Upstream FCP specification: <https://github.com/everruns/fcp>
- `knowledge/integrations/app-invocation-channels.md`, common app-channel invariants
  (session ownership, internal tag prefixes).
- `knowledge/execution/public-endpoints.md`, error-sanitization contract used by all
  unauthenticated app channels.
- `knowledge/security/threat-model.md`, TM-AUTHZ-005, TM-AUTHZ-006, TM-DOS-010,
  TM-LLM-020 mitigations apply to FCP via the same shape AG-UI uses.
