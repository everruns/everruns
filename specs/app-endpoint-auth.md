# App Endpoint Authentication

## Abstract

Defines a shared authentication framework for the **inbound** endpoints that
Apps publish — initially A2A (`POST /v1/apps/{app_id}/a2a/{channel_id}`) and
AG-UI (`POST /v1/apps/{app_id}/ag-ui`), and any future App-published HTTP or
JSON-RPC channel.

Today every App channel implements its own credential check: AG-UI uses an
optional channel-local shared bearer token, A2A uses a per-channel hashed API
key, and webhook channels use a channel-local token. Each is fine in
isolation but none is strong enough for enterprise integration scenarios
(OIDC/JWT bearer from Google / Okta / Auth0 / Kubernetes service accounts,
OAuth2 opaque token introspection, HTTP Basic, mTLS), and there is no
single place to rotate, audit, or compose these schemes across channels.

This spec introduces:

1. **Org-scoped auth providers** — reusable, named credential-verification
   definitions (one OIDC issuer, one OAuth2 introspection endpoint, one
   mTLS trust bundle, etc.) that an org configures once and binds to many
   channels.
2. **Channel-level auth policies** — per-`app_channels`-row enforcement
   policy that references one or more providers plus requirements
   (audiences, scopes, claims, allowed subjects), optionally inheriting an
   app-level default.
3. **A shared verifier service** — one code path that every published App
   endpoint calls before any session work. Channel handlers stop owning
   credential parsing.

This replaces the narrow A2A-only "additional auth schemes" framing in
EVE-443: A2A still has to advertise protocol security schemes in its
Agent Card, but the verification logic is shared with AG-UI, webhooks,
and future endpoints.

## Goals

1. Let Apps require enterprise-grade auth (OIDC/JWT, OAuth2 introspection,
   HTTP Basic, mTLS, shared secret / API key) on any published channel
   without duplicating config per channel.
2. Keep the existing channel-local shared-secret / API-key fields working
   while a migration path bridges them onto the new model — no breaking
   change to deployed AG-UI, webhook, or A2A channels.
3. Provide a single verifier surface so adding a new App-published endpoint
   (Discord, web widget, generic HTTP) costs one channel adapter and zero
   credential parsers.
4. Surface effective policy correctly in protocol discovery (A2A Agent
   Card `securitySchemes` / `security`, future AG-UI capability metadata).
5. Make rotation, audit, and outage handling visible: provider state is
   read centrally, JWKS / introspection caches have explicit lifetimes,
   and verification failures are non-enumerating to external callers.

## Non-Goals

1. Platform login / SSO. `specs/authentication.md` covers user identity
   for the platform UI and the `AUTH_MODE` matrix. This spec covers
   **resource-server** authentication for App-published endpoints, which
   is independent of how the org's own operators log in.
2. Outbound user-connection OAuth (e.g. "Connect GitHub" on a user
   profile). Those grants are user-scoped, persist refresh tokens, and
   live in a separate provider registry. See **Decision: Separate
   registry from user connections** below.
3. mTLS termination on the server itself. Most production deployments
   sit behind a reverse proxy that terminates TLS. The spec defines the
   trusted-header contract for that case and leaves direct in-process
   client-cert validation as an optional second iteration.
4. AND-composition of multiple schemes (e.g. mTLS + JWT) at launch. Only
   OR composition between schemes is supported; AND can be added later
   without breaking any of the launch shape.
5. Token issuance. The platform validates inbound credentials issued
   elsewhere; it does not act as an OIDC/OAuth issuer for App callers.

## Concepts

### App Endpoint Auth Provider

An org-scoped, reusable credential-verification definition. Providers are
**not** per-channel — duplicating an Okta / Google / K8s configuration on
every channel makes rotation and audit unmanageable. Providers carry the
discovery URL / JWKS URI / introspection endpoint / CA bundle / claim
mapping rules and the cache settings.

Identity: `aeauthprov_` prefix, org-scoped, soft-deletable.

Lifecycle: `active → disabled → deleted`. Disabling a provider must cause
any channel policy that references it to deny new requests with
`provider_disabled` instead of silently falling through.

Field shape (canonical types live in `crates/core/src/app_endpoint_auth.rs`):

```rust
pub struct AppEndpointAuthProvider {
    pub id: AppEndpointAuthProviderId,    // aeauthprov_
    pub org_id: OrgId,
    pub name: String,                     // org-unique, human-readable
    pub kind: AppEndpointAuthProviderKind,
    pub config: AppEndpointAuthProviderConfig, // type-specific JSON
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub enum AppEndpointAuthProviderKind {
    OidcJwtBearer,         // OIDC discovery + JWKS-validated JWT bearer
    OAuth2Introspection,   // RFC 7662 introspection of opaque tokens
    HttpBasic,             // username + bcrypt/argon2 hashed password set
    Mtls,                  // mTLS via trusted-proxy header contract
    SharedSecret,          // legacy AG-UI / webhook token (compat)
    ApiKey,                // legacy A2A hashed API key (compat)
}
```

Provider-kind-specific `config` shapes (illustrative; full struct
definitions live in `crates/core/src/app_endpoint_auth.rs`):

- **OidcJwtBearer**: `issuer` (https URL), optional explicit `jwks_uri`
  override, `discovery_cache_ttl_secs`, `jwks_cache_ttl_secs`,
  `clock_skew_secs`, accepted signing algorithms allowlist (default
  `[RS256, ES256]`, never `none` or `HS*`).
- **OAuth2Introspection**: `introspection_endpoint` (https URL),
  `client_credentials_secret_ref` (envelope-encrypted),
  `cache_ttl_secs`, optional `bearer_token_attribute` (default
  `active`).
- **HttpBasic**: list of `{username, password_hash}` entries; password
  hashes use Argon2id and are envelope-encrypted at rest.
- **Mtls**: `trusted_header` (e.g. `X-Client-Cert-Verified`),
  `identity_header` (e.g. `X-Client-Cert-Subject`),
  `trusted_proxy_cidrs` (callers outside these source ranges are
  rejected even if the headers are present), optional
  `accepted_issuer_dn` allowlist. See **mTLS deployment** below.
- **SharedSecret**: legacy AG-UI / webhook token form. Stored as
  envelope-encrypted secret with a non-secret display prefix.
- **ApiKey**: legacy A2A form. Hashed SHA-256 with non-secret prefix —
  matches the existing `evra2a_` shape so existing channels migrate
  without rotating keys.

### App Endpoint Auth Policy

Per-channel (and optionally per-app default) enforcement policy. The
policy points at one or more providers and adds the resource-server
requirements (audiences, scopes, claims, subject allowlists) that the
provider's tokens must satisfy.

Effective policy is resolved per channel: a channel's own policy wins;
if absent, the app's default policy applies; if neither is set, the
existing channel-local auth fields continue to apply (compat path).

Field shape:

```rust
pub struct AppEndpointAuthPolicy {
    pub schemes: Vec<AppEndpointAuthSchemeBinding>, // OR-composed at launch
    pub allow_anonymous: bool,                      // default false
}

pub struct AppEndpointAuthSchemeBinding {
    pub provider_id: AppEndpointAuthProviderId,
    pub required_audiences: Vec<String>,    // empty = no audience requirement
    pub required_scopes: Vec<String>,       // AND-of-scopes within one scheme
    pub allowed_subjects: Vec<String>,      // optional subject allowlist
    pub required_claims: serde_json::Value, // JSON Pointer → value match map
}
```

Composition rules at launch:

- Multiple `schemes` in one policy: OR — any matching scheme passes.
- Within one binding: `required_audiences` is OR (the token must claim
  at least one), `required_scopes` is AND (the token must carry all),
  `allowed_subjects` is OR.
- `allow_anonymous = true` short-circuits the verifier and is reserved
  for the existing AG-UI anonymous mode and any explicit opt-in path.
  Setting `allow_anonymous = true` while `schemes` is non-empty is a
  validation error — the surface must be either gated or open, never
  both, to avoid silent bypass.

### Effective Policy Resolution

Per request, the verifier computes the effective policy:

1. If `app_channels.auth_policy_id` resolves to an active policy, use it.
2. Else if `apps.default_auth_policy_id` resolves to an active policy,
   use it.
3. Else fall back to the channel's legacy auth fields (AG-UI `token`,
   webhook `token`, A2A `api_key_hash`), wrapped in synthesized
   policies internally. This preserves zero-config behavior.

The columns are nullable foreign keys into `app_endpoint_auth_policies`
(see **Migration** below); the policy object is read by ID, never
embedded inline on the channel or app row, so policy updates do not
require rewriting every referencing row. API request/response field
names mirror the column names (`auth_policy_id` on channel patch,
`default_auth_policy_id` on app patch); the resolved policy object is
inlined under `auth_policy` in read responses for convenience.

Steps 1–2 always win over the legacy fields, so once a channel adopts
the new model the legacy fields are inert. This is intentional: there
must never be a state where a stricter policy is configured and a
weaker legacy credential still works.

### Verifier Service

The `AppEndpointAuthVerifier` exposes a single async entry point used by
every App-published endpoint:

```rust
pub trait AppEndpointAuthVerifier: Send + Sync {
    async fn verify(
        &self,
        ctx: &AppEndpointAuthContext, // app, channel, request metadata
        headers: &HeaderMap,
        client_ip: Option<IpAddr>,
    ) -> Result<AppEndpointPrincipal, AppEndpointAuthError>;
}
```

`AppEndpointPrincipal` is a structured outcome (`anonymous`, `user`,
`service_account`, `client_credentials`, `mtls_subject`, `shared_secret`,
`api_key`) carrying the subject identifier and validated scope / claim
set. Channel handlers consume it for audit, budget tagging, and template
context but never for credential parsing.

`AppEndpointAuthError` is intentionally non-enumerating to external
callers for the **auth-scheme outcome**: every credential-validation
failure collapses to `unauthorized` regardless of which check failed
(wrong issuer, wrong audience, expired token, missing scope, etc.).
`forbidden` covers `channel disabled / app unpublished` and
`provider_outage` covers the case in **Operational Behavior on
Provider Outage** below. Resource-not-found responses keep the
per-channel HTTP contract (A2A returns HTTP 404 for "App or channel
not found" per `specs/a2a-channel.md`; AG-UI returns its existing
response shape) because the verifier runs **after** the
published-app + enabled-channel gate that owns those responses.
Internal logs carry the granular reason for every failure path.

## Decision: Separate Registry from User Connections

User-connection providers (the `ConnectionProvider` registry in
`crates/core/src/connection_provider.rs`, resolved at runtime by
`DbConnectionResolver` / `UserConnectionResolver` in
`crates/server/src/storage/connection_resolver.rs`, and exposed by the
`/v1/user/connections/*` API in
`crates/server/src/api/user_connections.rs`) are **outbound** OAuth
grants tied to one user at a time and used to call external APIs on
that user's behalf. App endpoint auth providers are
**inbound** resource-server validators used to authenticate callers
reaching us. Mixing them would:

- Confuse blast radius (revoking an Okta connection should not
  invalidate the inbound trust path that the same Okta tenant uses to
  reach apps).
- Confuse audit (the connection-grant audit trail is per-user; the
  inbound-validation audit trail is per-org / per-app).
- Conflate secret shapes (user OAuth refresh tokens vs. org-scoped
  introspection client credentials).

Therefore the App endpoint auth registry is a separate table and a
separate CRUD surface. The two domains may reuse `crypto/envelope` for
secret storage, but nothing else.

## Decision: OIDC/JWT-Bearer First, Introspection Second

OIDC discovery + JWKS-validated JWT bearer is the priority initial
provider kind because:

- Google, Okta, Auth0, Microsoft Entra, GitHub Actions OIDC, and
  Kubernetes projected service-account tokens all ship JWTs validatable
  via `iss` + JWKS lookups.
- Validation has no per-request RTT once the JWKS is cached.
- The provider config is small and stable: `issuer` + cache TTLs +
  allowed algorithms.

OAuth2 token introspection (RFC 7662) is the second iteration. It is
required only when the caller's tokens are opaque (e.g. some legacy
Okta / Auth0 setups, GitHub fine-grained personal access tokens). The
spec reserves the provider kind and config shape so adding it is
non-breaking; implementation can land in a follow-up PR.

## Decision: Kubernetes via OIDC, Not TokenReview

For Kubernetes service-account tokens, the spec requires the cluster's
issuer to be configured for OIDC discovery (`--service-account-issuer`,
`--service-account-jwks-uri`) and validates projected tokens against
that issuer. The alternative — calling `TokenReview` on a configured
API server — would require platform-managed cluster credentials with
broad permission and would create a cross-trust dependency we do not
want at launch. Operators wanting TokenReview semantics can put a
TokenReview-fronting webhook in front of the platform and surface it
as a generic OAuth2 introspection provider once that kind ships.

## Decision: OR Composition Only at Launch

Composing schemes with AND (e.g. require mTLS proof AND a JWT)
introduces non-trivial identity reconciliation: whose subject ID wins
for audit, which scopes count toward budgets, what error message to
return when only one half is present. There is no current concrete
deployment requirement for AND composition, so the launch shape
supports only OR.

Forward-compatibility is preserved by versioning the stored policy: the
`app_endpoint_auth_policies` row carries a `schema_version` column
(launch value `1`, OR-only). A future version may add a
`composition_mode: Or | And` field with `Or` as the default for
existing rows; readers branch on `schema_version` so this is an
additive change and not a breaking migration. The launch struct does
not include `composition_mode` — it ships when AND is implemented, not
before.

## Migration

Existing channels keep working unchanged. Two migration layers:

1. **Legacy compat path** — channels with no explicit policy continue
   to validate against their existing fields (`AgUiChannelConfig::token`,
   webhook token, `A2aChannelConfig::api_key_hash`). The verifier
   synthesizes an internal policy from these fields so the same code
   path runs for legacy and policy-driven channels.
2. **Bridge migration** — `041_app_endpoint_auth.sql` (or next free
   number; see `specs/migrations.md`) adds:
   - `app_endpoint_auth_providers` table
   - `app_channels.auth_policy_id` nullable FK
   - `apps.default_auth_policy_id` nullable FK
   - `app_endpoint_auth_policies` table (1:1 with the FK columns
     above; policies are not reused across channels because their
     requirement set is per-resource)

Backfill: none required. Channels without a policy continue on the
legacy path until an operator attaches one.

Rollback path: dropping the FK columns and the new tables restores the
exact pre-migration behavior because legacy fields are untouched.

## A2A Behavior

The Agent Card emits `securitySchemes` and `security` derived from the
**effective policy**, not the channel's legacy `api_key_hash`. Mapping:

| Effective binding kind | Agent Card scheme entry |
|------------------------|-------------------------|
| `OidcJwtBearer`        | `{ type: "openIdConnect", openIdConnectUrl: "<issuer>/.well-known/openid-configuration" }` |
| `OAuth2Introspection`  | `{ type: "oauth2", flows: { ... } }` (only when public flow URLs are present in provider config) |
| `HttpBasic`            | `{ type: "http", scheme: "basic" }` |
| `Mtls`                 | `{ type: "mutualTLS" }` |
| `ApiKey` (legacy)      | `{ type: "http", scheme: "bearer" }` (current shape) |
| `SharedSecret`         | not advertised — channel-private |

For `OidcJwtBearer` the `openIdConnectUrl` is published only when the
issuer is itself a public URL (the common case). Private-issuer
deployments (internal K8s service-account issuers) set
`publish_discovery: false` on the provider; the card then advertises
no scheme entry, matching today's behavior where the caller must learn
the auth requirement out-of-band.

`security` is built as `[{ <scheme_id>: <scopes> }]` per binding. Multiple
bindings produce multiple top-level entries (OR per A2A spec).

Backward compatibility: A2A channels with no explicit policy keep the
exact card shape they emit today (`apiKey` scheme, bearer scheme).

## AG-UI Behavior

Both `POST /v1/apps/{app_id}/ag-ui` and
`POST /v1/apps/{app_id}/ag-ui/images` invoke the verifier at the same
point in the request lifecycle where the current `token` check runs.

- A channel with `allow_anonymous = true` and no `schemes` matches the
  current `anonymous` mode.
- A channel with the legacy `token` field set and no new policy
  continues to accept `Authorization: Bearer <token>` and
  `X-Everruns-AG-UI-Token: <token>` exactly as today.
- A channel with a new policy (e.g. `OidcJwtBearer` binding) enforces
  that policy on every request; the legacy `token` field on the same
  channel is ignored once a policy is set.
- Rate-limit, session-expiration, and tool-visibility behavior is
  unchanged — they layer on top of authentication.

## Webhook Channel Behavior

Webhook channels gain access to the same model. A webhook channel with
an explicit policy enforces it before payload parsing; webhook channels
without a policy continue to validate the channel-local `token` field.

This unblocks deployments that want to back webhook channels by GitHub
Actions OIDC, AWS IAM SigV4 (future provider kind), or any HMAC scheme
without expanding `WebhookChannelConfig`.

## API

CRUD endpoints live under `/v1/app-endpoint-auth-providers` (provider
config) and on App / channel update routes (policy binding). Org
context is resolved from the caller's session / API key via the
standard `X-Org-Id` mechanism (see `specs/authentication.md`), the
same as every other org-scoped resource on the platform — there is no
`{org_id}` path segment.

| Method | Path | Description |
|--------|------|-------------|
| POST   | `/v1/app-endpoint-auth-providers` | Create provider |
| GET    | `/v1/app-endpoint-auth-providers` | List providers in org |
| GET    | `/v1/app-endpoint-auth-providers/{id}` | Read provider (secrets are write-only) |
| PATCH  | `/v1/app-endpoint-auth-providers/{id}` | Update provider |
| DELETE | `/v1/app-endpoint-auth-providers/{id}` | Disable / delete provider |
| PATCH  | `/v1/apps/{app_id}` | `default_auth_policy` accepted on update |
| PATCH  | `/v1/apps/{app_id}/channels/{channel_id}` | `auth_policy` accepted on update |

Reads never echo provider secrets. They return:

- Provider: `id`, `name`, `kind`, non-secret `config_public`, `enabled`,
  `created_at`, `updated_at`, plus `<secret>_configured: bool` flags.
- Policy: full `schemes` shape without secrets (provider IDs only).

`platform_management` capability gains
`manage_app_endpoint_auth_providers` for the provider CRUD path so
MCP / bash command catalogs can create providers under script.

## ID Schema

| Entity | Prefix | Example |
|--------|--------|---------|
| App Endpoint Auth Provider | `aeauthprov_` | `aeauthprov_01933b5a000070008000000000000001` |
| App Endpoint Auth Policy | `aeauthpol_` | `aeauthpol_01933b5a000070008000000000000002` |

## Caching

Provider verification has three caches, all bounded and explicit:

- **OIDC discovery doc**: TTL from provider config, default 24 h.
- **JWKS**: TTL from provider config, default 1 h. Forced refresh on
  `kid` miss before returning `unauthorized`, with a per-provider
  rate limit on forced refreshes to prevent JWKS-DoS via crafted
  `kid` values.
- **Introspection responses**: TTL from provider config, default 60 s.
  Negative responses (`active=false`) cached for at most 30 s to keep
  revocations near-real-time.

Caches are in-process when running single-node; Valkey-backed when
`VALKEY_URL` is set, mirroring `ChannelRateLimiter`. The cache key
namespace is `aeauth:` so it does not collide with other caches.

## Operational Behavior on Provider Outage

If the provider's remote endpoint is unreachable past the **relevant
cache TTL × 2**, the verifier returns `provider_outage` (HTTP 503)
instead of falsely accepting or rejecting. The applicable TTL per
provider kind:

- `OidcJwtBearer`: `discovery_cache_ttl_secs` for the OIDC discovery
  document; `jwks_cache_ttl_secs` for the JWKS endpoint. Either being
  unreachable past its own `ttl × 2` window trips the outage state.
- `OAuth2Introspection`: `cache_ttl_secs` for the introspection
  endpoint.
- `HttpBasic`, `Mtls`, `SharedSecret`, `ApiKey`: no remote dependency,
  so `provider_outage` does not apply.

This fails closed — an outage cannot become an authentication bypass —
and gives operators a clear signal distinct from credential failures.

## mTLS Deployment

mTLS providers do **not** terminate TLS in the server process at launch.
Deployments terminate TLS at a trusted reverse proxy (Caddy, Envoy,
nginx, cloud LB) which:

1. Validates the client certificate against the operator's CA bundle.
2. Strips any pre-existing values of the trusted headers from inbound
   requests so the client cannot forge them.
3. Adds the configured `trusted_header` and `identity_header` to the
   request before forwarding.

The verifier checks `client_ip ∈ trusted_proxy_cidrs` **before** trusting
the headers; if the request did not come from a configured trusted
proxy, the mTLS provider rejects with `unauthorized`. This is the same
trust-boundary contract documented in `specs/production-deployment.md`
for reverse-proxy headers; the mTLS provider just consumes more of it.

Native in-process client-cert validation can be added as an optional
provider config flag in a follow-up iteration once a deployment
demonstrates the need.

## Audit Logging

The verifier emits one audit log entry per successful verification with:

- domain/action: `agent` / `agent.app_invocation.authenticated`
- target: `app_channel:{channel_id}`
- actor: the verified principal where stable (e.g. JWT `sub`); none for
  anonymous / opaque-key paths
- metadata: `provider_id`, `provider_kind`, `binding_index`,
  `subject`, `scopes` (no token contents)

Failed verifications emit a single coarse-grained audit entry with the
public error code only, never the granular reason — this keeps the
audit log itself non-enumerating for operators.

## Threat Model

A new threat category `TM-AEAUTH` will cover the inbound App-endpoint
auth surface. The entries below enumerate the design-time threat
analysis. They will be added to `specs/threat-model.md` alongside the
first implementation PR that introduces verifiable mitigations in code
(per the threat-model convention that every entry links to the
mitigating code path). The category is reserved as
**"App Endpoint Auth (TM-AEAUTH)"**:

- `TM-AEAUTH-001` — JWT signature algorithm confusion / `alg=none`.
  Mitigation: algorithm allowlist enforced before signature verify;
  `none` and HS\* are rejected by default; allowed set is per provider.
- `TM-AEAUTH-002` — JWKS poisoning via crafted `kid`. Mitigation:
  forced refreshes are rate-limited per provider; the refresh fetch
  uses the JWKS URI **discovered from the validated OIDC discovery
  document over HTTPS** (not a caller-supplied URL); the refreshed
  document is parsed strictly (only JWKs with supported `kty` / `alg`
  values retained) before replacing the cache; and a forced refresh
  that fails to surface a key matching the requesting `kid` still
  returns `unauthorized` rather than retrying unbounded. Trust in the
  JWKS itself flows from TLS to the issuer plus the discovery
  linkage, not from a separate JWKS signature.
- `TM-AEAUTH-003` — `iss` / `aud` mismatch / cross-tenant token
  reuse. Mitigation: `iss` validated against provider config; `aud`
  validated against policy `required_audiences`; tokens that don't
  bind to the channel's policy are rejected even if signed by a
  configured issuer.
- `TM-AEAUTH-004` — `exp` / `nbf` / clock skew abuse. Mitigation:
  hard `exp` check + per-provider `clock_skew_secs` (default 60 s,
  capped at 300 s).
- `TM-AEAUTH-005` — Token replay within validity window. Mitigation:
  TLS at the edge (TM-AUTH-005); optional per-app nonce / `jti`
  denylist is a follow-up (see `TM-A2A-010`).
- `TM-AEAUTH-006` — OAuth2 introspection outage masking revocation.
  Mitigation: negative-response cache TTL capped at 30 s; outage
  past `cache_ttl_secs × 2` returns `provider_outage`.
- `TM-AEAUTH-007` — mTLS header spoofing. Mitigation: trusted-proxy
  CIDR allowlist enforced before trusting the header pair; the proxy
  contract requires stripping client-supplied values.
- `TM-AEAUTH-008` — Cross-org provider reuse. Mitigation: provider
  rows are `org_id`-scoped; policies reference provider IDs the
  policy's org owns; the verifier rejects a policy whose provider
  belongs to a different org.
- `TM-AEAUTH-009` — Provider disable leaving an active policy.
  Mitigation: disabled providers cause verification failure with a
  distinct internal reason; channels referencing a disabled provider
  do not silently fall through to anonymous.
- `TM-AEAUTH-010` — Auth-bypass during app or channel publish
  transitions. Mitigation: published-app + enabled-channel gate runs
  before the verifier, matching the existing A2A and AG-UI gate
  order (`TM-A2A-004`, `TM-AUTHZ-006`).
- `TM-AEAUTH-011` — Secret exposure in provider list responses.
  Mitigation: read endpoints return `*_configured` booleans and a
  non-secret prefix only; envelope-encrypted secrets never leave
  storage in cleartext (see `specs/encryption.md`).
- `TM-AEAUTH-012` — Discovery-metadata leakage in Agent Card.
  Mitigation: provider config `publish_discovery: bool` controls
  whether the Agent Card advertises `openIdConnectUrl`; private
  issuers stay private.

Existing `TM-A2A-*` entries that were specific to API-key auth remain
in force for channels still using the legacy provider kind.

## Testing

Coverage required across provider kinds:

1. **OidcJwtBearer**: valid token success; expired / not-yet-valid /
   wrong issuer / wrong audience / wrong algorithm / missing required
   scope / missing required claim all fail with the public
   `unauthorized` response and a distinct internal reason.
2. **OAuth2Introspection** (when implemented): `active=false`,
   missing required scope, missing required audience, introspection
   endpoint outage past TTL.
3. **HttpBasic**: correct credential, wrong password, unknown user,
   case-insensitive header parsing.
4. **Mtls**: trusted-proxy CIDR allowed vs. rejected; correct subject
   vs. mismatched subject; missing identity header.
5. **Legacy SharedSecret / ApiKey**: unchanged from today's tests —
   regression guard that the synthesized policy preserves existing
   behavior for channels that have no new policy.
6. **Policy resolution**: channel policy beats app default beats
   legacy fields; switching policy types on a channel does not
   regress existing valid sessions.
7. **Provider disabled**: requests fail closed (not silently
   anonymous).
8. **Non-enumerating errors**: every **post-resolution** auth failure
   returns the same public response shape regardless of which scheme
   failed (e.g. wrong issuer vs. wrong audience vs. wrong scope all
   collapse to one `unauthorized` body). Resource-existence errors
   keep the per-channel contract documented elsewhere — A2A continues
   to return HTTP 404 for "App or channel not found"
   (`specs/a2a-channel.md`), AG-UI continues to return its existing
   response shape for unpublished apps. Non-enumeration is scoped to
   the auth-scheme outcome after the published-app + enabled-channel
   gate has already resolved the resource; this preserves the gate
   ordering documented in `TM-A2A-004` and `TM-AUTHZ-006`.
9. **A2A Agent Card**: shape changes correctly when policy switches
   between provider kinds; never echoes secrets or private issuer
   URLs.

## References

- [`specs/apps.md`](apps.md)
- [`specs/a2a-channel.md`](a2a-channel.md)
- [`specs/app-invocation-channels.md`](app-invocation-channels.md)
- [`specs/authentication.md`](authentication.md) — platform login (different surface)
- [`specs/encryption.md`](encryption.md) — envelope encryption for provider secrets
- [`specs/threat-model.md`](threat-model.md) — TM-AEAUTH category
- [`specs/migrations.md`](migrations.md) — migration numbering and conflict resolution
- A2A protocol: <https://a2aproject.github.io/A2A>
- RFC 7519 (JWT), RFC 7662 (OAuth2 Introspection), RFC 8414 (Authorization Server Metadata)
