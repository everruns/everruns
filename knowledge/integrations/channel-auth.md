---
type: Specification
title: "Channel Authentication"
description: "Shared inbound authentication framework for agent channels."
tags:
  - everruns
  - integrations
---
# Channel Authentication

## Abstract

Agent channels such as AG-UI and A2A need a shared authentication model so
enterprise schemes can be added once instead of reimplemented per transport.
Authentication configuration is channel-local. There is no required
organization-level provider setup. Agent-owned channel APIs and editors
manage the configuration; App management and App editor routes are retired.

## Goals

1. Support OAuth2/OIDC-style enterprise auth for agent channels,
   especially Google OIDC, generic OIDC/JWT bearer, OAuth2 introspection, HTTP
   Basic, and reverse-proxy mTLS identity headers.
2. Preserve existing channel behavior when `auth` is absent:
   AG-UI keeps `anonymous` plus optional `token`; A2A keeps the generated
   hashed API key.
3. Keep one verifier path for all supported channel auth modes.
4. Advertise the effective A2A security scheme in the Agent Card.

## Non-Goals

1. Platform login or operator SSO. See `knowledge/security/authentication.md`.
2. Outbound user-connection OAuth. Those grants are user-scoped and separate
   from inbound resource-server validation.
3. Server-process mTLS termination. The launch model trusts a configured
   reverse-proxy identity header after the edge strips caller-supplied values.
4. Webhook replacement auth. Webhook channels continue to use the existing
   channel-local token until their handler is wired into this verifier.
5. Token issuance. Everruns validates credentials issued by external identity
   providers.

## Channel Auth Model

Supported channels store auth in `agent_channels.auth`. Deployments with
encryption configured store the JSON
only in `agent_channels.auth_encrypted`; this includes Argon2id password
hashes, OAuth client secrets, and mTLS proxy secrets.

Legacy nested `channel_config.auth` remains readable. The migration moves
plaintext rows immediately. It leaves encrypted transport rows intact because
SQL cannot decrypt them; the next channel write decrypts and separates their
auth. First-class auth is authoritative when both forms exist.

```json
{
  "mode": "oidc",
  "provider": {
    "type": "oidc",
    "issuer": "https://auth.example.com",
    "jwks_url": "https://auth.example.com/.well-known/jwks.json"
  },
  "requirements": {
    "audiences": ["api://support-agent"],
    "scopes": ["invoke:agent"],
    "domains": ["example.com"],
    "groups": ["support"]
  }
}
```

Modes:

- `anonymous` bypasses credential checks for an explicitly public channel.
- `shared_secret` validates a bearer token against a configured shared secret.
  AG-UI's legacy `token` path still also accepts `X-Everruns-AG-UI-Token` when
  `auth` is absent.
- `api_key` preserves A2A's legacy generated API key behavior.
- `google_oidc` validates Google ID tokens with the configured client ID as
  audience and optional hosted-domain checks.
- `oidc` validates bearer JWTs through OIDC discovery and JWKS.
- `oauth2_introspection` validates opaque bearer tokens through an RFC 7662
  introspection endpoint, then applies requirements.
- `http_basic` validates standard HTTP Basic credentials. Passwords are
  write-only and stored as encrypted Argon2id hashes.
- `mtls` validates a configured identity header set by a trusted reverse proxy
  after client certificate verification. Requires `proxy_secret_header` and
  `proxy_secret`, a shared secret the trusted proxy injects to prove the
  request passed through it. Configs without a proxy secret fail closed
  (misconfigured). The proxy secret is write-only and redacted in GET
  responses.

`requirements` can constrain audiences, scopes, subjects, groups, domains, and
exact claim values. Empty requirement lists mean no constraint for that field.
Audience requirements match any token `aud` value; scope requirements require
all configured scopes.

## Channel Addresses

Each agent channel has a stable channel ID. Canonical ingress routes use
`/v1/e/{channel_id}`:

- Slack: `POST /slack/events` and `GET /slack/manifest`.
- AG-UI: `POST /ag-ui` and `POST /ag-ui/images`.
- A2A: `POST /a2a` and `GET /a2a/.well-known/agent-card.json`.
- FCP: `GET /fcp` and `POST /fcp`.
- Webhook: `POST /webhook`.
- API endpoint: `POST /sessions`, plus its session read, message, and cancel
  subroutes.
- Public Chat: `GET /public-chat/config` and `POST /public-chat`.

Existing `/v1/apps/{app_id}/...` routes are permanent aliases. They resolve
from channel-owned `legacy_app_public_id` and never read `apps` or
`app_channels`. Channel-less aliases resolve the only live channel of the
requested type. They return `409 Conflict` when multiple live channels match,
with a detail that directs the caller to the channel-scoped URL.

## Enforcement

Supported handlers resolve channel identity and liveness first, then run the
auth verifier before rate limiting, session lookup, task polling, cancellation,
image upload, or message dispatch.

AG-UI behavior:

- Without `auth`, current behavior remains: `anonymous` controls public access
  and optional `token` accepts `Authorization: Bearer <token>` or
  `X-Everruns-AG-UI-Token`.
- With first-class `auth`, the shared verifier is authoritative and the legacy token gate
  is ignored.
- Stream and image-upload routes use the same auth decision on channel-scoped
  and app-scoped addresses.

A2A behavior:

- Without `auth`, current behavior remains: `Authorization: Bearer <api_key>`
  is checked against the stored SHA-256 hash.
- With `auth.mode = "api_key"`, the same legacy API-key check applies.
- Other modes use the shared verifier.
- The Agent Card derives `securitySchemes` and `security` from the effective
  auth mode: bearer API key, HTTP Basic, OpenID Connect, OAuth2, mTLS, or no
  security for anonymous.

Webhook behavior:

- Webhook channels continue to require their existing `token`.
- The API rejects channel auth on webhook channels until the webhook
  handler is wired into the shared verifier. This avoids storing a policy that
  callers might assume is enforced.

## Provider Safety

- OIDC and JWKS URLs must be safe outbound HTTPS URLs and cannot target local
  or private network addresses.
- JWT verification rejects unsigned and symmetric `HS*` algorithms for OIDC
  providers.
- OIDC discovery and JWKS documents are cached with bounded in-process caches.
- OAuth2 introspection failures fail closed; `active=false` is unauthorized.
- Public auth errors are deliberately generic so credential probing cannot
  distinguish wrong issuer, wrong audience, expired token, missing scope, or
  provider misconfiguration.

## UI
Agent Integrations presents channel inventory and Agent-owned create and
configure flows for supported channel types. The inventory also exposes
publish, unpublish, delete, and migrated schedule run-now controls. App editor
routes are retired.

## Threat Model

See `knowledge/security/threat-model.md` entries:

- `TM-AUTH-020`, public Agent channel auth bypass.
- `TM-AUTH-021`, mTLS identity header spoofing.
- `TM-AUTH-022`, JWKS / OIDC discovery abuse or poisoning.
- `TM-A2A-014`, Agent Card advertises the wrong or stale auth scheme.

## Testing

Required coverage:

1. Shared verifier unit tests for bearer shared secret, HTTP Basic parsing,
   and claim requirement checks.
2. Negative verifier paths for missing credentials and requirement mismatch.
3. A2A regression tests for legacy API key behavior and Agent Card scheme
   generation.
4. AG-UI regression tests that stream and image upload use the same auth gate.
5. Route-parity tests for canonical channel addresses and permanent aliases.

## References

- `crates/server/src/api/channel_auth.rs`, the shared verifier
- [`knowledge/integrations/agent-exposure.md`](agent-exposure.md), why channels are agent-owned
- [`knowledge/integrations/apps.md`](apps.md)
- [`knowledge/integrations/a2a-channel.md`](a2a-channel.md)
- [`knowledge/integrations/app-invocation-channels.md`](app-invocation-channels.md)
- [`knowledge/security/authentication.md`](../security/authentication.md)
- [`knowledge/security/threat-model.md`](../security/threat-model.md)
- RFC 7519 (JWT), RFC 7662 (OAuth2 Introspection), RFC 8414 (Authorization
  Server Metadata)
