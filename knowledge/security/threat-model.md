---
type: Specification
title: "Threat Model"
description: "Security threat model."
tags:
  - everruns
  - security
---
# Threat Model

## Abstract

This document defines the security threat model for Everruns, a durable agentic harness engine. It enumerates threats by category with stable IDs, documents mitigations, and identifies accepted risks and caller responsibilities.

## Threat ID Scheme

Format: `TM-<CATEGORY>-<NNN>`

| Prefix | Category | Description |
|--------|----------|-------------|
| TM-AUTH | Authentication | Credential theft, session hijack, brute force |
| TM-CRYPTO | Cryptography | Key compromise, encryption weakness |
| TM-TENANT | Tenant Isolation | Cross-org data access, enumeration |
| TM-AUTHZ | Permissions / Authorization | Policy bypass, privilege escalation, missing enforcement |
| TM-API | API Security | Injection, input validation, SSRF |
| TM-FS | Filesystem | Path traversal, data leakage in session files |
| TM-SQL | SQL Database | SQLite sandbox escape, resource exhaustion |
| TM-TOOL | Tool Execution | Unvalidated tools, MCP poisoning |
| TM-LLM | LLM Integration | API key exposure, prompt injection |
| TM-DURABLE | Durable Engine | Task hijack, gRPC security, queue abuse |
| TM-SCHED | Scheduled Tasks | Schedule injection, catch-up explosion |
| TM-OBS | Observability | Data leakage via traces/logs |
| TM-WEB | Web Security | XSS, CSRF, CORS misconfiguration |
| TM-AGENT | AI Agent | Prompt injection, jailbreak, capability abuse, cost runaway |
| TM-VOICE | Voice Sessions | Microphone capture, Realtime client secrets, sideband tool control |
| TM-BASH | Bash Sandbox | Bashkit sandbox escape, resource exhaustion, VFS boundary |
| TM-DOS | Denial of Service | Resource exhaustion, large payloads |
| TM-CLIENT | Client-Side Tools | Tool ID spoofing, timeout abuse |
| TM-MCP | MCP Server | First-party `/mcp` endpoint, MCP OAuth, external MCP clients, MCP server tool discovery/execution |
| TM-SLACK | Slack Integration | Webhook forgery, signing secret leak, bot loops |
| TM-A2A | A2A Channel | API key forgery, replay, method abuse, card disclosure |

### Managing Threat IDs

1. Assign ID using next available number in category
2. Never reuse deprecated IDs
3. Add code comment referencing threat ID at mitigation point
4. Create test for new threats where feasible

### Code Comment Format

```rust
// THREAT[TM-XXX-NNN]: Brief description of the threat being mitigated
// Mitigation: What this code does to prevent the attack
```

## Trust Model

```
                    ┌────────────────────────────┐
                    │       External Users        │
                    │  (Browser, API clients)      │
                    └─────────────┬──────────────┘
                                  │ HTTPS
                    ┌─────────────▼──────────────┐
                    │    Reverse Proxy / TLS      │ ← Trust boundary 1
                    └─────────────┬──────────────┘
                                  │
                    ┌─────────────▼──────────────┐
                    │      Control Plane          │
                    │  (API server, auth, DB)      │ ← Trust boundary 2
                    └──────┬────────────┬────────┘
                           │ gRPC       │ SQL
              ┌────────────▼───┐   ┌────▼─────────┐
              │    Workers     │   │  PostgreSQL   │
              │  (stateless)   │   │              │
              └────┬───────┬───┘   └──────────────┘
                   │       │
         ┌─────────▼──┐ ┌──▼──────────────┐
         │ LLM Call   │ │ Agent Loop       │ ← Trust boundary 4
         │ (HTTPS)    │ │ (LLM → Tools)    │
         └─────┬──────┘ └──┬──────────────┘
               │           │ Tool execution
    ┌──────────▼──┐  ┌─────▼────────────────────────┐
    │ LLM Provider│  │ Sandboxed Tools               │
    │ MCP Servers │  │ (Bash, SQL, FS, WebFetch)     │
    └─────────────┘  └──────────────────────────────┘
                          ← Trust boundary 3
```

**Trust boundary 1, User → API:** All user input is untrusted. Authentication, authorization, input validation applied here.

**Trust boundary 2, Control Plane → Workers:** Workers are stateless executors with no database credentials. Communication via gRPC with bearer token auth (required) and optional mutual TLS (mTLS). Workers are intentionally cross-org.

**Trust boundary 3, Workers → External Services:** LLM providers and MCP servers are external. API keys transmitted over HTTPS. MCP responses parsed defensively.

**Trust boundary 4, LLM → Agent Tools:** The LLM decides which tools to call and with what arguments. The agent loop executes LLM-chosen tool calls within sandboxed capabilities. The LLM is semi-trusted: it operates within registered tools and iteration limits, but its outputs (tool arguments, text) are not validated for intent.

## 1. Authentication (TM-AUTH)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-AUTH-001 | Brute force login | High | Per-IP rate limiting on auth endpoints (login 10/min, register 5/min, refresh 30/min; OAuth redirect/callback share the login tier) plus a per-account throttle keyed on the submitted email across all IPs (20 attempts / 15 min, caps distributed credential stuffing); dual backend: in-memory governor or Valkey distributed sliding-window | MITIGATED |
| TM-AUTH-002 | JWT secret compromise | Critical | Stored in env var `AUTH_JWT_SECRET`; min 32 bytes recommended; never logged | MITIGATED |
| TM-AUTH-003 | Token replay after logout | Medium | Refresh tokens stored in DB, revocable via DELETE; `POST /v1/auth/logout` deletes the refresh-token row server-side (best-effort) in addition to clearing cookies; access tokens short-lived (15 min) | MITIGATED |
| TM-AUTH-004 | Weak password | Medium | Minimum 8 characters enforced **server-side** in `register` (`crates/server/src/auth/routes.rs`, before account lookup or creation), independent of the UI's `minLength={8}`. Newly set passwords require 12+ characters including a number (register/reset, `validate_new_password`); maximum 128 characters, and login rejects oversized inputs pre-hash (bounds Argon2 work, oversized-password DoS). Argon2id hashing on storage. Covered by `test_register_rejects_short_password_via_api`. | MITIGATED |
| TM-AUTH-005 | Personal access token exposure in transit | High | HTTPS required in production; tokens prefixed `evr_pat_` for scanning | MITIGATED |
| TM-AUTH-006 | Personal access token brute force | Medium | Tokens stored as SHA-256 hashes; 128-bit entropy makes brute force infeasible | MITIGATED |
| TM-AUTH-007 | OAuth state fixation | High | State generated in `oauth_redirect`, stored in HttpOnly/Secure/SameSite=Lax cookie (`oauth_state`), validated and consumed (single-use) in `oauth_callback`; mismatch or missing cookie returns 401 | MITIGATED |
| TM-AUTH-008 | Session fixation via cookie | Medium | New tokens issued on login; HTTP-only, SameSite=Lax cookies | MITIGATED |
| TM-AUTH-009 | Refresh token theft | High | Stored hashed in DB; HTTP-only cookie; revocable | MITIGATED |
| TM-AUTH-010 | Admin password in env var | Low | Limited to admin mode; documented risk; shell history exposure possible | **ACCEPTED** |
| TM-AUTH-011 | Auth bypass in `none` mode | Info | By design for local development; anonymous user gets admin role. Fail-closed: `none` is gated to dev deployments and unknown `AUTH_MODE` values are rejected at startup (`AuthConfig::from_env`), so production cannot silently come up unauthenticated by omission or typo (EVE-621). | **BY DESIGN** |
| TM-AUTH-012 | OAuth account linking collision / pre-hijacking | High | `oauth_callback` only auto-links a provider identity to an existing same-email account after TM-AUTH-017 has accepted the provider identity and the existing Everruns account already has `email_verified=true`. Unverified local/password shadow accounts are refused (never silently merged), preventing a pre-registration attacker from retaining password access after the mailbox owner completes OAuth. Accounts already bound to a different OAuth provider are also refused because provider-reported verification does not prove current ownership of stale or reassigned email claims across providers. The refusal returns `409` mapped to the `oauth_account_exists` login category ("sign in with your original method") rather than the transient "try again" copy. Successful same-provider or local-account auto-linking also revokes the user's existing refresh tokens so sessions minted before the identity-link event do not survive the trust-boundary change. OAuth identities are stored as additive `(provider, provider subject)` bindings: a provider subject is globally unique, and an account can bind only one subject per provider. | MITIGATED |
| TM-AUTH-013 | Expired personal access token still in use | Medium | Expiration checked on every request via DB lookup; `last_used_at` tracked | MITIGATED |
| TM-AUTH-014 | Account enumeration via registration | Medium | Returns generic "Registration failed" for existing emails; password hash computed first for timing consistency | MITIGATED |
| TM-AUTH-015 | JWT secret insecure default | High | Startup **panics** if `AUTH_JWT_SECRET` is unset in `admin`/`full`/`external` mode; `none` mode generates a random per-process secret. The hardcoded `insecure-dev-secret-change-me` fallback was removed (`crates/server/src/auth/config.rs`, asserted gone by the JWT-secret tests). | MITIGATED |
| TM-AUTH-016 | OSS harness reseeding via public signup | High | The signup safety net in `register` / `oauth_callback` uses the operator-composed `state.built_in_harnesses` set (threaded from `ServerAppBuilder::built_in_harnesses`, EVE-881) via `initialize_org_harnesses_with_definitions`, **not** `oss_built_in_harnesses()`. The operator's custom composition is the source of truth, public signup cannot reintroduce OSS harnesses that were removed. Original concern tracked by PR #1462; the safety-net semantics re-added in EVE-390 preserve pre-seed correctness without re-opening the override path. | MITIGATED |
| TM-AUTH-017 | OAuth identity bypass (Google, GitHub) | High | After `exchange_code` and before user lookup or creation, `oauth_callback` calls `oauth_identity_rejection_reason` (`crates/server/src/auth/routes.rs`). Google: rejects `email_verified=false` and, when `AUTH_GOOGLE_ALLOWED_DOMAINS` is set, rejects email domains not in the list (case-insensitive). GitHub: rejects `email_verified=false`, where the flag is derived from the real `/user/emails` `verified` bit rather than hardcoded, a public profile email inherits verification only when it appears verified in that list, else the account's primary address is used (`select_github_email` in `crates/server/src/auth/oauth.rs`; EVE-702). This closes the prior GitHub gap that let an unverified GitHub address pre-empt an account (TM-AUTH-012). Applied to both first-time and returning OAuth users. Failure path emits `auth.oauth.failure` audit with a reason and returns `403`. | MITIGATED |
| TM-AUTH-018 | Refresh-token rotation race | Medium | The previous `refresh_token` handler (`crates/server/src/auth/routes.rs`) read the refresh-token row via `get_refresh_token_by_hash` and then issued a separate `delete_refresh_token`, allowing two concurrent refreshes with the same token to both pass the read before either delete committed. The MCP OAuth refresh handler had the same get-then-delete shape for `oauth_refresh_tokens`. Both paths now use atomic consume helpers: PostgreSQL `DELETE … WHERE token_hash = $1 AND expires_at > NOW() RETURNING …` and in-memory equivalents under a single write lock. Single-use rotation is restored even under concurrency. Covered by `test_refresh_concurrent_requests_only_one_succeeds`, `test_oauth_refresh_token_rotates_and_rejects_reuse`, and `test_oauth_refresh_token_concurrent_retries_are_single_use`. | MITIGATED |
| TM-AUTH-019 | Account enumeration via login error differences | Medium | Backend `login` (`crates/server/src/auth/routes.rs`) previously returned `"Password login not available for this account"` when an OAuth-only user attempted password login, distinguishable from the unknown-email and bad-password paths. Now all credential failure branches return the same generic `Invalid email or password`. UI `apps/ui/src/app/(auth)/login/page.tsx` no longer renders raw server messages on a 401, it shows a fixed `Invalid email or password.` so a future regression cannot leak the difference through the UI. Covered by `test_login_oauth_only_account_returns_generic_error`. | MITIGATED |
| TM-AUTH-020 | Public App endpoint auth bypass | High | AG-UI and A2A can now carry inline `channel_config.auth` with Google/OIDC JWT bearer, OAuth2 introspection, HTTP Basic, or trusted-header mTLS policy. Both ingress handlers resolve the published app + enabled channel first, then call the shared `AppEndpointAuthVerifier` before session lookup, image upload, task polling, cancellation, or message dispatch. Missing/invalid credentials return generic 401/403-style failures and do not expose provider details. Legacy AG-UI token and A2A API-key behavior remains the default only when `auth` is absent. | MITIGATED |
| TM-AUTH-021 | mTLS identity header spoofing | High | `verify_mtls` requires BOTH a configured identity header (set by the trusted reverse proxy after client-cert verification) AND a `proxy_secret`/`proxy_secret_header` shared secret that proves the request came through the trusted TLS terminator. Header-only configs (no `proxy_secret`) fail closed with `Misconfigured`. The proxy secret is stored write-only and redacted in GET responses. (EVE-545) | MITIGATED |
| TM-AUTH-022 | JWKS / OIDC discovery abuse or poisoning | High | Inline OIDC auth validates issuer/JWKS/introspection URLs with `validate_safe_url`, rejects symmetric JWT algorithms for OIDC, requires issuer + audience + exp claims, validates `nbf`, and caches discovery/JWKS for a bounded 15 minutes. Provider fetch failures fail closed before session creation. | MITIGATED |
| TM-AUTH-023 | Org invite token brute force or exposure | High | Invite tokens carry 256 bits of entropy (`evrinv_` + 32 random bytes) and are stored only as SHA-256 hashes (`org_invitations.token_hash`); the raw token is returned once as `invite_url` and never persisted or logged. Tokens expire (`expires_at`, default 7 days). Acceptance (`POST /v1/invites/{token}/accept`) requires an authenticated principal, re-loads the accepting user row, requires `email_verified=true`, and rejects expired/revoked/already-accepted/unknown tokens with distinct codes that reveal no token internals. Tests in `api::org_invitations`. (EVE-602) | MITIGATED |
| TM-AUTH-024 | Password reset token abuse (brute force, replay, exposure, or account enumeration) | High | Reset tokens carry 256 bits of entropy and are stored only as SHA-256 hashes (`password_reset_tokens.token_hash`, migration 089), reusing the invite-token hashing helper; the raw token is surfaced once in the emailed `{frontend_url}/reset-password?token=…` link and never persisted or logged. Single-use and short-lived: claimed via one atomic `UPDATE … SET used_at = NOW() WHERE used_at IS NULL AND expires_at > NOW() RETURNING user_id` (1h TTL), so concurrent claims cannot both succeed and a used/expired token returns a generic 400. `POST /v1/auth/forgot-password` is enumeration-safe: it always returns a timing-normalized generic 200, dispatches token creation and best-effort email delivery off the request path, and issues reset links for existing accounts including OAuth-created accounts so mailbox proof can establish a local password. `POST /v1/auth/reset-password` enforces the password policy before consuming the token, rehashes with Argon2id, and revokes all of the user's refresh tokens so a completed reset invalidates any pre-existing (possibly attacker) sessions. Tests in `auth::routes`. | MITIGATED |
| TM-AUTH-025 | Email verification token abuse or verification bypass | High | Verification tokens use the same hashed, single-use, atomic-claim model as TM-AUTH-024 (`email_verification_tokens`, migration 089; 24h TTL); the raw token is surfaced once in the emailed `{frontend_url}/verify-email?token=…` link and never stored. `POST /v1/auth/verify-email` only sets `email_verified=true` on a valid unused unexpired token and returns a generic 400 otherwise, so verification cannot be forced without possession of the emailed token. `POST /v1/auth/resend-verification` always returns a timing-normalized generic 200 and dispatches token creation plus best-effort delivery off the request path, issuing only for an existing unverified local account. Verification email is auto-sent best-effort on register. Tests in `auth::routes`. | MITIGATED |
| TM-AUTH-026 | Email bombing via forgot-password / resend-verification | Medium | Per-address send budget in `AuthRateLimiter` (1/minute plus a small daily cap, keyed on the lowercased target email) shared by `forgot-password` and `resend-verification`, on top of the per-IP register limiter. Over budget the endpoints return the normal enumeration-safe `200 {"ok":true}` without creating a token or sending, so the throttle is not an account-existence oracle. Dual backend (governor / Valkey), fail-closed. | MITIGATED |
| TM-AUTH-027 | Bot-driven signup / recovery abuse | Medium | Optional Cloudflare Turnstile on register, forgot-password, and resend-verification: enabled when `AUTH_TURNSTILE_SITE_KEY`+`AUTH_TURNSTILE_SECRET_KEY` are set (fail-fast if only one is). `/v1/auth/config` advertises the site key; requests carry `captcha_token`, verified server-side via the shared `TurnstileVerifier` (fail-closed: rejected → generic 403, siteverify outage → retryable 500-class). No-op when unconfigured (self-host default). | MITIGATED |
| TM-AUTH-028 | Recovery-path dead-ends (availability + enumeration pressure) | Low | An account that cannot self-recover (locked out with no working affordance) both degrades availability and pushes users toward out-of-band support channels, a social-engineering surface, and a standing temptation to weaken anti-enumeration ("just tell them which method they used"). The auth flow's reachability contract is modelled and enforced in code (`apps/ui/src/lib/auth-flow/machine.ts` + `machine.test.ts`, three layers: UI / backend / external): every `(goal, account-state)` situation must reach its goal via an affordance that works for that account, and no surfaced remediation may silently no-op. Closed dead-ends: OAuth-only account on the password/reset path (login copy now names the OAuth alternative, generic, shown to everyone), signed-in unverified user with no verify path (persistent in-app verify banner + resend), permanent OAuth link-refusal shown as transient (now `409 → oauth_account_exists` with "use your original method"), `/verify-email` dead link (self-serve resend), and `/signup` rendering a password form under `password_auth_enabled=false`. **Every fix holds the enumeration-safe line** (TM-AUTH-014/019): each is generic copy shown to everyone, an action on the user's own authenticated account, or addressed to a caller who already proved mailbox ownership via OAuth. | MITIGATED |
| TM-AUTH-029 | Email case-sensitivity breaks the one-account-per-mailbox invariant | Medium | User email is the account identity key, but was previously stored verbatim and guarded by a case-sensitive unique index, so `John@x.com` and `john@x.com` were two accounts for one mailbox, both able to reset to the same inbox, and OAuth linking (TM-AUTH-012) matched by exact case, so a casing mismatch between the provider email and the stored email silently created a duplicate instead of linking. Email is now canonicalized (trim + lowercase) at the storage trust boundary, both backends' `create_user*` and `get_user_by_email`, so register, login, forgot/resend, verify, `oauth_callback` linking, and admin bootstrap all share one identity, backed by a case-insensitive unique index on `users(lower(email))` (migration `099_normalize_user_email_case_insensitive.sql`). The data migration fails loudly on any pre-existing case-duplicate rows rather than silently merging (ambiguous winner). Tests in `storage::memory::tests`. (EVE-704) | MITIGATED |

### Mitigation Details

**TM-AUTH-001, Rate Limiting (MITIGATED):**
Per-IP rate limiting implemented on all auth endpoints via `AuthRateLimiter` (`crates/server/src/auth/rate_limit.rs`):
- Login: 10 requests/min per IP
- Register: 5 requests/min per IP
- Refresh: 30 requests/min per IP
- **Dual backend**: In-memory (governor crate, per-instance) when `VALKEY_URL` not set; Valkey distributed sliding-window counter when set. Fail-open on Valkey errors (availability > strictness).
- **Client IP (anti-spoofing)**: The rate-limit key is the real client IP, derived by peeling trusted reverse-proxy hops off the **right** of `X-Forwarded-For`, not the leftmost (client-supplied, spoofable) entry. Forwarding headers are honored only when the immediate peer is a trusted proxy, and only the entry `TRUSTED_PROXY_HOPS`-from-the-right is used (default 1, a single reverse proxy, the documented topology). Set `TRUSTED_PROXY_HOPS` to the number of trusted proxies in front when stacking a CDN/LB. This closes the bypass where a client rotated a forged leftmost `X-Forwarded-For` to mint unlimited distinct rate-limit keys (EVE-700).
- **Residual risk**: Without Valkey, rate limits are per-instance. With N instances behind a load balancer, an attacker gets N× the budget. Set `VALKEY_URL` for coordinated limits in multi-instance deployments. Forensic audit logging shares this same trusted-proxy extraction contract (TM-OBS-009).

**TM-AUTH-002, JWT Secret:**
```
AUTH_JWT_SECRET=<secure-random-32+-bytes>
AUTH_JWT_ACCESS_TOKEN_LIFETIME=900      # 15 min
AUTH_JWT_REFRESH_TOKEN_LIFETIME=2592000 # 30 days
```
JWT signed with HMAC-SHA256 via `jsonwebtoken` crate. Secret must never appear in logs, error messages, or source control.

**TM-AUTH-006, Personal Access Token Storage:**
```
User sees: evr_pat_<full-random-token>    (shown once at creation)
DB stores: SHA-256(evr_pat_<full-token>)  (irreversible)
Display:   evr_pat_<first-8-chars>...      (prefix for identification)
```

## 2. Cryptography (TM-CRYPTO)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-CRYPTO-001 | KEK compromise | Critical | Production KEKs are stored in `SECRETS_ENCRYPTION_KEY` and never committed; the repository's public local-development KEK is explicitly non-production and stable for persisted local data | MITIGATED |
| TM-CRYPTO-002 | Nonce reuse in AES-GCM | Critical | Fresh 12-byte random nonce per encryption; 2^96 space | MITIGATED |
| TM-CRYPTO-003 | Ciphertext tampering | High | GCM authentication tag detects modification | MITIGATED |
| TM-CRYPTO-004 | Known-plaintext attack | Medium | Unique DEK per encryption; same plaintext produces different ciphertext | MITIGATED |
| TM-CRYPTO-005 | Stale encryption key | Medium | Key rotation supported (primary + previous KEK); key_id in payload | MITIGATED |
| TM-CRYPTO-006 | Re-encryption job missing | Low | CLI tool `reencrypt_secrets` implemented with batch processing, dry-run mode, and key rotation detection | MITIGATED |
| TM-CRYPTO-007 | Limited encryption scope | Medium | LLM API keys encrypted; system prompt encryption reverted (PII should not be in prompts) | **OPEN** |
| TM-CRYPTO-008 | Machine-payment wallet key exposure | Critical | When machine payments are disabled, custody UI/navigation is hidden, payment commands are omitted from Platform/MCP discovery, and payment account/policy/attempt routes return a structured `feature_not_enabled` 404; when enabled, wallet private keys are accepted only on payment account create/update, encrypted with the server envelope encryption service, never returned from API responses, decrypted only inside `ServerPaymentAuthority` immediately before native rail signing, and never sent to workers | MITIGATED |

### Mitigation Details

**TM-CRYPTO-001, Envelope Encryption Architecture:**
```
Plaintext
    ↓
Generate random DEK (32 bytes) + nonce (12 bytes)
    ↓
Encrypt plaintext with DEK via AES-256-GCM → ciphertext
    ↓
Wrap DEK with KEK via AES-256-GCM → dek_wrapped
    ↓
Store JSON: {version, alg, key_id, dek_wrapped, nonce, ciphertext}
```

Key rotation: Deploy new KEK as `SECRETS_ENCRYPTION_KEY`, move old to `SECRETS_ENCRYPTION_KEY_PREVIOUS`. Both active for decryption; only new key used for encryption.

**TM-CRYPTO-006, Re-encryption (MITIGATED):**
Re-encryption CLI tool implemented at `crates/server/src/bin/reencrypt_secrets.rs`. Features:
- Batch processing with configurable batch size
- Dry-run mode for safety
- Per-table filtering
- Key rotation detection via `is_current_key()`
- Full UPDATE statements to write re-encrypted data back

**TM-CRYPTO-008, Machine-Payment Wallet Custody (MITIGATED):**
`FEATURE_MACHINE_PAYMENTS` gates the entire custody surface. When it is false, the payment
routes are not mounted and the UI removes Settings navigation, direct page access, and
global-search discovery. When it is true, payment accounts store wallet signing material in
`credential_encrypted`, protected by the same envelope encryption service used for other
secrets. The public `PaymentAccount` model intentionally omits this field. Native x402 signing
happens only in `ServerPaymentAuthority`; external workers call the control-plane
`ExecuteMachinePayment` RPC and never receive private keys.

## 3. Tenant Isolation (TM-TENANT)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-TENANT-001 | Cross-org resource access | Critical | All DB queries include `WHERE org_id = $org_id`; enforced at repository layer | MITIGATED |
| TM-TENANT-002 | Org enumeration via error codes | Medium | 404 returned for cross-org access (not 403); prevents existence discovery | MITIGATED |
| TM-TENANT-003 | Org cookie manipulation | High | Cookie value is `public_id`; server validates user membership against DB | MITIGATED |
| TM-TENANT-004 | Personal access token cross-org access | High | Personal access tokens are user-scoped; org resolved per-request via `X-Org-Id` header or cookie, validated against user's org memberships loaded from DB | MITIGATED |
| TM-TENANT-005 | Internal org_id exposure | Medium | `org_id` (BIGINT) never in APIs, URLs, logs, or error messages; only `public_id` exposed | MITIGATED |
| TM-TENANT-006 | Session inherits wrong org | Medium | Sessions scoped via agent FK; agent scoped to org; query joins enforce chain | MITIGATED |
| TM-TENANT-007 | Durable tasks cross-org | Medium | gRPC `GetTurnContext` validates org_id in request matches record in DB | MITIGATED |
| TM-TENANT-008 | User listing cross-org | High | `GET /v1/users` uses `ResolvedOrg` and calls `list_users_by_org(org.org_id)`, scoping results to the caller's org membership | MITIGATED |
| TM-TENANT-009 | AG-UI thread ID collision crosses app session boundaries | High | AG-UI session routing tags include both `ag_ui:app:{app_id}` and `ag_ui:thread:{thread_id}` so a shared thread UUID cannot attach to another app's session | MITIGATED |
| TM-TENANT-010 | Cross-org resource→org oracle via `/v1/resolve-org` | Medium | Endpoint requires `AuthUser` and answers only when the owning org is a membership of the caller (`is_organization_member` check before returning any identity). Unknown ids, unknown prefixes, and non-member owners all produce 404, identical to what the entity APIs would return. Attacker learns nothing they couldn't already learn by manually switching between their own orgs. See knowledge/security/multitenancy.md (Cross-Org Resource Resolution). | MITIGATED |
| TM-TENANT-011 | Org invite accepted by unintended user / cross-org grant | High | Invite acceptance binds membership to the invite row's `org_id` (never caller-supplied), and only when the accepting user's database email is verified and its normalized value equals the invited email. Org-scoped create/list/revoke routes use the `OrgAdmin` extractor, which returns 404 for non-members (no enumeration). Membership is local-DB authoritative and never derived from external identity-provider org claims. See knowledge/security/multitenancy.md (Organization Invitations). (EVE-602) | MITIGATED |
| TM-TENANT-012 | Latent cross-org regression via bare-id repository methods returning secrets/content | Low | Defense-in-depth; **no exploitable path today**. A handful of storage methods take a bare global identifier with no org/parent filter and return or mutate org-scoped secrets/content: `app_channels` get/update/delete (`channel_config_encrypted`), `agent_identity_connections` get/list/delete (`access_token_encrypted` / `refresh_token_encrypted`), and the now-removed dead `get_session_file_by_id` (file content). Every reachable caller already pre-scopes by org: app-channel callers fetch the parent app under the caller's org and assert `channel_row.app_id == app.id` (`domains/apps/commands.rs`); connection callers go through `resolve_identity` → `get_agent_identity(caller.org_id, identity_id)` (`api/agent_identity_connections.rs`). Hardening: deleted the unused `get_session_file_by_id` (pg + in-memory + backend facade) so no bare-id content reader exists, and added `THREAT[TM-TENANT-012]` invariant comments on the remaining methods stating callers MUST org-scope first. A future caller that skips the parent fetch would reintroduce the hazard, so these are flagged in source. The deeper fix (threading `org_id` into these methods) is deferred: the tables lack an `org_id` column and the change would cascade through both backends and the facade for no behavior change today. (EVE-634) | MITIGATED (defense-in-depth) |
| TM-TENANT-013 | Private scoped memory leaks through workspace/session files | High | Agent/user scoped Memories are hidden from public memory listing and cannot be explicitly mounted through the `memory` capability. `/memory/*` is reserved for server-managed mounts. User memory is only auto-mounted into default one-session workspaces; caller-attached shared workspaces do not receive `/memory/user` until runtime mounts are participant-local instead of workspace-wide. Session file APIs additionally check the session's `resolved_owner_user_id` before read/write/stat/move/copy access to `/memory/user` and redact that subtree from cross-user recursive listings, so org-wide session permissions cannot disclose or mutate another user's private memory. | MITIGATED |
| TM-TENANT-014 | Forged detached budget root links spend across organizations | High | `budget_root_session_id` is internal-only and stripped with all delegation metadata at the public HTTP boundary. Both PostgreSQL and in-memory session creation resolve the referenced session with the creating `org_id`, reject missing/foreign roots, and canonicalize through the stored root before insert. | MITIGATED |

### Mitigation Details

**TM-TENANT-001, Query Isolation:**
```sql
-- Every org-scoped query:
SELECT * FROM agents WHERE org_id = $1 AND id = $2;

-- Session access joins through agent:
SELECT s.* FROM sessions s
JOIN agents a ON s.agent_id = a.id
WHERE a.org_id = $1 AND s.id = $2;

-- Events join through session→agent:
SELECT e.* FROM events e
JOIN sessions s ON e.session_id = s.id
JOIN agents a ON s.agent_id = a.id
WHERE a.org_id = $1 AND e.session_id = $2;
```

**TM-TENANT-002, Error Response Strategy:**
```rust
// Cross-org access returns 404, not 403
ApiError::NotFound("Agent not found")    // ✓ No information leakage
ApiError::Forbidden("No access")         // ✗ Reveals resource exists
```

## 3b. Permissions / Authorization (TM-AUTHZ)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-AUTHZ-001 | Default Owner role grants full access | Medium | By design for phase 1; all users are Owners. Future phases will assign roles via admin UI/invitation flow | **BY DESIGN** |
| TM-AUTHZ-002 | Worker control plane trusts client-supplied `org_id`; one shared token grants cross-org authority | Medium | Every worker RPC builds `Caller::internal(req.org_id)` from the **client-supplied** `org_id`, which bypasses all policy (Owner role, `is_internal`). There is **no per-org token scoping**: any holder of the single shared worker bearer token can act on any org. Assumption/mitigation: the worker gRPC boundary is **never reachable from untrusted networks**, and the shared secret is **constant-time compared** in the auth interceptor (`grpc_service/mod.rs`) | **BY ASSUMPTION** (network isolation + shared secret) |
| TM-AUTHZ-003 | Policy error reveals permission names | Low | 403 response includes policy ID and required permission; acceptable for debugging, no internal state leaked | **ACCEPTED** |
| TM-AUTHZ-004 | Missing policy on mutating command | Medium | Every caller (HTTP/MCP/gRPC/platform) routes through `Command::run` which evaluates `Command::policy()`. Inventory coverage test (`crates/server/tests/command_policy_enforcement_test.rs`) asserts every non-GET command declares a policy, a missing declaration fails the build | MITIGATED |
| TM-AUTHZ-005 | Anonymous app channel reaches draft, disabled, or protected app config | High | Public AG-UI ingress requires `AppStatus::Published` and an enabled `ag_ui` channel before any request work. Legacy configs then require `anonymous=true` plus the configured token when present. New inline endpoint auth (`channel_config.auth`) bypasses the legacy anonymous flag only after the shared verifier accepts the configured credential policy. Failures return before session creation or image upload. | MITIGATED |
| TM-AUTHZ-006 | Anonymous webhook reaches draft or disabled app channel | High | Public webhook ingress requires `AppStatus::Published`, a `webhook` channel, `enabled=true`, and the per-channel token before creating or reusing a session | MITIGATED |
| TM-AUTHZ-007 | HTTP callers bypass declared command policy | High | Before the command runner, HTTP adapters called `Command::execute` directly, skipping `Command::policy()`. Now all adapters call `Command::run`, which enforces policy using `ctx.permission_resolver.evaluate_with`. Tests: `run_blocks_member_from_manage_command`, `dispatch_blocks_member_from_manage_command` | MITIGATED |
| TM-AUTHZ-008 | SaaS custom `PermissionResolver` bypassed during enforcement | High | Legacy `#[policy]` macro calls `Policy::evaluate(caller)` which hardcodes `DefaultPermissionResolver`, ignoring custom resolvers. `Command::run` now threads `ctx.permission_resolver` (from `AuthState`) into `evaluate_with`, so billing-tier / per-user grant resolvers apply uniformly. Tests: `run_honors_custom_resolver_denying_owner_write`, `dispatch_honors_custom_resolver` | MITIGATED |
| TM-AUTHZ-009 | External caller spoofs `app:`/`app_channel:` session tag to attach to another app's budget | Medium | App-scoped budgets cascade onto sessions via `app:<id>` / `app_channel:<id>` tags (see knowledge/security/budgeting.md). `SessionService::create` now rejects these prefixes from non-internal callers, mirroring the existing `__internal:` reservation. Only the apps domain (which routes through `Caller::internal`) can stamp them, so an org member cannot opt their personal session into a sibling app's budget cap. | MITIGATED |
| TM-AUTHZ-010 | Disabled LLM model still reachable through resolution paths | High | `llm_models.enabled = FALSE` is enforced at every model-resolution read: `Database::get_default_llm_model`, `get_llm_model_by_model_id`, and `get_llm_model` (UUID lookup used by agent execution and validation) all add `AND m.enabled = TRUE`; the in-memory backend mirrors the same gate. Admin listing (`list_all_llm_models`) intentionally returns disabled rows so operators can re-enable them through the management UI. Test: `test_disabled_model_is_not_resolvable_or_default_postgres` (`crates/server/tests/repository_integration_test.rs`) | MITIGATED |
| TM-AUTHZ-011 | Knowledge Index source edit reuses another user's GitHub connection | High | Updating `source_config` also rebinds `resolved_owner_user_id` to the caller, so subsequent syncs resolve the GitHub token for the user who selected the new source coordinates rather than the previous index owner. Metadata-only edits preserve the existing sync owner. Tests: `source_config_update_rebinds_sync_owner_to_caller`, `metadata_update_preserves_sync_owner` | MITIGATED |
| TM-AUTHZ-012 | Privilege escalation via org invite role | Medium | Creating, listing, and revoking invites requires the `OrgAdmin` extractor (admin+). Inviting the `owner` role additionally requires the caller to be an owner, mirroring `add_member` ("only owners can add owners"). Acceptance grants exactly the invited role and nothing more. Tests: `non_admin_cannot_invite_owner` and the HTTP integration suite. (EVE-602) | MITIGATED |
| TM-AUTHZ-013 | Coarse permission gates many unrelated domains (least-privilege) | Medium | The VIEW/MANAGE policies for apps, MCP servers, plugins, skills, capabilities, and agent identities used to share the single `OrgAgentsManage` permission, so a future custom `PermissionResolver` could not grant access to one domain without all six. Each domain now routes through its own per-domain `Org<Domain>View` / `Org<Domain>Manage` permission (`crates/core/src/permissions.rs`). Behavior is preserved for the built-in role map: every role that held `OrgAgentsManage` (Owner, Admin, Member) is granted the equivalent per-domain permissions; plugin **manage** stays Admin+ only via `OrgPluginsManage`. Tests: `eve656_roles_with_agents_manage_keep_per_domain_access`, `eve656_plugin_manage_stays_admin_plus_only`. (EVE-656) | MITIGATED |
| TM-AUTHZ-014 | Detached spawn bypasses session-creation permission | High | `spawn_agent(lifetime=detached)` requires a host-injected `SessionCreationAuthority` before child creation. Direct and gRPC hosts load the current session under its org, resolve its effective human owner, and evaluate `SESSION_MANAGE` / `OrgSessionsManage` with the active `PermissionResolver`; missing authority or denial produces a clear `ToolError`. | MITIGATED |
| TM-AUTHZ-015 | Knowledge Index model reference crosses an org or targets an unauthorized service | High | Create, update, and manual retry resolve model and provider inside the caller's org, require an enabled embeddings-tagged model and an active provider driver declaring `Embeddings`, and collapse missing, cross-org, disabled, and incompatible references into one error to prevent existence disclosure. | MITIGATED |
| TM-AUTHZ-016 | Disabled provider credentials remain usable by runtime tools | Medium | Org-scoped default credential resolution considers active providers only. Disabling a provider therefore prevents image and other runtime capability adapters from retrieving its stored key. Covered by `resolve_provider_credentials_ignores_disabled_provider`. | MITIGATED |
| TM-AUTHZ-017 | Org-disabled feature remains reachable through an alternate API, Platform, MCP, or persisted capability surface | Medium | Domain command execution checks org-effective flags at the shared command boundary; Platform/MCP discovery filters the same command metadata. Capability APIs, assignment validation, Platform listing, and worker turn-context assembly apply the same effective flags, including stripping feature-owned capabilities from persisted configurations. Direct non-command endpoints use the same `feature_not_enabled` response. Covered by focused command/capability tests and `test_disabled_feature_is_hidden_from_api_platform_and_mcp_catalog`. | MITIGATED |

### Mitigation Details

**TM-AUTHZ-001, Default Owner Role (BY DESIGN):**
Phase 1 assigns `OrgRole::Owner` as the default for all users. This means no permission-based restrictions are active in practice. This is intentional to avoid breaking existing workflows while the role assignment infrastructure is built in phase 2. As least-privilege groundwork for phase 2, the permission **granularity** is being tightened ahead of role assignment: per-domain `Org<Domain>View` / `Org<Domain>Manage` permissions now back each domain's policies (see TM-AUTHZ-013) instead of the coarse `OrgAgentsManage`, so future role/grant assignment can scope access per domain without re-plumbing policies.

**TM-AUTHZ-002, Worker control plane trusts client-supplied `org_id` (BY ASSUMPTION):**
Every worker→server RPC (`grpc_service/worker_service_impl.rs`) builds its caller via `Caller::internal(req.org_id)` (`crates/core/src/permissions.rs`), using the `org_id` supplied by the client in the request. `Caller::internal` constructs an Owner / `is_internal` caller that bypasses ALL policy evaluation. There is **no per-org scoping of the worker token**: the worker control plane authenticates with a **single shared bearer token**, so any client that presents a valid token can act on **any org**: the full tenant-isolation guarantee on the worker boundary rests on this one secret.

This is accepted by assumption rather than fixed by per-org token scoping (a large redesign). The two pillars of the assumption are:

1. **Network isolation**: the worker gRPC boundary MUST never be reachable from untrusted networks. HTTP handlers, by contrast, always construct `Caller` from `ResolvedOrg` with the user's actual role and are never able to reach `Caller::internal`.
2. **Shared secret integrity**: the gRPC auth interceptor (`grpc_service/mod.rs`) requires a bearer token in production (`TM-DURABLE-002`) and now compares it in **constant time** (`crate::security::constant_time_eq`), so the token cannot be recovered via a timing side-channel.

If the worker boundary is ever exposed to untrusted networks, or the shared token leaks, an attacker gains full cross-org Owner authority. Future hardening would scope worker tokens per org.

**TM-AUTHZ-004, Command Runner as Single Enforcement Point:**
`Command::run` (`crates/server/src/domains/common.rs`) evaluates `Command::policy()` against the active `PermissionResolver` before dispatching to `execute`. HTTP adapters call `run`; MCP and gRPC `ExecuteCommand` route through `dispatch()` which calls `run`. Coverage is enforced by iterating `inventory::iter::<CommandDescriptor>` in a test, so new mutating commands that forget `policy()` fail the build. The legacy `#[policy]` attribute macro was removed, service-layer checks were redundant with `Command::run` and hardcoded `DefaultPermissionResolver`, re-introducing `TM-AUTHZ-008`.

**TM-AUTHZ-007 / TM-AUTHZ-008, Historical gap:**
Prior to the command runner, only MCP/gRPC `dispatch` evaluated `Command::policy()`, and the evaluation used `Policy::evaluate` (default resolver only). HTTP adapters called `Command::execute` directly, so role-based restrictions and SaaS custom resolvers were not enforced on HTTP writes for fully-migrated domains. The runner closes both gaps in a single code path.

## 4. API Security (TM-API)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-API-001 | SQL injection | Critical | All queries use sqlx prepared statements (parameterized). The sessions list/facet filters (EVE-852) compose their WHERE clause dynamically, but only from `$N` placeholders and `&'static str` literals returned by closed enums (`SessionSource`/`SessionActivity`); the `&'static str` bound on `sql_string_list` makes it a type error for a runtime string to reach query text, and every request-supplied value stays a bound parameter. | MITIGATED |
| TM-API-002 | Large payload DoS | High | Input validation with size limits on agent/session/message fields | MITIGATED |
| TM-API-003 | Path injection in filesystem API | High | Regex constraint: `path ~ '^/([^/\0]+(/[^/\0]+)*)?$'`; no `..`, `//`, or null bytes | MITIGATED |
| TM-API-004 | Multipart upload abuse | Medium | Max file 100MB; max request 101MB; allowed MIME types: image/png, image/jpeg, image/gif, image/webp | MITIGATED |
| TM-API-005 | Internal error detail leakage | Medium | Generic `{"error": "Internal server error"}` for 500s; details logged server-side only | MITIGATED |
| TM-API-006 | Missing auth on protected routes | Critical | All protected routes require `AuthUser` extractor; compile-time route registration | MITIGATED |
| TM-API-007 | CORS misconfiguration | Medium | `CORS_ALLOWED_ORIGINS` not set by default (same-origin only); configurable for cross-origin | MITIGATED |
| TM-API-008 | WebFetch SSRF to internal services | High | fetchkit v0.1.2 `DnsPolicy::block_private_ips()` blocks loopback, RFC1918, link-local, and reserved IPs via resolve-then-check | MITIGATED |
| TM-API-009 | WebFetch cloud metadata access | Critical | fetchkit v0.1.2 blocks 169.254.0.0/16 (link-local) via DnsPolicy; infrastructure-level IMDSv2 recommended as defense-in-depth | MITIGATED |
| TM-API-010 | WebFetch internal DNS probing | Medium | fetchkit v0.1.2 resolve-then-check validates resolved IPs against blocked ranges before connecting | MITIGATED |
| TM-API-011 | WebFetch internal port scanning | Medium | fetchkit v0.1.2 blocks private IP ranges; agents cannot reach internal hosts | MITIGATED |
| TM-API-012 | WebFetch DNS rebinding | Medium | fetchkit v0.1.2 DNS pinning: resolves hostname, validates IP, pins to resolved address for connection | MITIGATED |
| TM-API-013 | LLM provider base URL SSRF | High | `validate_safe_url` blocks private IPs, loopback, link-local, cloud metadata, non-HTTPS on provider create/update (EVE-69). Request-time the shared provider HTTP clients (`driver_helpers::shared_streaming_http_client` / `shared_request_http_client`, used by the OpenAI-protocol/Anthropic/Gemini/embeddings drivers) disable redirect following and install an SSRF-guarding DNS resolver that rejects any hostname resolving to a blocked range, closing the DNS-rebind/302-redirect gap a create-time-only check leaves open (EVE-623) | MITIGATED |
| TM-API-014 | Search query SQL wildcard injection | Low | LIKE wildcards (`%`, `_`, `\`) in `?search=` input are escaped; tokens capped at 8 to prevent query amplification from long inputs | MITIGATED |
| TM-API-015 | Provider secret leakage via leased-resource metadata | High | Leased-resource metadata is explicitly non-secret; cleanup reconstructs provider auth from user connections/session secrets, and session resources stay org/session scoped | MITIGATED |
| TM-API-016 | Public-endpoint internal error and tool-detail leakage | High | AG-UI streaming `RUN_ERROR` payloads route every payload-phase error through `crates/server/src/api/public.rs::PublicError`, mapping internal codes to a stable public set (`rate_limited`, `service_unavailable`, `request_too_large`, `internal_error`); raw provider strings, model IDs, HTTP status codes, quota state, and stack traces never reach the wire. Public AG-UI tool activity is translated at the endpoint boundary according to `AgUiChannelConfig.tool_visibility` (`none`, `generic`, `narrated`) and never emits raw tool names, arguments, results, or internal tool call IDs. Universal fallback is `internal_error`. Pre-stream HTTP rejections (`bad_request`, `forbidden`, `not_found`, generic 500) keep their existing texts but already avoid internal detail. Other public endpoints (Slack webhook + manifest) inherit the same contract for any payload-phase errors they add. See `knowledge/execution/public-endpoints.md` | MITIGATED |
| TM-API-017 | Public AG-UI image upload abuse: oversize writes, MIME spoofing, decompression bombs | High | The public `/v1/apps/{app_id}/ag-ui/images` route caps body size at 10 MB (router `DefaultBodyLimit` plus in-handler check), validates the uploaded bytes match the declared content type via `image::guess_format` (rejecting MIME spoofing), and decodes thumbnails through `image::ImageReader` with explicit `Limits` (max width/height 20_000 px, max alloc 160 MB) so a crafted image cannot exhaust CPU or memory. Authenticated `/v1/images` retains the larger 100 MB cap behind authentication and rate limits | MITIGATED |
| TM-API-018 | Memory and Knowledge Index source credential leakage | High | Source-backed Memory and Knowledge Index creation normalize GitHub coordinates to credential-free `owner/repo`; unsupported hosts, inline credentials, query strings, and fragments are rejected before storage. GitHub credentials resolve from user/identity connections only at sync time. Clone failures map to credential-safe input, authentication/not-found, or network guidance in `last_sync_error`; raw provider details remain operator-only. | MITIGATED |
| TM-API-019 | CSV formula injection in report exports | Medium | Reporting CSV exports prefix formula-like cells (`=`, `+`, `-`, `@`, tab, CR, LF) with an apostrophe before RFC 4180 quoting so spreadsheet clients treat exported values as data, not formulas | MITIGATED |
| TM-API-020 | Task webhook SSRF | High | Org admins configure webhook URLs that the server POSTs to on terminal task transitions. A malicious URL could target internal services or cloud metadata. Mitigated by `validate_safe_url` applied on create/update, which blocks private IPs, loopback, link-local, and cloud metadata ranges (169.254.169.254). See `crates/server/src/api/task_webhooks.rs`. | MITIGATED |
| TM-API-021 | Provider OAuth connection forgery / SSRF | High | `GET /v1/providers/{id}/oauth/authorize` and `.../oauth/callback` let an org admin connect a provider (e.g. OpenRouter PKCE) without pasting a key; the obtained credential is written to the provider's encrypted credentials. Both endpoints require `provider.manage` (re-checked on the callback). CSRF/injection is mitigated by an HttpOnly, `Secure`, `SameSite=Lax`, 10-minute state cookie bound to the provider id, org id, and the PKCE verifier; the callback rejects any request whose cookie is missing or whose state/provider/org does not match, so a forged callback cannot inject an attacker's credential into a victim org. The token-exchange endpoint is driver-declared (not user input) and still passes `validate_safe_url` as defense-in-depth. See `crates/server/src/api/providers.rs`. | MITIGATED |
| TM-API-022 | ATIF trajectory import abuse (untrusted payload → eval cases) | Medium | `POST /v1/evals/{eval_id}/atif_import` (knowledge/evaluation/atif-adoption.md) parses attacker-supplied trajectory JSON/NDJSON. Gated by the `EVAL_MANAGE` policy; org-scoped through the eval-case lookup so cross-org eval ids are 404 (TM-TENANT-001). Resource exhaustion is bounded: axum's default 2 MiB transport body limit applies first (same posture as the OKF importer), with an app-level 4 MiB parse cap as defense-in-depth, plus 200 trajectories per call, 64 KiB per imported message, and a 2 000-char reference excerpt (`crates/server/src/atif.rs` import section, cap-enforcing unit tests). Malformed or unsupported payloads are rejected with 400 without echoing internals. Imported content is stored as inert case data (conversation text + description), never executed, and only interpreted as prompts when a user later runs the eval under their own agent policy | MITIGATED |

### Mitigation Details

**TM-API-002, Input Size Limits:**
```
Agent system_prompt: < 2 KB
Session title:       < 1 KB
Message content:     < 10 KB per part
Image upload:        < 100 MB
```
Returns `400 Bad Request: "Input exceeds allowed limits"` (generic, no detail leakage).

**TM-API-003, Path Validation:**
```
✓ /src/main.rs
✓ /folder/file.txt
✗ ../etc/passwd        (traversal blocked)
✗ /src//main.rs        (double slash blocked)
✗ /src/\0hidden.rs     (null byte blocked)
```
Enforced at both application layer and database constraint (`session_files_path_check`).

**TM-API-008, WebFetch SSRF (MITIGATED, fetchkit v0.1.2):**

Previously OPEN: Everruns called `fetchkit::fetch()` with no URL filtering configured. Reclassified from ACCEPTED after investigation revealed agents could reach internal services, cloud metadata, and arbitrary hosts from the worker container.

**Mitigation (fetchkit v0.1.2):** Upgraded to fetchkit v0.1.2 which implements resolve-then-check with DNS pinning via `DnsPolicy::block_private_ips()` (enabled by default). The `WebFetchTool` uses `fetchkit::fetch_with_options()` with default `FetchOptions`, which blocks:

| Range | CIDR | Purpose |
|-------|------|---------|
| Loopback | `127.0.0.0/8`, `::1` | Localhost |
| Private | `10.0.0.0/8` | RFC1918 Class A |
| Private | `172.16.0.0/12` | RFC1918 Class B |
| Private | `192.168.0.0/16` | RFC1918 Class C |
| Link-local | `169.254.0.0/16` | Cloud metadata |
| Link-local IPv6 | `fe80::/10` | IPv6 link-local |
| Carrier-grade NAT | `100.64.0.0/10` | CGNAT |
| Unique local IPv6 | `fc00::/7` | IPv6 private |
| Multicast | `224.0.0.0/4`, `ff00::/8` | Multicast |
| Special | `0.0.0.0`, `::`, `255.255.255.255` | Broadcast/unspecified |

IPv6-mapped IPv4 addresses (e.g., `::ffff:127.0.0.1`) are canonicalized before validation, preventing bypass via mapped addresses. DNS pinning resolves the hostname once and pins the connection to that IP, preventing DNS rebinding.

**Defense-in-depth:** Infrastructure-level controls (AWS IMDSv2, cloud firewall egress rules, worker network isolation) are still recommended as caller responsibilities.

**TM-API-009, Cloud Metadata Access (MITIGATED, fetchkit v0.1.2):**
Cloud metadata at `http://169.254.169.254/` is blocked by fetchkit's DnsPolicy which blocks the entire `169.254.0.0/16` link-local range.

- **Defense-in-depth:** Enable IMDSv2 (AWS), metadata concealment (GCP), or equivalent cloud-level protections. Block 169.254.0.0/16 egress at cloud firewall.

**TM-API-010, Internal DNS Probing (MITIGATED, fetchkit v0.1.2):**
Fetchkit's resolve-then-check validates all resolved IP addresses against blocked ranges before connecting. If an internal service name (e.g., `postgres`, `server`) resolves to a private IP, the request is blocked with `BlockedUrl` error. Error messages no longer distinguish between DNS failures and blocked addresses in a way that enables enumeration.

**TM-API-011, Internal Port Scanning (MITIGATED, fetchkit v0.1.2):**
Private IP ranges are blocked at the DNS resolution layer. Agents cannot reach internal hosts regardless of port, eliminating the port scanning attack vector.

**TM-API-012, DNS Rebinding (MITIGATED, fetchkit v0.1.2):**
Fetchkit v0.1.2 implements DNS pinning: the hostname is resolved once, all resolved IPs are validated against blocked ranges, and the first non-blocked IP is pinned via `reqwest::resolve()`. A second DNS lookup cannot return a different IP because the connection is pinned to the validated address.

**TM-API-015, Leased-Resource Metadata Secrets (MITIGATED):**
The session Resources API returns leased-resource metadata to users and the UI, so this feature must not persist provider bearer tokens or equivalent secrets in `metadata`.

- Lease registration stores only non-secret metadata needed for cleanup and debugging.
- Cleanup handlers reconstruct provider auth from the original user connection or session secret store at execution time.
- Session resources are still gated by the existing org/session ownership check before rows are listed.

Code references:
- [`crates/core/src/leased_resource.rs`](../../crates/core/src/leased_resource.rs)
- [`integrations/browserless/src/session_tools.rs`](../../integrations/browserless/src/session_tools.rs)
- [`integrations/daytona/src/state.rs`](../../integrations/daytona/src/state.rs)

## 5. Session Filesystem (TM-FS)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-FS-001 | Path traversal | High | DB constraint rejects `..`; paths must match `^/([^/\0]+(/[^/\0]+)*)?$` | MITIGATED |
| TM-FS-002 | Cross-session file access | Critical | Files scoped by `session_id` FK; session scoped by org via agent join | MITIGATED |
| TM-FS-003 | Null byte injection | High | Regex constraint rejects `\0` bytes | MITIGATED |
| TM-FS-004 | Double-slash bypass | Medium | Regex constraint rejects `//` | MITIGATED |
| TM-FS-005 | Readonly file modification or deletion | Medium | `is_readonly` flag enforced; readonly files cannot be modified or deleted; recursive directory deletion blocked if subtree contains readonly files | MITIGATED |
| TM-FS-006 | File content unencrypted at rest | Low | Stored as BYTEA in PostgreSQL; relies on infrastructure-level encryption (disk, TDE) | **ACCEPTED** |
| TM-FS-007 | No file access audit log | Low | File reads/writes not logged; privacy tradeoff | **ACCEPTED** |
| TM-FS-008 | Large file storage abuse | Medium | Per-session and per-file byte quotas enforced in `WorkspaceFileService` and `DirectWorkerAdapters`; configurable via `SESSION_FILE_MAX_BYTES` / `SESSION_FILE_SINGLE_MAX_BYTES` env vars (defaults: 500 MB/session, 100 MB/file) | MITIGATED |
| TM-FS-009 | CLI `initial_files` hidden-path exfiltration | High | Three-layer policy in `crates/cli/src/commands/agents.rs`: hard-deny floor (`DENIED_DOT_ENTRIES`) blocks `.env`, `.ssh`, `.aws`, `.gnupg`, `.git`, etc. unconditionally; built-in `ALLOWED_DOT_ENTRIES` permits common dev assets (`.github`, `.vscode`, `.claude`, `.mcp.json`, etc.); per-agent `initial_files_allow_hidden` manifest field extends the allowlist but cannot bypass the hard-deny floor. Skipped paths emit a stderr warning. See `knowledge/foundations/cli.md` (Initial Files Hidden Path Policy). | MITIGATED |
| TM-FS-010 | Object-store cross-tenant access (S3 blob backend) | Critical | When `STORAGE_BLOB_BACKEND=s3`, blob keys embed the tenant scope (`workspaces/{workspace_id}/…`, `images/org-{org_id}/…`); object access requires bucket credentials held only by the control plane; clients and workers never receive bucket credentials or presigned URLs (Everruns proxies all bytes). Org/workspace ownership is enforced above the storage layer exactly as for inline storage (unchanged). `STORAGE_S3_PREFIX` further isolates deployments sharing a bucket. See `knowledge/runtime-resources/object-storage.md`. | MITIGATED |
| TM-FS-011 | DR metadata exposure on offloaded objects | Low | Offloaded objects carry user metadata (`everruns-kind` + base64(JSON) recovery record with path/filename/size/sha256) for disaster recovery. Visible only to bucket-credential holders (the control plane); never presigned or forwarded to clients (`get()` returns bytes only). Sensitivity is equivalent to the object bytes and key already visible to such a caller. Private bucket + bucket-side encryption (SSE/KMS) recommended. | **ACCEPTED** |
| TM-FS-012 | Offloaded object content unencrypted at rest (S3) | Low | Bytes stored in the object store rely on bucket-side encryption (SSE / SSE-KMS); application-level envelope encryption of blobs is out of scope (mirrors TM-FS-006 for inline PostgreSQL storage). | **ACCEPTED** |
| TM-FS-013 | Multi-root host workspace escape | High | `WorkspaceRootSet` canonicalizes every registered host root, rejects duplicates and overlapping directories, rejects host-absolute paths outside the registered root set, and routes additional roots only through `/workspace/roots/<name>/...`. Each mounted root is backed by its own `RealDiskFileStore`, preserving per-root symlink rejection and containment checks; `MountFs` rejects `..` traversal under additional-root prefixes before normalization so an attempted escape cannot silently reroute to the primary root. | MITIGATED |
| TM-FS-014 | Canonical event JSONL exposes conversation content to other local users | Medium | `JsonlEventLog` creates new files with owner-only `0600` permissions on Unix. The embedder selects and protects the containing data directory; existing files retain their operator-managed permissions. | MITIGATED |
| TM-FS-015 | Framework workspace-policy bypass through alternate paths or hidden content | High | `WorkspacePolicy` validates one portable `/workspace` namespace, fails closed on traversal, NUL bytes, and platform separators, compares ASCII case conservatively across case-sensitive and case-insensitive providers, applies deny-over-allow precedence, protects hidden and common sensitive paths by default, and supports exact component write denies at every depth without depending on a public global blocklist. Listings and grep summaries are filtered as well as direct reads. Recursive-delete opt-in preflights every provider-visible descendant so it cannot override a deny. The host applies policy after resolving any platform filesystem factory, so custom providers do not bypass it. Concrete host providers retain responsibility for host-path containment and symlink handling. | MITIGATED |
| TM-FS-016 | Host filesystem symlink-swap race between validation and I/O | High | `RealDiskFileStore` rejects symlinks in every existing path component immediately before each operation, including symlinks introduced after store construction; regression coverage swaps a checked directory for an outside symlink between operations. The local host provider is not an OS sandbox, and a same-user process can still race the final check and path-based syscall. Applications with mutually untrusted local processes must use a sandboxed filesystem provider or OS isolation rather than treating `WorkspacePolicy` as a process boundary. | **ACCEPTED** |
| TM-FS-017 | Local session resume catalog serializes Agent credentials or configuration | High | The Framework local catalog persists typed session ids plus a size-bounded, credential-free opaque workspace binding. `InMemoryEngine` retains immutable Agent snapshots only in process. After restart, the application rebuilds trusted Agent behavior and explicitly attaches it to a new engine; attachment verifies that the id exists in the Agent-configured local catalog before the engine accepts the snapshot. MCP headers/environment values, provider credentials, and initial-file content never enter the catalog. Providers are contractually forbidden from placing credentials in binding payloads. A credential-sentinel acceptance test scans every local profile file. | MITIGATED |
| TM-FS-018 | Local Git head creation permits command injection, provider-state path redirection, lifecycle races, or implicit destructive cleanup | High | Repository/base inputs are trusted application configuration, never model input. The provider invokes Git without a shell, rejects option-like/NUL/oversized revisions, generates branch and worktree identities internally, protects its state root with owner-only permissions, rejects symlinked state/metadata/worktree paths, bounds metadata, and validates manifest identities plus derived worktree/branch paths before use. Provider lifecycle mutations are serialized. Drop has no cleanup behavior; archive retains contents, and explicit destroy removes only the worktree while retaining its branch. | MITIGATED |

### Mitigation Details

**TM-FS-001, Defense in Depth:**
Path validated at three layers:
1. **Application:** Path parsing rejects traversal patterns
2. **Database constraint:** `session_files_path_check` CHECK constraint
3. **Unique constraint:** `(session_id, path)` prevents collision

**TM-FS-008, Storage Quota:**
Per-session and per-file byte quotas are enforced at the application layer in both the HTTP API path (`WorkspaceFileService::create_file` / `update_file`) and the agent tool path (`DirectWorkerAdapters::write_file`). Limits are configurable via env:
- `SESSION_FILE_MAX_BYTES`, total bytes per session (default 500 MB)
- `SESSION_FILE_SINGLE_MAX_BYTES`, per-file ceiling (default 100 MB)

Writes that would exceed either limit fail with a clear error before any DB insert.

**TM-FS-015/016, Framework Host Workspaces:**

Policy is intentionally split from storage. `WorkspacePolicy` decides which
model-facing paths are visible or mutable, while the selected provider decides
how those paths map to storage. This keeps policy portable and provider-extensible
without claiming that a path policy is a host-process sandbox. The real-disk
provider rechecks symlink components per operation and fails closed for swaps
that happen before the check; eliminating a malicious same-user TOCTOU race
requires descriptor-relative/no-follow I/O or a sandbox provider.
Filesystem policy applies at `SessionFileSystem`; a custom tool that performs
direct host I/O or launches a shell is a separate capability boundary and must
be restricted or sandboxed by its embedder.

## 6. Session SQL Database (TM-SQL)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-SQL-001 | ATTACH database escape | Critical | SQLite authorizer callback blocks ATTACH/DETACH | MITIGATED |
| TM-SQL-002 | LOAD_EXTENSION code execution | Critical | Authorizer blocks `load_extension` function | MITIGATED |
| TM-SQL-003 | Storage exhaustion | High | 50 MB/database, 100 MB/session, 10 databases/session; VFS returns SQLITE_FULL | MITIGATED |
| TM-SQL-004 | CPU exhaustion (cartesian join) | High | 30-second query timeout via progress handler + tokio timeout | MITIGATED |
| TM-SQL-005 | Filesystem escape via SQLite | Critical | Custom VFS intercepts all I/O (no real filesystem); DEV_MODE uses in-memory | MITIGATED |
| TM-SQL-006 | Cross-session data access | Critical | VFS filenames use database UUID (not user-controlled); ATTACH blocked; FK chain enforces session isolation | MITIGATED |
| TM-SQL-007 | Concurrent write corruption | High | PostgreSQL advisory lock per database serializes writers; DEV_MODE uses RwLock | MITIGATED |
| TM-SQL-008 | Result size exhaustion | Medium | Max 1000 rows, max 1 MB payload per query; truncation notice appended | MITIGATED |
| TM-SQL-009 | Dangerous PRAGMA modification | Medium | Authorizer blocks write-mode PRAGMAs (journal_mode, page_size, locking_mode, etc.) | MITIGATED |
| TM-SQL-010 | Virtual table abuse | Medium | Authorizer blocks CREATE/DROP VIRTUAL TABLE | MITIGATED |
| TM-SQL-011 | Recursive CTE bomb | High | Progress handler interrupts after 30 seconds; row limit prevents unbounded output | MITIGATED |

### Mitigation Details

**TM-SQL-001 / TM-SQL-002, Authorizer Callback:**
```rust
// THREAT[TM-SQL-001]: ATTACH database escape
// THREAT[TM-SQL-002]: LOAD_EXTENSION code execution
// Mitigation: Authorizer callback denies dangerous operations
fn authorizer(action: AuthAction) -> Authorization {
    match action {
        Attach | Detach => Deny,
        Function { name: "load_extension" } => Deny,
        CreateVtab | DropVtab => Deny,
        Pragma { name, value } if is_write_pragma(name, value) => Deny,
        _ => Ok,
    }
}
```

**Allowed read-only PRAGMAs:** `table_info`, `table_list`, `table_xinfo`, `index_list`, `index_info`, `database_list`, `foreign_key_list`.

**TM-SQL-003, Size Limits:**
| Limit | Value |
|-------|-------|
| Per database | 50 MB |
| Per session (total) | 100 MB |
| Databases per session | 10 |
| Rows per query result | 1,000 |
| Payload per query result | 1 MB |
| Query timeout | 30 seconds |

## 7. Tool Execution (TM-TOOL)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-TOOL-001 | Unregistered tool execution | High | Only registered tools in `ToolRegistry` can execute; unknown tools rejected | MITIGATED |
| TM-TOOL-002 | MCP server response poisoning | Medium | Responses parsed defensively; errors converted to `{"error": "..."}` in tool result | MITIGATED |
| TM-TOOL-003 | MCP tool name confusion | Medium | Double-underscore separator (`mcp_server__tool_name`) prevents ambiguity | MITIGATED |
| TM-TOOL-004 | MCP server timeout abuse | Medium | Timeout capped; exponential backoff with max retry-after of 60s | MITIGATED |
| TM-TOOL-005 | Tool result prompt injection | Medium | Results returned as `tool_result` role, not injected into system prompt | MITIGATED |
| TM-TOOL-006 | Disabled MCP server still callable | Medium | Server `status` flag checked before execution; disabled servers rejected | MITIGATED |
| TM-TOOL-007 | MCP API key exposure | High | API keys encrypted at rest via envelope encryption; decrypted only at runtime | MITIGATED |
| TM-TOOL-008 | Tool policy bypass | Low | `requires_approval` policy planned but not yet enforced (all tools auto-execute) | **OPEN** |
| TM-TOOL-009 | No per-org tool rate limiting | Medium | `OutboundToolRateLimiter` trait wired into `ActAtom.execute_single_tool`; `OrgRateLimiter::check_outbound_tool_call` enforces 1000 RPM per org (in-memory governor or Valkey sliding window); limit tunable via `RATE_LIMIT_ORG_TOOL_CALLS_PER_MINUTE`; fail-open on Valkey errors to preserve availability | MITIGATED |
| TM-TOOL-010 | Skill SKILL.md prompt injection | Medium | Skill instructions returned as `tool_result` role (not system prompt); `<skill>` XML wrapper provides clear boundary | MITIGATED |
| TM-TOOL-011 | Skill archive path traversal | High | ZIP extraction validates all paths; rejects `../`, absolute paths, symlinks; max 100 files, 1 MB each, 10 MB total | MITIGATED |
| TM-TOOL-012 | Skill archive zip bomb | High | Decompressed size capped at 10 MB; file count capped at 100; individual file size capped at 1 MB | MITIGATED |
| TM-TOOL-013 | Skill name collision across orgs | Medium | Skill names are unique per organization; capability IDs include UUID for global uniqueness | MITIGATED |
| TM-TOOL-014 | Disabled skill still activatable | Medium | `CapabilityService.list_all()` filters out disabled skills; disabled skills not included in `<available_skills>` | MITIGATED |
| TM-TOOL-015 | Browserless SSRF via tool URL | Medium | Shared `validate_safe_url()` blocks private/internal hosts for all Browserless navigation entry points, including interaction-step redirects | MITIGATED |
| TM-TOOL-016 | Browserless timeout DoS | Medium | All wait/timeout values capped at 120s; prevents unbounded resource consumption | MITIGATED |
| TM-TOOL-017 | Browserless API token in logs | Medium | CDP debug logging redacts token from WebSocket URLs before logging | MITIGATED |
| TM-TOOL-018 | MCP server SSRF via configured server URL | High | MCP server URLs are validated on create/update (static) and re-validated at execution time with DNS resolution via `validate_url_dns_pinned`; every resolved IP is checked against the blocked ranges, closing the DNS-rebinding gap | MITIGATED |
| TM-TOOL-019 | MCP `query`/`execute` positional-arg rewrite injection | Low | Rewriter only inserts compile-time `--<flag>` tokens at statement-start boundaries, respects quotes/escapes/comments, and never modifies or reorders user bytes. Flag names come from the command registry, not user input. See EVE-323. | MITIGATED |
| TM-TOOL-020 | Skill `` !`command` `` activation RCE on worker host | High | `ActivateSkillFromVfsTool::execute_with_context` never invokes `preprocess_command_injections`, the trust gate is forced off because no non-user-spoofable provenance signal exists on `SessionFile` today. Expansion is also capped at `MAX_COMMAND_PLACEHOLDERS_PER_SKILL` (32) with concurrency ≤ 4 in the expansion function itself. See EVE-388. | MITIGATED |
| TM-TOOL-021 | Agent handoff credential leakage or unauthorized target delegation | High | `agent_handoff` delegates only to configured target Agent ids through `spawn_agent(target.type="agent")`, requires server-side `UserConnectionResolver` checks before start, never accepts credentials in tool args/config/task metadata, records only non-secret target/provider/scope labels in `session_tasks`, and keeps invite-mode joins in the current session behind duplicate capability/mount conflict checks. Child-session follow-up control routes through `message_task` for the parent-owned task. Provider tools must still enforce scoped grants before real external writes. | MITIGATED |
| TM-TOOL-022 | Cross-tenant MCP credential reach via guardrail `mcp` check | High | The guardrails `mcp` check (knowledge/execution/guardrails.md) names a `server`/`tool` to call over the scoped-MCP client. The check resolves connections through the host's per-session `McpConnectionResolver`, which only resolves servers scoped to the current session/org; a tenant's guardrail config can only reach that tenant's servers and never another tenant's MCP credentials. A misconfigured/unknown server fails open (allow) rather than blocking. The verdict parser reads only `verdict`/`reason`, so a poisoned endpoint response cannot widen behavior beyond the fail-open baseline. | MITIGATED |
| TM-TOOL-024 | WebFetch rendered-page JavaScript escapes egress controls or exhausts worker resources | High | Rendered fetch is explicitly request-opt-in (`render: "rakers"`) on an already admin-gated high-risk capability. FetchKit fetches the initial document through the normal DNS/egress/redirect/body-size policy, runs inline scripts with a per-script timeout, sends renderer subresource traffic to a local deny proxy, and caps rendered output before conversion. Everruns integration tests prove the backend is opt-in and returns rendered content; FetchKit upstream tests cover subresource denial, timeout, and output capping. | MITIGATED |
| TM-TOOL-025 | MCP OAuth refresh-token theft, torn rotation, or refresh stampede | High | Refresh tokens stay envelope-encrypted and are never logged; token endpoints are revalidated and DNS-pinned through `EgressService` for every refresh (TM-TOOL-018); a bounded per-grant single-flight coalesces concurrent refreshes; access token, rotated refresh token, expiry, and scope are persisted atomically before the new access token is returned. Rejected or incomplete grants fail closed to `connection_required`. | MITIGATED |
| TM-TOOL-026 | Model-visible tool lacks a required host service and fails only after invocation | Medium | Context-aware tools declare hard `ToolContext` service requirements. Runtime hosts build one complete service snapshot, validate the active registry before reason exposes definitions, and clone that snapshot into every act call. Configuration errors name the tool and missing service. Runtime parity and reason→act history tests cover propagation. | MITIGATED |
| TM-TOOL-027 | `query_history` bypasses provider-bound prompt rewrites | High | Persisted messages intentionally retain the original user input as an audit record. When any `user_prompt_submit` hook is configured, runtime capability assembly removes `query_history` from both the reason and act registries so the model cannot retrieve text that a hook removed, while retaining Infinity Context's message filter so prior raw audit messages cannot re-enter the live provider prompt. Provider-visible history can restore the combination once it has a separate durable representation. | MITIGATED |
| TM-TOOL-028 | Cached MCP `tools/list` result reused across authorization contexts | Medium | `2026-07-28` servers return `cacheScope` on `tools/list`. The client caches under a key carrying an explicit scope variant (`crates/mcp/src/http.rs::ToolsCacheScope`): `public` results share one entry per server, `private` results, and any result whose scope is missing or unrecognized, the conservative default, are keyed by the resolved credential and never shared. The scope is a distinct enum variant rather than a sentinel credential hash, so a credential cannot collide into the shared bucket. A server that mislabels caller-specific data as `public` can still over-share its own catalog; per spec, `cacheScope` is the server's declaration to make. | MITIGATED |
| TM-TOOL-029 | MCP tool-argument credential reaches model context, events, logs, or another tenant | Critical | Agent credential values use a write-only org/Agent-scoped API and encrypted storage. Runtime removes the bound parameter from model schemas, persists the model's credential-free arguments, rejects model overrides, and injects plaintext only in the outbound MCP executor argument clone. Before the executor returns, it redacts every injected value from successful tool content (including resources and images) and transport/JSON-RPC errors, preventing a remote server from reflecting credentials into model context, events, persistence, or logs. Endpoint mismatch or missing value fails closed with a non-secret `credential_required` setup result. Repository and API mutations include org + Agent ownership predicates; responses expose metadata and configured state only. Sentinel tests cover successful and error reflections. | MITIGATED |
| TM-TOOL-030 | Capability config payload leaks through Debug/log output, or capability refs bypass validation on one write surface | Medium | Capability identity/configuration is one neutral contract (`everruns-capability`, EVE-873): `CapabilityRef`, also the persisted `AgentCapabilityConfig` attachment row and `BuiltInCapabilityDefinition` provisioning entry, redacts its config from `Debug`, so attachment rows in logs and error contexts never expose config payloads (which may carry handles or misplaced credentials). The Framework `AgentBuilder::build` and server capability write paths call the same ID grammar (incl. reserved `__everruns_` namespace) and JSON-object config validation, so a ref rejected by one surface cannot be persisted through another. Guarded by `scripts/lib/check-capability-contract.sh` against reintroduction of competing capability types; covered by redaction and write-path consistency tests. | MITIGATED |
| TM-TOOL-031 | Repository-controlled MCP configuration executes commands on the coding CLI host | High | The coding CLI (`examples/coding-cli`) registers no MCP servers and exposes no `--mcp-config` flag, so a repository-local `.mcp.json` cannot introduce a process-spawning stdio server on the developer machine. MCP setup left the CLI with the coding-agent simplification (#3186); `examples/coding-cli/tests/historical_parity.rs` asserts `--mcp-config` stays absent from the rendered help, so reintroducing implicit or explicit MCP loading without a trust decision fails CI. Hosted MCP execution keeps its own controls under TM-TOOL-002 and TM-TOOL-028. | MITIGATED |

### Mitigation Details

**TM-TOOL-002, Defensive MCP Parsing:**
MCP tool execution flow:
1. Parse tool name: extract server name and tool name from `mcp_<server>__<tool>` format
2. Validate server exists and is not disabled
3. POST JSON-RPC `tools/call` to server URL with decrypted API key
4. Parse response (JSON or SSE format); malformed responses become tool errors
5. Convert MCP content types to internal format
6. Return to LLM as tool result

**TM-TOOL-005, Prompt Injection Boundary:**
Tool results occupy the `tool_result` message role in the conversation. They are not concatenated into the system prompt. The LLM processes them as structured tool outputs, not instructions. However, LLMs may still be influenced by adversarial content in tool results (inherent limitation of current LLM architecture).

**TM-TOOL-010, Skill Instruction Injection Boundary:**
When `activate_skill` is called, the full SKILL.md body is returned as a tool result wrapped in `<skill name="...">` XML tags. This maintains the tool_result role boundary. Only skill names and descriptions appear in the system prompt (via `<available_skills>` XML block), limiting the injection surface to metadata validated during upload.

**TM-TOOL-020, Skill Command Injection Trust Gate:**
SKILL.md content may contain `` !`command` `` placeholders that, if expanded by `preprocess_command_injections`, spawn shell processes on the worker host. This is RCE against the worker if the SKILL.md body is attacker-controlled.

1. The trust signal must be a non-user-controllable provenance indicator for the SKILL.md entry read from the session VFS, for example, an origin field populated only by the capability/registry mount layer. `SessionFile::is_readonly` is **not** such a signal: both the session-files HTTP API (create/update) and `InitialFile` configuration accept `is_readonly = true` from user input.
2. Because no such provenance signal exists on `SessionFile` today, the enforcement point in `ActivateSkillFromVfsTool::execute_with_context` keeps `is_trusted_source = false` for every source. `preprocess_command_injections` is never reached at runtime.
3. The function itself is preserved (full implementation, unit-test coverage) with bounded fan-out: at most `MAX_COMMAND_PLACEHOLDERS_PER_SKILL` (32) placeholders expanded per activation, at most 4 shells concurrently. These bounds protect a future re-enable from per-activation CPU / process exhaustion.
4. SKILL.md content originating from user-facing session/file creation or update flows, including the session-files API, `initial_files`, and runtime `write_file` calls, stays untrusted regardless of metadata.
5. The single enforcement point is `ActivateSkillFromVfsTool::execute_with_context` in `crates/builtins/src/skills.rs`. `preprocess_command_injections` in `crates/core/src/skill.rs` assumes the caller has already performed the trust check.
6. Command execution MUST target the session sandbox (bashkit shell) against the session virtual filesystem, not the worker host shell. The current `ProcessCommandExecutor` (which spawns host `bash -c`) is dormant scaffolding only; re-enabling command substitution without also routing it through the session sandbox would still be RCE against the worker host. Any re-enable PR must both (a) introduce the provenance signal in (1) AND (b) replace host-bash execution with a sandbox-backed executor before flipping the gate.

Follow-up work (tracked on EVE-388): (a) add a platform-controlled provenance field, e.g. a `mount_capability_id` column on `session_files` populated only by mount application code and rejected on all user-facing API paths, AND (b) replace `ProcessCommandExecutor` with a session-sandbox-backed executor (`bashkit` / managed session sandbox) so execution is confined to the session VFS. Both must land before the gate is flipped. See `knowledge/project/skills-registry.md` "Activation Substitution Pipeline" for the source/outcome matrix.

**TM-TOOL-021, Agent Handoff Delegation Gate:**
`agent_handoff` is a high-risk orchestration tool because one agent can start a
child session using another configured Agent's tools and data, or invite another
configured Agent to respond inside the current session. The mitigation is to
keep authority explicit and non-secret:

1. Source agents can only target ids listed in their `agent_handoff` config.
2. Required provider connections are resolved server-side through
   `UserConnectionResolver`; bearer tokens are never accepted in tool arguments,
   capability config, system prompt context, or session task metadata.
3. Handoff task records store only non-secret target id, target agent id,
   provider ids, scope labels, mode, and lifecycle metadata. Full credentials
   are excluded.
4. Follow-up control uses `message_task` against the parent-owned
   `agent_handoff` task, whose `links.child_session_id` is set by the server.
5. Invite mode adds the target as a current-session member participant instead
   of creating a task. Before joining, it rejects duplicate capability
   configuration and mounted-resource conflicts so the shared environment is
   not silently overwritten.
6. This gate proves the user has the required connection before delegation.
   Real provider write tools still need scoped grant enforcement before mutating
   external infrastructure.

**TM-TOOL-011/012, Skill Archive Validation:**
ZIP archive extraction in `SkillService::create_from_archive()` enforces:
1. No path traversal: paths checked for `../`, absolute paths, and symlinks
2. File count limit: max 100 files per archive
3. Per-file size limit: 1 MB per individual file
4. Total decompressed size limit: 10 MB
5. Files extracted into `skill_files` table as individual rows (no runtime ZIP extraction)

## 7b. MCP Server (TM-MCP)

This section covers Everruns acting as an MCP server for external MCP clients. It does not cover harness/session tool execution or Everruns acting as an MCP client to remote MCP servers; those belong under `TM-TOOL`, `TM-AGENT`, and integration-specific categories.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-MCP-001 | Everruns MCP server tenant escape | Critical | The first-party `/mcp` endpoint is always mounted, so feature availability is deliberately not a security boundary. MCP-specific authentication and org resolution run before dispatch; target-org overrides re-check current membership; `query` exposes only read-only inventory commands; `execute` and tier-1 agent/session tools dispatch through org-scoped domain commands and `Command::run` policy checks. The root-route API rate limiter remains applied to `/mcp`. Authenticated smoke testing verifies anonymous requests fail closed; regression coverage in `crates/server/tests/mcp_endpoint_test.rs` proves fresh organizations need no opt-in and chains `discover`, `query`, `execute`, `agent_run`, `session_get_status`, and `session_send_message` against cross-org bait with no read/write escape; resources/read is also covered against cross-org bait. | MITIGATED |
| TM-MCP-002 | Mutating command exposed through read-only `query` | High | `query` builds a read-only command toolset from `Command::read_only()`. Inventory coverage in `crates/server/tests/command_policy_enforcement_test.rs` allows only a small reviewed set of POST-style read helpers to override `read_only() == true`. | MITIGATED |
| TM-MCP-003 | Card HTML XSS via entity-controlled fields | High | Every interpolation in `crates/server/src/api/mcp_endpoint/cards.rs` flows through a single `escape_html` helper (covered by `escapes_all_html_specials` and `agent_card_renders_and_escapes` unit tests); the rendered document carries an inline `Content-Security-Policy` meta tag (`default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; connect-src 'none'`); host-side rendering uses an iframe with `sandbox="allow-scripts"` (no `allow-same-origin`/`allow-forms`/`allow-popups`/`allow-top-navigation`) and `referrerpolicy="no-referrer"`; `srcdoc` populates the iframe directly so the document is never fetched over HTTP. See `knowledge/ui/mcp-cards.md`. | MITIGATED |
| TM-MCP-004 | Card-driven CSRF or unauthorized mutation | High | Cards in `cards.rs` are read-only and contain no out-of-band write path. The phased action protocol routes button clicks through host `postMessage` → host-issued `tools/call`, so Everruns's normal MCP auth, `Command::run` policy checks, and per-call `organization_id` resolution are re-applied to every action. Hosts MUST validate `MessageEvent.source` against the iframe `contentWindow` (enforced by `apps/ui/src/components/mcp/mcp-card-iframe.tsx`) and apply user confirmation for tools whose annotations are not `read_only_hint: true`. | MITIGATED |
| TM-MCP-005 | Card-induced denial of service via oversized HTML | Medium | `cards::render_html` enforces a 64 KiB rendered-document cap (`MAX_CARD_BYTES`), rejecting (rather than truncating) oversized cards (covered by `render_caps_size`). Card tool timeouts (`10s`) and `count_sessions_for_agent` single-COUNT queries bound server-side cost. Host-side iframe rate limiting in `mcp-card-iframe.tsx` (10 messages/sec token bucket) bounds inbound `postMessage` storms. | MITIGATED |
| TM-MCP-007 | MCP OAuth discovery/registration/token SSRF | High | The MCP OAuth flow fetches `/.well-known/oauth-protected-resource`, `/.well-known/oauth-authorization-server`, the dynamic-registration endpoint, and the token endpoint. The registration/token/authorization endpoints come from the attacker-controlled discovery response, and the MCP server URL is org-configurable and validated only at create time, so a `validate_safe_url`-passing host could DNS-rebind or 302-redirect to `169.254.169.254`/loopback/RFC1918 at fetch time. These fetches previously used a bare `reqwest::Client::new()` (follows redirects, no DNS pinning). Fix (EVE-623): all four fetches in `crates/server/src/api/user_connections.rs` route through the host `EgressService` via `egress_oauth_json`, which resolves-and-pins the connection (`validate_url_dns_pinned` + `pinned_addrs`) and inherits the boundary's disabled-redirect policy, refusing any endpoint that resolves to a blocked range. Covered by `oauth_egress_blocks_private_ip_literal_before_send` and `oauth_egress_blocks_localhost_hostname_before_send`. | MITIGATED |
| TM-MCP-006 | MCP OAuth access token confused-deputy (missing audience binding) | High | MCP OAuth access tokens were previously minted by `JwtService::generate_access_token`, byte-identical to a browser/session token (`token_type="access"`, no audience). Because `/mcp` and `/api/*` shared one `AuthUser` extractor → `validate_token`, a token a user authorized only for `/mcp` was accepted as a full user token across the entire REST API. Fix (EVE-596): `mcp_oauth.rs` now mints via `JwtService::generate_mcp_access_token` with `token_type="mcp_access"` and `aud={root}/mcp` (RFC 8707 resource binding). The general path `validate_token` → `validate_access_token` rejects `mcp_access`; the `/mcp` endpoint uses a separate `McpAuthUser`/`McpResolvedOrg` extractor → `AuthBackend::validate_mcp_token` → `validate_mcp_access_token`, which accepts only `mcp_access` tokens bound to the exact `/mcp` resource and rejects regular session/access tokens and cookie sessions. Acting-as-user is preserved; unbounded-resource is not. The mint+validate split is exposed on `JwtService`/`AuthBackend` so SaaS wrappers (PropelAuth) adopt it; `validate_mcp_token` defaults to fail-closed (`401`). Covered by `auth::jwt::tests::test_mcp_token_*`, `auth::builtin::tests::mcp_token_audience::*`, and `crates/server/tests/mcp_endpoint_test.rs::test_oauth_token_is_mcp_scoped_with_audience`. (Maps to TM-SAAS-AUTHZ-007 upstream.) | MITIGATED |
| TM-MCP-008 | Remote MCP OAuth resource substitution | High | Protected-resource discovery is controlled by the remote MCP server. The shared MCP client validates that the advertised RFC 8707 resource is an HTTPS URL on the configured server's origin before constructing an authorization URL or exchanging a code, preventing a malicious endpoint from obtaining a token minted for another resource. Covered by cross-origin and insecure-resource rejection tests in `crates/mcp/src/oauth.rs`. | MITIGATED |

### Mitigation Details

**TM-MCP-001, Everruns MCP Server Tenant Escape:**
The first-party MCP endpoint is both a discovery surface and an execution surface: `discover` publishes the command catalog, `query` runs bashkit scripts over read-only builtins, `execute` runs scripts over the full command set, and tier-1 tools (`agent_run`, `session_send_message`, `session_get_status`) compose session and message commands directly. The threat is a caller using catalog discovery plus scripted control flow, guessed IDs, or `organization_id` overrides to read or mutate another organization.

Mitigations are layered:
- The request `ResolvedOrg` is derived from authenticated org membership; per-tool `organization_id` overrides are resolved against fresh membership before dispatch.
- API-key callers cannot use per-tool org overrides to switch away from the org selected by request auth.
- `query` receives a read-only toolset built from inventory descriptors whose `read_only()` flag is true.
- `execute` and tier-1 tools dispatch through domain commands, preserving repository org filters and `Command::run` policy checks.
- MCP `resources/read` routes through policy-gated list commands instead of raw storage reads.
- Bashkit runs the scripted surface with parser, input, command-count, loop, function-depth, AST-depth, and timeout limits.

Regression coverage: `test_mcp_adversarial_tool_chain_cannot_escape_org_scope` creates real cross-org bait, discovers the relevant agent/session operations, then attacks the wrong org with `query`, `execute`, `agent_run`, `session_get_status`, `session_send_message`, and a non-member `organization_id` override. `test_mcp_resources_read_cannot_escape_org_scope` verifies resource reads do not leak cross-org agent summaries. Both tests assert no data leak and no mutation.

**TM-MCP-002, Read-Only Query Catalog Drift:**
The MCP `query` tool is intentionally safer than `execute`, but that safety depends on the inventory metadata for every command. By default, only `GET` commands are read-only. POST-style helpers must explicitly override `read_only() == true`; each such override is reviewed in `mcp_query_read_only_overrides_are_allowlisted` to prevent a future mutating command from becoming available through `query`.

**TM-TOOL-015, Browserless URL Validation (MITIGATED):**
Browserless tools now reuse the shared `validate_safe_url()` policy from core. This blocks:
- loopback and `localhost`
- RFC1918 private ranges
- link-local and cloud metadata endpoints
- IPv6 loopback/link-local/private ranges

Validation runs for:
- direct Browserless tool URLs (`navigate`, `content`, `screenshot`, `scrape`)
- `browserless_open_browser` initial navigation
- nested `navigate` actions inside `browserless_interact`

**TM-TOOL-018, MCP Server SSRF (MITIGATED):**
MCP server URLs are validated twice:
1. On create/update in the control plane, static check via `validate_safe_url` rejects unsafe schemes, loopback, RFC1918, link-local, and cloud metadata targets.
2. At execution time (before each tool call and `tools/list` fetch) via `validate_url_dns_pinned`, performs the same static checks then resolves the hostname via `tokio::net::lookup_host` and verifies every returned IP against the blocked ranges. This closes the DNS-rebinding gap: an attacker cannot register a public hostname that initially resolves to a safe IP but later rebinds to an internal address, because the IP is re-checked on every outbound request.

The same execution-time validation and DNS pinning applies to MCP OAuth token
endpoint requests, including lazy refreshes; cached OAuth discovery metadata is
never treated as permission to bypass the egress boundary.

## 8. LLM Integration (TM-LLM)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-LLM-001 | API key at rest exposure | Critical | Encrypted via envelope encryption (AES-256-GCM); stored in `llm_providers.api_key_encrypted` | MITIGATED |
| TM-LLM-002 | API key in logs | High | Never logged; tracing filters sensitive fields; generic error messages only | MITIGATED |
| TM-LLM-003 | API key in error messages | High | Provider errors are sanitized before returning to users; full errors remain server-side. Agent Analyze responses and persisted health-check terminal errors use the shared user-facing provider taxonomy, which preserves actionable failure categories without copying raw provider bodies. Invalid tool-schema rejections expose only a stable category and bounded schema path in standard mode; raw provider text remains restricted to operator-enabled detailed disclosure. | MITIGATED |
| TM-LLM-004 | API key lifetime in memory | Medium | Decrypted only at the org-scoped provider composition boundary and held in a non-serializable runtime provider for its bounded execution lifetime; model DTOs/events never carry it; provider/request `Debug` redacts values | MITIGATED |
| TM-LLM-005 | Provider failure retry amplifies cost or hides permanent account errors | Medium | Provider-boundary semantic classification separates transient transport/stall/overload/rate-limit/server failures from credentials, billing quota, unavailable models, invalid requests, and long-horizon usage limits. Automatic recovery is allowed only before assistant output commits, uses jittered exponential backoff, and is bounded by shared attempt and elapsed-time budgets; lower layers report consumed retries so nested loops cannot multiply attempts. Permanent classes fail fast. | MITIGATED |
| TM-LLM-006 | Provider MITM | High | HTTPS required for all LLM provider communication | MITIGATED |
| TM-LLM-007 | Indirect prompt injection | High | Tool results and user messages are role-separated; no complete mitigation exists for LLM-level prompt injection | **ACCEPTED** |
| TM-LLM-008 | Cost runaway via agent loop | Medium | Max 10 iterations per agent turn; configurable | MITIGATED |
| TM-LLM-020 | Client-supplied privileged message roles in AG-UI input | Medium | Anonymous AG-UI/CopilotKit clients could send `role: "system"` / `developer` / `tool` messages that flow into the LLM context alongside the agent's real system prompt. Mitigated in `crates/server/src/api/ag_ui.rs::validate_input_messages` by rejecting any non-{user,assistant} role at the runtime trust boundary with a generic 400 `invalid_request`, and by rejecting duplicate message ids. | MITIGATED |
| TM-LLM-021 | Utility LLM key exposed through agent model configuration | High | The utility LLM uses deployment env secret `UTILITY_OPENAI_API_KEY`, is carried as a host service on `HostComposition`, and is threaded only into capability `ToolContext`. It is not stored in provider records, exposed through model selection, or accepted from session/agent config. | MITIGATED |
| TM-LLM-022 | Tenant execution silently spending platform env keys | High | `LlmResolverService::resolve_provider_api_key` and `resolve_provider_credentials` are fail-closed: they return `None` when no database key is found rather than falling back to `DEFAULT_*_API_KEY` env vars. A selected-but-unconfigured provider may still assemble a command context so setup can repair it, but `DriverRegistry` wraps its driver in a credential gate that rejects chat, model-listing, and compaction locally before network I/O. `ProviderStore::get_provider_config` has no implicit default, forcing custom hosts to declare credential ownership. Env var helpers remain available only for explicit dev/CLI entrypoints. For single-tenant/dev convenience, `seed::seed_default_provider_keys_from_env` may materialize `DEFAULT_*_API_KEY` into the **default org's** provider rows at startup (encrypted), gated by `SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV` (defaults to `DeploymentGrade::is_dev()`). Non-dev opt-in is ignored while built-in signup or built-in OAuth can self-provision users into `DEFAULT_ORG_ID`, so open-registration deployments cannot seed platform keys into an org that untrusted users can join and spend from. See `knowledge/foundations/llm-drivers.md` (Key Resolution Contract). | MITIGATED |
| TM-LLM-023 | Provider credentials exposed through the capability command or kernel context contract | High | The `CommandHost` facilities (knowledge/project/commands.md, EVE-543) give capability `execute_command` implementations access to the session's turn context and a tool-less completion against the session's resolved model. `CommandTurnContext` is a deliberately credential-free view (session id, model name, and provider type only); concrete context loading, provider configuration, driver creation, and completion live in `everruns_host::StoreCommandHost`. Main-turn host orchestration likewise converts credential-bearing provider configuration into a safe model/provider identity plus an opaque, non-serializable ready driver before core execution. Debug output redacts the driver. Per-invocation model overrides resolve through the same org-scoped provider store as a main turn. Completions are out-of-band: nothing is persisted to messages or events. | MITIGATED |
| TM-LLM-024 | Provider error detail leaking to untrusted session viewers | Medium | Session error disclosure is governed by the `error_disclosure` capability (`knowledge/execution/error-disclosure.md`). `detailed` mode (provider error text in a `detail` field) is operator-opt-in per harness/agent; per-message `controls.error_disclosure` is clamped to the capability-configured ceiling so clients can narrow but never widen disclosure; `generic` mode collapses all blocking errors for public-facing agents. Public endpoints sanitize independently via `PublicError` (TM-LLM-003 unchanged: provider error bodies never include API keys). | MITIGATED |
| TM-LLM-025 | Agent config / response steering the utility LLM during agent checks | Medium | Agent checks (knowledge/evaluation/agent-checks.md) pass the agent's own prompt, generated cases, and agent responses to the utility LLM for analysis, case generation, and judging. All such text is untrusted and is wrapped as XML-tagged data with metacharacters escaped (`xml_escape`); output is parsed into fixed shapes (findings, cases, a judge verdict) with severity/count/length clamps. A steered judge can at worst mis-score an advisory, behavioral health check, which never gates save, publish, or version creation. | MITIGATED |
| TM-LLM-026 | Health check runs spending agent model budget | Medium | `trigger_agent_health_check` (knowledge/evaluation/agent-checks.md) runs real sessions on the agent's configured model. Bounded: a fixed small number of generated cases (≤6), bounded per-case concurrency, a per-case turn budget and timeout, and the standard per-turn iteration cap. The trigger requires `agent.manage` and the utility LLM to be configured. | MITIGATED |
| TM-LLM-027 | llm_judge policy prompt steering via agent-authored tool content | Medium | `llm_judge` checks send tool-call arguments and tool results to the utility LLM for evaluation. This content is agent/model-authored and could attempt to override the policy prompt. Mitigated: the policy prompt is always the system message; the evaluated content is a user message with a `Content:` prefix. Tool names are XML-escaped. The judge is instructed to return a fixed JSON schema; the response parser only reads the `verdict` and `reason` fields, discarding anything else. A steered judge fails open (`allow`), never allowing more permissive behavior than the baseline. | MITIGATED |
| TM-LLM-028 | llm_judge latency amplification per tool call | Low | Each tool call can invoke up to `MAX_JUDGE_CALLS_PER_INVOCATION` (4) judge requests, each bounded by `JUDGE_TIMEOUT` (10 s). Maximum added latency per tool call: 40 s. Judge calls are serialized within a single tool invocation hook. Timeouts fail open. | MITIGATED |
| TM-LLM-029 | Guardrail `mcp` check data egress and verdict steering via external endpoint | Medium | The guardrails `mcp` check (knowledge/execution/guardrails.md) sends a bounded excerpt of tool-call arguments / tool output to an external MCP guardrail endpoint chosen by the agent operator. Egress is explicit and operator-configured (the server is a scoped-MCP reference resolved per session/org, see TM-TOOL-022 for tenant scoping), documented on the check type, and the payload is capped at 2 000 bytes (UTF-8 char-boundary safe). The endpoint returns a fixed `{"verdict":...}` JSON shape; the parser reads only `verdict`/`reason` and discards anything else, and any failure (timeout, connection/tool error, missing/unparseable verdict, server-not-configured) fails open (`allow`), a hostile or broken endpoint can never make execution more permissive than the no-guardrail baseline, only block. | MITIGATED |
| TM-LLM-030 | Guardrail `moderation` output check: assistant-text egress and latency amplification | Low | The guardrails `moderation` check (EVE-573, knowledge/execution/guardrails.md) runs on the end-of-message output boundary, sending a bounded excerpt (≤ 4 000 bytes, UTF-8-safe) of the finalized assistant message to the **org's own configured utility LLM** for classification, the same provider the agent already uses, not a new third party, so no additional cross-tenant or new-vendor egress is introduced. The classifier returns a fixed `{"scores":{...}}` shape; the parser reads only numeric category scores and ignores anything else. Latency is bounded: at most `MAX_MODERATION_CALLS_PER_INVOCATION` (4) calls per finalized message, each capped by a 10 s timeout (TM-DOS). Any failure (timeout, LLM error, unparseable response, or no utility service configured) fails open (`allow`), a classifier outage can only ever block, never make output more permissive than the no-guardrail baseline. Cost flows through the utility-LLM accounting pipeline. | MITIGATED |
| TM-LLM-031 | Provider-native compact context disclosure, cross-model replay, or concurrent history loss | High | Native compact payloads are envelope-encrypted in `session_compaction_checkpoints`, size-bounded before encryption, redacted from `Debug`, and carried only over the authenticated internal worker RPC. Public events expose metrics and an opaque checkpoint id, never payload bytes. Checkpoints are keyed by exact provider, model, and format version; incompatible requests reconstruct from raw events. A monotonic source-sequence CAS prevents an older concurrent compaction from replacing a newer boundary, while messages committed after the winning boundary remain a raw suffix. Proactive and reactive native paths share that install/apply boundary; window and cost pressure (bounded cumulative usage plus saturating raw tool-result byte counts) share the same checkpoint path and require a non-trivial current prompt. A failed install leaves the request unchanged, and an output with no material token/byte reduction is neither installed nor reported as successful. Failed and ineffective proactive calls record a bounded process-lifetime retry watermark keyed by session/provider/model, source boundary, input pressure, and transcript lineage; another call requires at least 4,096 tokens and 5% growth, bounding invisible no-op spend without crossing rollback branches. Forks copy only checkpoints whose source boundary is included in the copied event prefix. | MITIGATED |

### Mitigation Details

**TM-LLM-001, Key Retrieval Flow:**
```
Worker resolves model and provider
    → gRPC GetTurnContext returns credential-free model + provider identity
    → authenticated provider RPC resolves that exact provider under the turn org
    → control plane decrypts the provider credential
    → worker composes non-serializable runtime Provider over protocol driver
    → ProviderAuth resolves headers/signing per request attempt
    → LLM API call over HTTPS
    → provider dropped with the bounded execution context
```

Workers never have direct database access or encryption keys. Credentials do
not enter model records, internal model DTOs, events, logs, or serializable
runtime values.

**TM-LLM-007, Prompt Injection (ACCEPTED):**
Indirect prompt injection via tool results or user messages is an inherent LLM limitation. Mitigations:
- Role separation (system, user, assistant, tool_result)
- Max iteration limit prevents infinite loops
- No automatic code execution without registered tool
- Monitoring via usage tracking

**TM-LLM-021, Utility LLM Service:**
The utility LLM exists for capability internals only. It is configured from the
host environment, defaults to disabled, fixes the model in code, and reaches
capabilities through `ToolContext` rather than through agent model/provider
configuration.

## 9. Durable Execution Engine (TM-DURABLE)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-DURABLE-001 | Task hijacking | High | Ownership verified on completion; late finisher gets `TaskNotOwned` error | MITIGATED |
| TM-DURABLE-002 | gRPC unauthenticated access | High | Bearer token auth via `WORKER_GRPC_AUTH_TOKEN` (required in production); optional mTLS via `WORKER_GRPC_TLS_*` env vars; workers are cross-org by design | MITIGATED |
| TM-DURABLE-003 | Event injection | Medium | Events created via gRPC only; validated for session membership | MITIGATED |
| TM-DURABLE-004 | Queue flooding | Medium | Per-workflow pending task limit (default 100, configurable via `MAX_PENDING_TASKS_PER_WORKFLOW`) | MITIGATED |
| TM-DURABLE-005 | Heartbeat timeout manipulation | Low | 30s timeout is reasonable for LLM operations; reclaimed tasks re-queued | MITIGATED |
| TM-DURABLE-006 | Dead letter queue growth | Low | Failed tasks preserved in DLQ; no automatic cleanup | **ACCEPTED** |
| TM-DURABLE-007 | Task state manipulation | Medium | Tasks immutable after creation; only status transitions allowed via state machine | MITIGATED |
| TM-DURABLE-008 | Worker impersonation | High | Bearer token auth + optional mTLS prevents unauthorized access (see TM-DURABLE-002) | MITIGATED |
| TM-DURABLE-009 | Replay attack on workflow events | Low | Event store is append-only; events processed in sequence order | MITIGATED |
| TM-DURABLE-010 | Durable API endpoints accessible to ordinary tenant users | High | All `/v1/durable/*` endpoints require explicit platform-user auth; `ResolvedOrg -> Caller` preserves `is_platform_user` for policy/config evaluation | MITIGATED |
| TM-DURABLE-011 | Presigned image URL forgery | Medium | HMAC-SHA256 signed with `WORKER_GRPC_AUTH_TOKEN`; 5-min expiry; signature covers image_id + org_id + expires; constant-time comparison | MITIGATED |

### Mitigation Details

**TM-DURABLE-001, Task Ownership:**
```
Worker A claims task → heartbeat timeout → task reclaimed by Worker B
Worker A finishes late → CompleteDurableTask → TaskNotOwned error
Worker B continues execution → task completes correctly
```
Prevents duplicate activity execution when workers lose connectivity.

**TM-DURABLE-002, gRPC Security (MITIGATED):**
Workers authenticate to control plane gRPC (port 9001) via two layered mechanisms:

1. **Bearer token auth** (`WORKER_GRPC_AUTH_TOKEN` env var), required in production (server panics on startup if unset in non-dev mode)
   - Server: `GrpcAuthInterceptor` validates `authorization: Bearer <token>` on every request
   - Client: `GrpcClientAuth` injects the bearer token into every outgoing request
2. **Mutual TLS (mTLS)**: optional, configured via `WORKER_GRPC_TLS_*` env vars
   - Server presents its certificate (`WORKER_GRPC_TLS_CERT`/`WORKER_GRPC_TLS_KEY`) and verifies client certs against `WORKER_GRPC_TLS_CA_CERT`
   - Worker presents its client certificate and verifies the server against the CA
   - Provides encryption + mutual identity verification at the transport layer
   - Bearer token auth remains active as defense-in-depth even when mTLS is enabled

**Design decision:** Workers are intentionally cross-org. They are stateless task executors that process work from any organization's queue. Org-scoping is enforced at the application layer (HTTP API), not the internal gRPC transport.

**TM-DURABLE-010, Durable API Endpoints (MITIGATED):**
All `/v1/durable/*` HTTP endpoints require explicit platform-user auth. The auth backend returns `AuthUser.is_platform_user`, HTTP caller construction preserves it through `ResolvedOrg`, and `/v1/durable/config` exposes the same policy result for UI gating.

## 10. Scheduled Tasks (TM-SCHED)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-SCHED-001 | Malicious schedule creation / resource abuse | Medium | Only platform users can create or manage durable schedules; schedule channels enforce a minimum cron interval (`SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS`, default 300 s) and an org-level cap on enabled schedule channels (`SCHEDULE_CHANNEL_MAX_PER_ORG`, default 10). Agent-created **session schedules** (`create_schedule` / `spawn_background` with a `schedule` arg), where each fire dispatches a real worker turn, enforce the same shape worker-side at create time: per-session cap (`MAX_ACTIVE_SCHEDULES_PER_SESSION`, 5), per-org cap on active schedules (`RESOURCE_LIMIT_MAX_SESSION_SCHEDULES_PER_ORG`, default 100, so unlimited sessions cannot imply unlimited schedules per org), and a minimum recurring cron interval (`SESSION_SCHEDULE_MIN_INTERVAL_SECONDS`, default 300 s). Per-org count is `org_id`-scoped (no cross-tenant leakage); SaaS tunes the caps per plan via env. | MITIGATED |
| TM-SCHED-002 | Catch-up explosion on restart | High | `max_catch_up` limits catch-up runs (default: 1); prevents hundreds of executions on restart | MITIGATED |
| TM-SCHED-003 | Concurrent execution overload | Medium | `max_concurrent` field enforced; trigger skipped if limit reached | MITIGATED |
| TM-SCHED-004 | Invalid cron expression DoS | Low | Cron parser validates expression at creation time; invalid expressions rejected | MITIGATED |
| TM-SCHED-005 | Scheduler crash leaves tasks untriggered | Medium | Durable execution ensures tasks are created; if executor crashes, tasks auto-reclaimed via heartbeat | MITIGATED |
| TM-SCHED-006 | Embedded schedule duplication, starvation, or stranding after runner failure | Medium | `everruns::local` claims due occurrences in an immediate SQLite transaction, scopes claims by org and the host's current routable-session snapshot before ordering/limiting, heartbeats claims during host delivery, and reclaims only stale leases. Successful delivery advances or disables the stored occurrence; failure remains durable and uses the claim timeout as retry cooldown. The external `send_message` boundary is at-least-once: a process crash after host acceptance but before completion commit can retry, so embedded hosts must tolerate duplicate scheduled prompts in that narrow window. Batch size and route-filter parameters are bounded by the configured batch size and the host's active routes. | MITIGATED |

## 11. Observability (TM-OBS)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-OBS-001 | PII in Braintrust events | Medium | Full messages + LLM completions sent to Braintrust; customer responsibility to enable/disable | **CALLER RISK** |
| TM-OBS-002 | API keys in OTel spans | Medium | API keys not emitted in spans; only token counts and model names traced | MITIGATED |
| TM-OBS-003 | OTLP endpoint compromise | Medium | OTLP endpoint is configurable; must be trusted internal infrastructure | **CALLER RISK** |
| TM-OBS-004 | Braintrust API key exposure | Medium | Key stored in env var; not logged | MITIGATED |
| TM-OBS-005 | Log injection | Low | Structured logging via `tracing` crate; no raw string interpolation in log output | MITIGATED |
| TM-OBS-006 | Sensitive data in error logs | Medium | Errors logged server-side only; API keys and passwords excluded from tracing fields | MITIGATED |
| TM-OBS-007 | No security audit logging | Medium | Structured audit_logs table with fire-and-forget writes for auth events; admin-only query API | MITIGATED |
| TM-OBS-008 | Raw trajectory content leak via dataset export | High | Dataset export (`POST /v1/evals/{eval_id}/runs/{run_id}/dataset` enqueues; `GET …/dataset/{dataset_id}` fetches, `knowledge/evaluation/dataset-export.md`) exports raw model-view message content, so both endpoints are gated by the dedicated `DATASET_EXPORT` policy (`dataset.export`, requires `OrgAgentsManage` + `OrgSessionsManage`) distinct from read-only `EVAL_VIEW`/`REPORT_VIEW`; org-scoped through `get_run` (cross-org runs are 404, see TM-TENANT-001/002) and `get_dataset` re-resolves the run and verifies the handle's `eval_run_id` before returning, so a guessed dataset id from another org/run is 404 (covered by `test_dataset_export_cross_org_returns_not_found`); always-on secret scrubbing strips credential patterns from every exported string before it is persisted; optional `redact_content` blanks message/tool content while preserving structure. Phase 2 adds an at-rest store: the scrubbed NDJSON is persisted on the org-scoped `eval_run_datasets` row (never raw un-scrubbed content), and the row's FK to `eval_runs` is `ON DELETE SET NULL` so deleting the run detaches its datasets | MITIGATED |
| TM-OBS-009 | Audit-log client IP spoofing through forwarding headers | Low | Audit IP attribution uses the same trusted-proxy extractor as auth rate limiting: forwarding headers are honored only from trusted peers and `X-Forwarded-For` is peeled from the trusted right side instead of trusting the client-controlled leftmost entry. Direct untrusted peers record the socket peer address. | MITIGATED |

### Mitigation Details

**TM-OBS-001, Braintrust Data Flow:**
```
Agent turn → events emitted → BraintrustEventListener (async)
    → Convert to OpenAI format
    → POST /v1/project_logs/{project_id}/insert
    → Fire-and-forget (no retry)
```
Full conversation data (user messages, LLM responses, tool results) is transmitted. Organizations must evaluate whether Braintrust integration is appropriate given their data classification requirements.

**TM-OBS-007, Security Audit Logging (MITIGATED):**
- `audit_logs` PostgreSQL table (migration 005) stores structured events with: org_id, actor_id, event_type, ip_address, metadata, created_at.
- Event types follow `domain.action.outcome` convention: `auth.login.success`, `auth.login.failure`, `auth.register.success`, `auth.token_refresh.success`, `auth.personal_access_token.created`, `auth.personal_access_token.deleted`, `auth.oauth.success`, `auth.oauth.failure`.
- Fire-and-forget writes via `auth::audit::emit()`, audit failures never block auth operations.
- Admin-only query API: `GET /v1/organizations/:org_id/audit-logs` with filters for event_type, actor_id, cursor pagination.
- Retention: `delete_audit_logs_before()` method available for scheduled cleanup.

## 12. Web Security (TM-WEB)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-WEB-001 | XSS via stored content | Medium | React UI auto-escapes; file preview uses Shiki (no raw HTML injection) | MITIGATED |
| TM-WEB-002 | CSRF on state-changing requests | Medium | SameSite=Lax cookies; JSON content type required; no GET side effects | MITIGATED |
| TM-WEB-003 | Cookie theft via XSS | High | Refresh token cookie: HTTP-only; access token cookie: HTTP-only | MITIGATED |
| TM-WEB-004 | Clickjacking | Medium | X-Frame-Options: DENY and CSP frame-ancestors: 'none' via SetResponseHeaderLayer | MITIGATED |
| TM-WEB-005 | Missing security headers | Low | CSP, X-Content-Type-Options: nosniff, Referrer-Policy, Permissions-Policy via tower-http middleware | MITIGATED |
| TM-WEB-006 | Open redirect in OAuth flow | Medium | OAuth callbacks validated against configured redirect URIs | MITIGATED |
| TM-WEB-007 | CORS wildcard exposure | Medium | `CORS_ALLOWED_ORIGINS` not set by default; must be explicitly configured | MITIGATED |
| TM-WEB-008 | Open redirect via login page `return_to` or request-controlled login origin | Medium | `sanitizeReturnTo` (`apps/ui/src/lib/auth-redirect.ts`) restricts `return_to` to relative paths: must start with `/`, never `//` (protocol-relative), never `/\` (browser-normalized), never an absolute URL. The optional remote login destination comes only from trusted `AUTH_LOGIN_ORIGIN` configuration, is validated at server startup as an HTTP(S) origin with no credentials/path/query/fragment, and is exposed read-only via `/v1/auth/config`; no request or query value can select it. UI middleware reads only the deployment environment. Configured absolute login URLs use full-page navigation, never the client router. CLI and MCP OAuth emit only relative `return_to` paths. See `knowledge/security/authentication.md` "Login Page Contract". | MITIGATED |
| TM-WEB-A2UI-01 | XSS via `javascript:`/`data:` URL in A2UI `open_url` action or `Image.src` | High | A2UI JSON is LLM-emitted. `isSafeUrl` in `apps/ui/src/components/chat/a2ui-renderer.tsx` restricts action URLs and image sources to `http:`/`https:`/`mailto:` schemes; `window.open` also uses `noopener,noreferrer`. React auto-escapes all text props. See `knowledge/ui/a2ui.md`. | MITIGATED |
| TM-WEB-009 | XSS via SVG file preview (`<script>`, `on*` handlers, `javascript:` URLs, `<foreignObject>` HTML) | High | `SVGPreview` (`apps/ui/src/components/files/file-previews.tsx`) renders SVG inside an `<iframe sandbox="" srcDoc=...>` carrying a strict CSP meta tag (`default-src 'none'; style-src 'unsafe-inline'; img-src data:`). Empty `sandbox` denies all flags (scripts, forms, popups, top-nav, same-origin); CSP is defense-in-depth. SVG bytes are NOT sanitized server-side, the gate is the iframe boundary. `getPreviewType` routes `.svg` to this path for both `text` and `base64` encodings; no `<img src=data:image/svg+xml>` path remains. Regression tests in `apps/ui/src/__tests__/file-previews.test.tsx` exercise script, on-handler, javascript-URL, and foreignObject payloads. See EVE-389. | MITIGATED |
| TM-WEB-010 | Credential/DOM theft or forced navigation via HTML file preview (user-supplied `.html`/`.htm` that runs JS) | High | In every mode the previewed document runs in an opaque origin (iframe `sandbox` WITHOUT `allow-same-origin`): `document.cookie`, `localStorage`, and `parent.document` all throw `SecurityError` (verified), and omitting `allow-top-navigation`/`allow-forms`/`allow-popups`/`allow-modals` blocks redirects, form posts, popups, and dialogs. **Server-backed mode** (file viewer): the iframe `src` loads `GET /v1/workspaces/{id}/fs/_/preview/{path}` (`workspace_files::preview_path`, HTML-only), whose response sets `Content-Security-Policy: sandbox allow-scripts; default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' data: blob:; …; connect-src 'none'; object-src 'none'; base-uri 'none'` and `X-Frame-Options: SAMEORIGIN` (`session_files::sandboxed_html_response`). A network response does not inherit the app's `script-src 'self'`, so JS runs in the opaque sandbox; auth cookies authorize the fetch but the rendered DOM has no access to them, and the preview CSP denies network fetches so untrusted script cannot exfiltrate the previewed document to remote endpoints. **Static fallback** (`HtmlPreview` `srcDoc`, e.g. initial-files preview): the `about:srcdoc` document inherits the app's `script-src 'self'` so inline scripts do NOT run (verified under the real CSP, a child meta cannot loosen it); CSS/markup render and a hardening CSP meta (`object-src`/`base-uri`/`form-action 'none'`) is injected. Regression tests in `apps/ui/src/__tests__/file-previews.test.tsx`; manual verification under the real app CSP. | MITIGATED |
| TM-WEB-011 | Malicious PDF file preview (embedded JS, `/Launch`/`/URI` actions, mislabeled HTML) | Medium | `PdfPreview` (`apps/ui/src/components/files/file-previews.tsx`) renders via `<iframe src="data:application/pdf;base64,…">`. Chromium disables its PDF viewer inside *any* sandboxed iframe (verified for `data:` and `blob:` across sandbox flag combinations), so `sandbox` is unavailable; security comes from the `data:` URL's opaque origin (no cookie/DOM access), the out-of-process PDF viewer (PDF JS cannot script the host page), and the forced `application/pdf` type (mislabeled HTML is parsed as a broken PDF, never executed). Parent CSP carries `frame-src 'self' data:` to permit the frame. | MITIGATED |
| TM-WEB-012 | XSS via `javascript:`/`data:` URI in a citation source (`TextAnnotation.source.uri`) rendered as a link | High | Citation source URIs are LLM/retrieval-sourced (from `search_index`/`search_knowledge` tool results, `knowledge/runtime-resources/citations.md`). `isLinkableUri` in `apps/ui/src/components/chat/message-citations.tsx` renders a citation as a clickable `<a href>` only when the URI matches `^https?://`; every other scheme (`github://`, `everruns://`, or a hostile `javascript:`/`data:`) renders as inert text. `title`/`snippet`/`uri` are React text children (auto-escaped); the answer text passes through Streamdown with only internal `#cite-n` markers injected (real model links still route through `MarkdownLink`, which rejects `javascript:`). Links use `rel="noopener noreferrer"`. Mirrors TM-WEB-A2UI-01. | MITIGATED |
| TM-WEB-013 | Browser agent uses WebMCP as a confused deputy for unauthorized or unintended actions | High | WebMCP registrations exist only in the authenticated app behind deployment and org-effective gates; callbacks call the ordinary org-scoped APIs, revalidate their route/org binding at execution time, and every mutating or billable action pauses for a visible Everruns confirmation. Browser annotations are advisory only. | MITIGATED |
| TM-WEB-014 | Cross-origin frame discovers or invokes Everruns WebMCP tools | High | The UI document's Permissions Policy is `tools=(self)` when enabled and `tools=()` otherwise. Registrations never set `exposedTo`, so the browser's same-origin default prevents a cross-origin document from discovering or invoking them. | MITIGATED |
| TM-WEB-015 | Prompt injection or sensitive-data disclosure through WebMCP search/context results | High | Search is lazy and org-scoped, caps result count and output size, omits descriptions/prompts/transcripts/credentials, and marks returned content with `untrustedContentHint`. Context exposes only current navigation/resource metadata. | MITIGATED |
| TM-WEB-016 | Stale WebMCP callback acts after route, resource, org, or auth change | High | Every registration is tied to an `AbortSignal`; route/resource/org/auth changes abort it, pending confirmations are rejected, and callbacks compare their captured binding with current state before calling an API. | MITIGATED |
| TM-WEB-017 | Browser retries duplicate a non-idempotent WebMCP mutation | Medium | Mutation annotations declare non-idempotence, every invocation requires confirmation, and the UI rejects concurrent execution while an action or approval is pending. Ambiguous sequential network retries remain a residual risk of backend endpoints without idempotency keys. | MITIGATED (partial) |

### Mitigation Details

**TM-WEB-004 / TM-WEB-005, Security Headers (MITIGATED):**
Applied via `SetResponseHeaderLayer` (`if_not_present`) in `app_builder.rs`:
- `X-Frame-Options: DENY`, prevents clickjacking
- `X-Content-Type-Options: nosniff`, prevents MIME sniffing
- `Referrer-Policy: strict-origin-when-cross-origin`, limits referrer leakage
- `Permissions-Policy` disables unused browser device APIs. Default: `camera=(), microphone=(), geolocation=()`. When the `voice` feature flag is enabled, microphone is narrowed to same-origin use: `camera=(), microphone=(self), geolocation=()`.
- `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; frame-src 'self' data:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'` (`frame-src` permits the PDF preview `data:` iframe, TM-WEB-011, while keeping `about:srcdoc` previews under `'self'`)

## 13. AI Agent Behavior (TM-AGENT)

The agent loop is a core trust boundary: an LLM decides which tools to call with what arguments. The system prompt, user messages, tool results, and MCP tool descriptions all influence LLM behavior. Agents are semi-trusted within organizational scope, the agent creator (org member) is trusted, but the LLM's runtime decisions are not fully controllable.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-AGENT-001 | Direct prompt injection via user message | High | Role separation (user vs system); LLM providers apply safety training; no complete defense | **ACCEPTED** |
| TM-AGENT-002 | Indirect prompt injection via tool results | High | Tool results use `tool_result` role, not `system`; LLM may still follow adversarial instructions in results | **ACCEPTED** |
| TM-AGENT-003 | Indirect prompt injection via MCP tool descriptions | Medium | MCP tool names/descriptions fed to LLM as tool schema; adversarial descriptions could influence behavior | **ACCEPTED** |
| TM-AGENT-004 | Agent jailbreak via system prompt | Medium | System prompt set by org member at agent creation; no sanitization of prompt content | **BY DESIGN** |
| TM-AGENT-005 | Capability escalation via agent creation | High | RiskLevel enum on Capability trait; high-risk capabilities (`a2a_agent_delegation`, `docker_container`, `daytona`, `e2b`, `deno`, `bashkit_shell`, `web_fetch`, `model_scout`) require Admin role to assign via API; gate is at create/update only, member-owned agents that already had high-risk capabilities are grandfathered (see `knowledge/execution/capabilities.md` "Admin-Only Tier Decision") | MITIGATED |
| TM-AGENT-006 | Cost runaway, unbounded LLM calls | High | Max iterations per turn (default 100); configurable per agent | MITIGATED |
| TM-AGENT-007 | Cost runaway, many tools per iteration | Medium | No per-iteration tool call limit; agent can invoke many tools in a single LLM response | **OPEN** |
| TM-AGENT-008 | Context window poisoning | Medium | Auto-compaction via `llm_driver.compact()` on `RequestTooLarge`; older messages compressed | MITIGATED |
| TM-AGENT-009 | Agent self-modification | Medium | Agents with `platform` or legacy `platform_management` can modify agents/sessions via tools; each capability must be explicitly assigned and is org-scoped | **OPEN** |
| TM-AGENT-010 | Agent spawning agent chains | Medium | Agents with `platform` or legacy `platform_management` can create agents/sessions; each capability must be explicitly assigned; no recursive depth limit | **OPEN** |
| TM-AGENT-011 | Sensitive data in system prompt | Medium | PII must not be placed in system prompts; no encryption at rest for prompts | **OPEN** |
| TM-AGENT-012 | Tool result size amplification | Medium | 64 KiB hard limit on tool results via `OutputHardLimitHook` (EVE-225); always-on final hook in ActAtom | MITIGATED |
| TM-AGENT-013 | Exfiltration via web_fetch | Medium | Agent with web_fetch capability can send session data to arbitrary URLs | **ACCEPTED** |
| TM-AGENT-014 | Confused deputy, tool call with wrong session | Low | Tool context includes session_id; tools scoped to active session only | MITIGATED |
| TM-AGENT-015 | Dangling tool calls cause LLM confusion | Low | Patched with synthetic "cancelled" results before LLM call; prevents API errors | MITIGATED |
| TM-AGENT-016 | Plaintext secrets in chat history | Medium | When agent asks user for API key in chat, plaintext value stored in events table as message content; session secrets encrypt separately but chat retains plaintext | **OPEN** |
| TM-AGENT-017 | Agent-initiated entity management | High | `platform` exposes the full scriptable command catalog and legacy `platform_management` exposes a smaller handwritten subset. Both are high-risk and explicitly assigned. Each call uses the session owner's real caller and active permission resolver; only that owner may submit Platform Chat turns or discover/execute its context-aware slash commands, and the distributed adapter reloads the org-scoped session without accepting a worker-supplied user identity. No fine-grained ownership RBAC exists within commands the caller is otherwise allowed to use. | **OPEN** |
| TM-AGENT-018 | Outbound URL filtering on web_fetch | Medium | Per-layer `NetworkAccessList` (harness ∩ agent ∩ session, narrow-only merge) plus optional deployment-wide system allowlist, both enforced at the `EgressService` boundary; web_fetch routes through egress with per-redirect-hop re-validation | MITIGATED |
| TM-AGENT-019 | Internal network probing via high-risk execution capabilities | High | `daytona` and `e2b` provide full network access by design; `docker_container` uses host networking in dev mode; all rely on Admin-only assignment plus infrastructure egress isolation | **ACCEPTED** |
| TM-AGENT-020 | Cross-session resource reuse via stale or guessed external IDs | Critical | Provider-owned resource IDs are checked against the active session's leased-resource/session-resource ownership before tool execution; raw sandbox list endpoints are filtered to owned IDs only | MITIGATED |
| TM-AGENT-021 | System prompt regurgitation | Medium | Opt-in `prompt_canary_guardrail` capability runs a streaming output guardrail that replaces the assistant message when the first sentence of the system prompt appears verbatim in the model output; original tokens are dropped and never persisted. Catches verbatim leaks only, paraphrased or partial leaks pass through. See `knowledge/execution/capabilities.md` § Output Guardrails | MITIGATED (partial, opt-in) |
| TM-AGENT-022 | Agent-initiated machine-payment spend | High | Paid capabilities cannot directly sign or submit arbitrary paid HTTP requests. They call `PaymentAuthority`, which selects an active policy matching the session/agent/agent identity/user/org, capability, target host, rail, and per-request limit before signing; attempts are audited | MITIGATED |
| TM-AGENT-023 | Redirect-based payment host-allowlist bypass | High | The payment authority validates only the original request URL host against the active policy's `allowed_hosts`. The outbound `reqwest` client used by `ServerPaymentAuthority::send_http_request` disables redirect following (`redirect::Policy::none()`), so 30x responses cannot be used to redirect a paid request to an unvalidated/internal host or downgrade from HTTPS | MITIGATED |
| TM-AGENT-024 | A2A outbound delegation egress / SSRF bypass | High | Outbound A2A delegation respects the merged `ToolContext.network_access` ACL. `enforce_network_access` (in `crates/platform/src/capabilities/a2a_delegation.rs`) validates the configured `base_url` and every resolved `AgentCard` interface URL against the runtime ACL before the A2A client is built; `submit_run`, `wait_for_run`, and `cancel_task` all flow through this gate, so configured or AgentCard-discovered endpoints cannot bypass egress controls | MITIGATED |
| TM-AGENT-027 | Untrusted A2A structured artifact bypasses the parent result contract | Medium | Schema-bound external delegation takes only the first terminal A2A data part, validates it with the same delegation-result validator used for local child reports, and persists it only after conformance. Missing data fails `no_result`; invalid data fails `schema_mismatch`; neither is exposed as a successful task result. `message_schema` is rejected because remote agents cannot receive the local progress-reporting tool. | MITIGATED |
| TM-AGENT-026 | Exfiltration / web reach via OpenRouter provider-executed server tools | High | The opt-in `openrouter_server_tools` capability (`integrations/openrouter-workspace/src/server_tools.rs`) lets the model invoke OpenRouter server tools (`web_search`, `web_fetch`, etc.). OpenRouter executes them **provider-side**, so Everruns' `NetworkAccessList`/`EgressService` egress controls (TM-AGENT-018) do **not** apply, same exfil class as TM-AGENT-013 but the egress boundary is OpenRouter's, not ours. Mitigations: capability is `RiskLevel::High` (admin-only assignment) and must be explicitly enabled per agent; only known server tools serialize (closed enum, unknown names dropped); no Everruns infra reach, secrets, or local execution are exposed. Residual cross-provider egress is the operator's OpenRouter-account responsibility | **ACCEPTED** |
| TM-AGENT-028 | Model-triggered detached session creation exceeds caller authority | High | The model can request detached spawning but cannot provide an authorization identity or budget root. `ToolContext` receives both from the host: the resolved session owner must satisfy `SESSION_MANAGE`, and the returned root is org-validated control-plane metadata. Authorization runs before session creation. | MITIGATED |

### Mitigation Details

**TM-AGENT-001 / TM-AGENT-002, Prompt Injection (ACCEPTED):**
Prompt injection is an inherent limitation of current LLM architecture. Defense-in-depth:
1. **Role separation:** System, user, assistant, tool_result messages are distinct roles
2. **Iteration limits:** Max turns prevents infinite manipulation loops
3. **Tool registry:** LLM can only call registered tools (no arbitrary code execution)
4. **Session isolation:** Even if manipulated, agent is confined to its session
5. **No auto-escalation:** Agent cannot grant itself new capabilities
6. **Instruction hierarchy:** Generic harness system prompt includes an explicit instruction hierarchy statement directing the LLM to prioritize system instructions over content in tool results, user messages, or agent instructions files

There is no reliable way to prevent an LLM from following adversarial instructions embedded in tool results or user messages. This is an industry-wide limitation.

**TM-AGENT-004, System Prompt Trust Model:**
```
Agent creator (org member) → sets system_prompt → stored in agents table
    ↓
Session created → system_prompt loaded (immutable for session)
    ↓
Capability prompts appended (hardcoded in Rust, not user-controlled)
    ↓
Combined prompt sent to LLM as system message
```
The agent creator is trusted within their org. A malicious system prompt can instruct the agent to misuse its capabilities, but only within the sandbox (session files, SQLite, bash sandbox). The blast radius is limited to the session.

**TM-AGENT-005, Capability Escalation (MITIGATED):**
Each capability declares a `RiskLevel` (Low, Medium, High) via the `Capability` trait. High-risk capabilities (`docker_container`, `daytona`, `e2b`) require `OrgRole::Admin` to assign. The check runs in create/update/upsert/import agent API handlers, returning 403 if a non-admin user attempts to assign a high-risk capability. The `risk_level` field is exposed in the capabilities list API for UI display.

**TM-AGENT-006, Iteration Limit:**
```rust
// Turn state machine enforces max iterations
if self.current_iteration >= self.max_iterations {
    TurnPlan::Terminal {
        stop_reason: TurnStopReason::MaxIterationsReached,
        ...
    }
}
```
Default: 100 iterations. Each Reason→Act cycle counts as one iteration. Configurable per agent.

**TM-AGENT-008, Context Window Poisoning (MITIGATED):**
When the message history exceeds the LLM's context window, the `ReasonAtom` catches `RequestTooLarge` errors and calls `llm_driver.compact()` to compress older messages. This prevents unbounded context growth. Adversarial early messages are still present but may be summarized during compaction. Native encrypted compact output remains typed provider transport state: it is reused only on the matching provider request, its `Debug` representation redacts payloads, and public compaction/generation events expose metadata rather than encrypted content.

**TM-AGENT-013, Exfiltration via web_fetch (ACCEPTED):**
An agent with `web_fetch` capability can:
1. Read session files via `read_file` tool
2. Send file contents to external URL via `web_fetch` tool

This is accepted because:
- Agent capabilities are chosen by org members (trusted)
- `web_fetch` is an opt-in capability, not default
- The intended use case requires external HTTP access
- Removing this would break legitimate functionality

**TM-AGENT-016, Plaintext Secrets in Chat History (OPEN):**
When an agent tool (e.g., Daytona) doesn't find an API key, it may instruct the user to provide one in chat. The user types the key as a chat message, which is stored as plaintext in the `events` table (message content). The `session_secrets` table encrypts the value separately, but the original chat message retains plaintext indefinitely.

- **Impact:** API keys visible in session history, event exports, and any observability pipeline that captures events.
- **Recommendation:** Prefer Settings UI for credential entry (user connections). Phase out in-chat secret collection. For tools that need credentials, guide users to Settings > Connections instead of requesting secrets in chat.
- **Priority:** High

**TM-AGENT-017, Agent-Initiated Entity Management (OPEN):**
Agents with the `platform` capability can invoke every command exposed to the
scripted platform catalog; the legacy `platform_management` capability exposes
a smaller handwritten set. Depending on caller permissions, this includes
creating, updating, and deleting platform entities and interacting with
sessions.

- **Impact:** An agent could escalate privileges by creating a new agent with dangerous capabilities, modify other agents' system prompts, or spawn session chains. No fine-grained RBAC exists within the org scope.
- **Current mitigations:** (1) Capability is high-risk and must be explicitly assigned by an authorized org member. (2) All operations are org-scoped, cross-org access is blocked by tenant isolation (TM-TENANT-001), and Platform tool schemas reject `organization_id`. (3) Platform execution resolves the owning session's user into a real `Caller` and evaluates every registered command through `Command::run` with the active `PermissionResolver`, so member-owned Platform Chat sessions do not inherit internal/owner bypass. Message creation and slash-command discovery/execution independently require a non-internal caller to be that Platform Chat session's resolved owner, preventing another member with org-wide session management permission from driving its command authority or reading context through `/btw`. (4) The in-process adapter uses its owner-bound command context. The dedicated gRPC command-surface RPC reloads the session from the requested org and resolves its persisted owner server-side; it never accepts a worker-supplied user identity. (5) Bashkit bounds commands, loops, input, AST depth, and timeout; read-only `query` omits mutating and open-world commands. (6) Current-user connection reads derive the user only from that resolved caller and return a secret-free projection; provider discovery is org-scoped and omits OAuth registration details, tokens, credentials, scopes, and provider metadata. (7) Internal command details are logged but redacted from tool errors.
- **Recommendation:** Add audit logging for all platform management tool calls. Consider RBAC (e.g., "can only manage own sessions") and approval workflows for dangerous operations (creating agents with `bashkit_shell`). Add recursion depth limits for agent-spawned session chains.
- **Code:** `// THREAT[TM-AGENT-017]` at the Platform capability registration, direct adapter, and worker RPC authorization boundary.
- **Priority:** High

**TM-AGENT-018, Outbound URL Filtering on web_fetch (MITIGATED):**
An agent influenced by prompt injection (via tool results or user messages) could chain data access tools with `web_fetch` to exfiltrate sensitive session data. While TM-AGENT-013 accepts this risk for legitimate use by trusted org members, prompt injection (TM-AGENT-001, TM-AGENT-002) can cause the agent to act against the user's intent.

- **Attack chain:** Injected instruction in tool result → agent reads sensitive file → agent calls `web_fetch` with file contents to attacker-controlled URL
- **Current mitigations:** (1) Per-layer `NetworkAccessList` (allowed/blocked patterns) on harness, agent, and session, merged narrow-only (intersection on `allowed`, union on `blocked`), see `specs/network-access.md`; configurable via API and the agent/harness edit UI. (2) Optional deployment-wide system allowlist of curated public hosts, AND-ed as a hard ceiling, see `specs/system-allowlist.md`. (3) Both are enforced at the `EgressService` boundary; `web_fetch` routes through egress (`integrations/web-fetch/src/egress_transport.rs`) with the list re-checked on every redirect and crawl request. Direct-transport crawl is rejected while either policy is active because discovered pages cannot be re-checked there.
- **Current mitigations:** (1) Per-layer `NetworkAccessList` (allowed/blocked patterns) on harness, agent, and session, merged narrow-only (intersection on `allowed`, union on `blocked`), see `knowledge/operations/network-access.md`; configurable via API and the agent/harness edit UI. (2) Optional deployment-wide system allowlist of curated public hosts, AND-ed as a hard ceiling, see `knowledge/operations/system-allowlist.md`. (3) Both are enforced at the `EgressService` boundary; `web_fetch` routes through egress (`integrations/web-fetch/src/egress_transport.rs`) with the list re-checked on every redirect hop.
- **Residual risk:** Defaults are open, with no `NetworkAccessList` configured and the system allowlist disabled, outbound destinations are unrestricted (TM-AGENT-013 ACCEPTED). Outbound calls are not yet audit-logged with URL + payload size.
- **Complements:** SSRF protection blocks private IPs with DNS pinning on the egress path (`validate_url_dns_pinned`, TM-API-008/TM-TOOL-018).
- **Priority:** Medium

**TM-AGENT-019, Internal Network Probing via High-Risk Execution Capabilities (ACCEPTED):**
Some execution capabilities intentionally originate network traffic outside the worker process:
- `daytona` sandboxes have full Linux and network access by design
- `e2b` sandboxes have full Linux and network access by design
- `docker_container` uses host networking and is experimental/dev-only

This means an agent with one of these capabilities can probe whatever network the sandbox/container can reach. Current mitigations are:
- Admin-only assignment for high-risk capabilities (TM-AGENT-005)
- `docker_container` is gated to development-grade deployments

Residual risk remains with the deployment topology. Production operators must enforce egress filtering and network segmentation for any execution environment that can reach internal services.

**TM-AGENT-020, Cross-Session Resource Reuse (MITIGATED):**
Tools that accept provider-owned external IDs (`sandbox_id`, raw Daytona toolbox paths, and
similar handles) resolve ownership through the active session's leased resources before calling
the backend. The session resource registry carries the same external-ID metadata for runtimes
that only expose the generic registry. Raw sandbox list calls are filtered to the IDs owned by
the active session before results are returned to the agent.

**TM-AGENT-022, Agent-Initiated Machine-Payment Spend (MITIGATED):**
Agents can invoke paid capabilities such as Parallel search/extract/task, but V1 deliberately has
no generic `paid_http_request` tool. The capability submits a typed payment request to
`PaymentAuthority`; the server selects only active spend policies scoped to the current session,
agent, agent identity, user, or organization and checks capability allowlist, host allowlist, rail
preference, and per-request maximum before creating any rail-specific signature. Every attempt is
persisted with status, amount, target URL, and receipt/error. Registration of any money-spending
capability is additionally gated by the `machine_payments` feature flag
(`FEATURE_MACHINE_PAYMENTS`), off by default on every grade, so spend tools are never offered
unless deliberately enabled. The same deployment gate removes wallet-management UI and leaves
payment account, policy, and attempt API paths unmounted, preventing custody without spend.

## 14. Voice Sessions (TM-VOICE)

Voice Sessions add browser microphone capture and provider realtime sessions.
See [voice.md](../operations/voice.md) for the feature contract.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-VOICE-001 | Standard OpenAI API key exposed to browser | Critical | Voice endpoints mint short-lived client secrets or proxy SDP server-side; standard provider API keys never leave the backend | MITIGATED |
| TM-VOICE-002 | Browser-supplied safety identifier impersonates another user | Medium | Server derives and sets `OpenAI-Safety-Identifier`; browser values are ignored | MITIGATED |
| TM-VOICE-003 | Sideband tool execution bypasses Everruns authorization | Critical | Sideband tool calls execute through the normal capability path using the session owner's caller context, permission resolver, audit logging, and org scoping | MITIGATED |
| TM-VOICE-004 | Raw audio retained without consent | High | V1 stores transcript text only; raw user/model audio, SDP bodies, and raw provider events are not persisted | MITIGATED |
| TM-VOICE-005 | Sensitive spoken content leaks through transcripts/logs | High | Voice transcripts are treated as normal chat messages for retention/export/observability; logs must not include raw SDP, client secrets, or unsanitized sideband payloads | OPEN |
| TM-VOICE-006 | Voice model performs write action after mishearing exact identifiers | High | Voice prompts require clarification for unclear audio and confirmation for high-precision identifiers and write/dangerous actions before tool calls | MITIGATED |
| TM-VOICE-007 | Long-running voice session causes cost runaway | Medium | Voice Connections are leased session resources with expiry, explicit end endpoint, cleanup, and future budget enforcement using usage metadata | OPEN |

## 15. Bash Sandbox (TM-BASH)

Everruns uses [bashkit](https://github.com/everruns/bashkit) (v0.2.1) as a sandboxed bash interpreter for the `bashkit_shell` capability. Bashkit provides WASM-like isolation: no real filesystem, no network, no system calls. The session file store is bridged via the `SessionFileSystemAdapter`.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-BASH-001 | Workspace boundary escape | Critical | Bash paths resolve through `MountFs` into the **session-scoped** session file store. The cwd is the stable `/workspace` presentation root; presentation is not the boundary because the root mount makes any path addressable (EVE-660). The actual boundary holds regardless: the server backend is a VFS with no real filesystem (TM-BASH-002) and is keyed per session/workspace, and each real-disk mount (`RealDiskFileStore`) clamps paths under its registered root with symlink rejection. `/etc/passwd` resolves to a key *inside this session's* store (server) or `<root>/etc/passwd` contained under a registered root (real-disk), never the host file or another tenant. | MITIGATED |
| TM-BASH-002 | Read host /etc/passwd or system files | Critical | No real filesystem; all I/O goes through the session-scoped store (server VFS) or root-clamped `RealDiskFileStore` mounts (embedded). This, not `/workspace` prefixing, is the primary boundary. | MITIGATED |
| TM-BASH-003 | Network access from bash | Critical | Off by default: without per-capability config `{"enable_http": true}` the interpreter has no network path (curl/wget fail with no socket opened). When enabled, bashkit is built with `NetworkAllowlist::allow_all()` + an egress-backed transport (`integrations/bashkit/src/egress_transport.rs`): bashkit keeps its SSRF precheck (private-IP blocking, resolve-then-check pinned into `EgressRequest.pinned_addrs`, TM-TOOL-018) and every hop, including curl/wget manual redirects, crosses `EgressService`, where the merged `NetworkAccessList` and system allowlist are enforced. No direct-dial fallback: absent an egress service the shell stays offline. Config sits on the admin-gated `bashkit_shell` capability (TM-AGENT-005 tier). Egress denials surface as curl exit 7, size-cap violations as exit 63 (tested in `bashkit_shell::tests::http_tests`) | MITIGATED |
| TM-BASH-004 | Fork bomb / process spawning | Critical | No real process execution; `exec`, subprocesses, background processes not implemented (exit 127) | MITIGATED |
| TM-BASH-005 | Infinite loop CPU exhaustion | High | `max_loop_iterations: 10000`; `max_commands: 1000`; parser timeout 5s | MITIGATED |
| TM-BASH-006 | Deep recursion stack overflow | High | `max_function_depth: 100`; `max_ast_depth: 100` | MITIGATED |
| TM-BASH-007 | Large script input DoS | High | `max_input_bytes: 1_000_000` (1 MB) | MITIGATED |
| TM-BASH-008 | Execution timeout | High | Default 30s, max 60s; enforced by tool executor | MITIGATED |
| TM-BASH-009 | Environment variable leak | Medium | Controlled env: only HOME, SHELL, PATH, WORKSPACE; hardcoded username/hostname ("everruns") | MITIGATED |
| TM-BASH-010 | Symlink escape | Medium | `SessionFileSystemAdapter.symlink()` returns `Error (unsupported)` | MITIGATED |
| TM-BASH-011 | Path traversal via bash | High | `MountFs` collapses `.`/`..` (a leading `..` is clamped at root), rejects `..` beneath additional-root mount prefixes before routing, and the real-disk backend additionally rejects `..` and symlinks and re-checks containment under each mounted root. Traversal cannot escape the session store / registered workspace roots. | MITIGATED |
| TM-BASH-012 | Privilege escalation (sudo, su) | Low | No privilege commands implemented; sandboxed interpreter only | MITIGATED |
| TM-BASH-013 | eval/bash re-invocation escape | Medium | `eval` and `bash`/`sh` commands re-invoke the sandboxed interpreter, not real shell | MITIGATED |
| TM-BASH-014 | File permission bypass | Low | `chmod` is a no-op; session filesystem has no permission model | **BY DESIGN** |
| TM-BASH-015 | Host information disclosure | Low | `hostname` → "everruns"; `whoami` → "everruns"; `uname` returns sandboxed values; mounted real-disk workspaces keep bash cwd/WORKSPACE in `/workspace` rather than the host checkout path | MITIGATED |
| TM-BASH-016 | Write amplification via bash | Medium | Per-session and per-file byte quotas enforced in `DirectWorkerAdapters::write_file` (see TM-FS-008) | MITIGATED |
| TM-BASH-017 | Timestamp spoofing via `touch -t` | Low | `SessionFileSystemAdapter.set_modified_time()` is a no-op, mirroring `chmod` (TM-BASH-014); the session store persists no mtimes and `stat` synthesizes them, so bash cannot backdate a file to influence anything that reads timestamps | **BY DESIGN** |

### Mitigation Details

**TM-BASH-001 / TM-BASH-011, Workspace Boundary:**
```text
bash path ──> SessionFileSystemAdapter ──> MountFs.resolve ──> backend
  /workspace/foo │ /foo │ ../x │ /etc/passwd   (all addressable)
                                   │
   server VFS: key in THIS session's store (session/workspace-scoped) — no host, no other tenant
   real-disk : <workspace-root>/etc/passwd, clamped + symlink-rejected — never host /etc/passwd
```
All bashkit filesystem operations go through `SessionFileSystemAdapter`, which hands the path to `MountFs` (the sole resolver). The cwd is the stable `/workspace` display root and the root mount makes any path addressable (EVE-660); the boundary is **not** a displayed prefix but the backend, the server VFS has no real filesystem and is per-session/workspace scoped, and the real-disk backend clamps every resolved path under the workspace root with symlink rejection. So a path outside the displayed root resolves *into the session's own store*, never to an unregistered host path or another tenant.

**TM-BASH-005, Resource Limits:**
```rust
ExecutionLimits::new()
    .max_commands(1000)
    .max_loop_iterations(10000)
    .max_function_depth(100)
    .max_input_bytes(1_000_000)
    .max_ast_depth(100)
    .parser_timeout(Duration::from_secs(5))
```

**TM-BASH-009, Controlled Environment:**
```rust
BashkitTool::builder()
    .username("everruns")
    .hostname("everruns")
    .env("HOME", "/home/agent")
    .env("SHELL", "/bin/bash")
    .env("PATH", "/usr/local/bin:/usr/bin:/bin")
    .env("WORKSPACE", "/workspace")
    .build()
```
No host environment variables leaked. Username and hostname are hardcoded sandbox values.

**TM-BASH-013, Sandboxed Re-invocation:**
When a bash script calls `bash` or `sh` or uses `eval`, bashkit re-invokes its own sandboxed interpreter rather than spawning a real shell process. All execution limits and filesystem isolation are preserved across re-invocations.

### Bashkit Isolation Summary

| Property | Status |
|----------|--------|
| Real filesystem access | Blocked (VFS adapter only) |
| Real process execution | Blocked (exit 127) |
| Network access | Blocked (no builtins) |
| Host environment leak | Blocked (controlled env) |
| Host info disclosure | Blocked (hardcoded values) |
| Symlink following | Blocked (unsupported) |
| Privilege escalation | Blocked (no sudo/su) |
| Resource exhaustion | Limited (commands, loops, depth, timeout) |

## 15A. Lua Sandbox (TM-LUA)

Experimental sandboxed Lua execution capability (`integrations/lua/src/lib.rs`,
`knowledge/execution/lua-execution.md`). Engine: **mlua** (vendored Lua 5.4, never LuaJIT),
linked only by the opt-in Framework/host `lua` feature. High risk, admin-gated (same gates as
`bashkit_shell`), and runtime-gated by `FEATURE_LUA`. One fresh VM per invocation,
never shared across sessions/tenants. All hardening is on by default, no
configuration knobs.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-LUA-001 | Arbitrary code execution | High | Admin-gated assignment (High risk tier); only string/table/math/os/utf8 libs loaded; dangerous globals scrubbed | MITIGATED |
| TM-LUA-002 | CPU / wall-clock exhaustion | High | Instruction-count hook (every 100k ops) enforces an instruction budget + wall-clock deadline; outer tokio timeout backstop. The VM runs on a dedicated blocking thread, so a pathological *synchronous* op (e.g. catastrophic Lua pattern in C, which the hook cannot interrupt) occupies one blocking-pool thread instead of stalling a shared runtime worker. **Residual:** such an op is not force-killable in-process, robust fix is out-of-process execution. | MITIGATED (best-effort for synchronous C ops) |
| TM-LUA-003 | Memory exhaustion | High | `Lua::set_memory_limit` hard 32 MiB cap (over-budget alloc → Lua error). Host-side reads bounded by `SessionFileSystem` quotas (TM-FS-008). | MITIGATED |
| TM-LUA-004 | Filesystem escape / cross-tenant access | High | All paths route through `LuaVfs` → the **session-scoped** `SessionFileSystem` (a `MountFs`). The session scope is the tenant boundary, not the `/workspace` prefix: `MountFs` rejects `..` traversal under additional-root mount prefixes and each real-disk backend clamps under its registered root with symlink rejection. `/workspace` is the default cwd; addressing outside it stays within this session's own store (consistent with bash, TM-BASH-001). `io` library not loaded. | MITIGATED |
| TM-LUA-005 | Network egress / SSRF / exfiltration | High | No socket library. `http.get/post` is **fail-closed**: routed only through the host `EgressService` (the central egress boundary) AND requires a non-empty `network_access` allow-list that permits the URL, checked before the request. Absent either, `http.*` is not even defined. Response bodies capped at 1 MiB. | MITIGATED (allow-listed egress) |
| TM-LUA-006 | Dynamic code / bytecode loading | Medium | `load`/`loadstring`/`dofile`/`loadfile`/`require`/`package` scrubbed to nil; `string.dump` removed; no untrusted-bytecode path | MITIGATED |
| TM-LUA-007 | Native escape (FFI / C modules) | High | Lua 5.4, never LuaJIT (no FFI); `package`/`require` scrubbed so `package.loadlib` cannot `dlopen` a shared object; `debug` library not loaded | MITIGATED |
| TM-LUA-008 | Output-channel abuse | Medium | Captured `print` output capped (64 KiB) in-engine; tool result further shaped via `tool_output_sanitizer` | MITIGATED |
| TM-LUA-009 | Code-mode tool re-entry / privilege escalation | High | `tools.<name>` exposes only `Auto`-policy, non-destructive, non-`cpu_bound` sibling tools; approval/client-side tools and the execution tools (`lua`/`bash`) are excluded. The child `ToolContext` has `tool_registry = None`, so a code-mode tool cannot itself open code mode (no recursion). Each call runs under the same session/org scope. | MITIGATED |

### Lua Isolation Summary

| Property | Status |
|----------|--------|
| Real filesystem access | Blocked (LuaVfs → session store only; no `io`) |
| Real process execution | Blocked (`os.execute`/`os.exit` scrubbed) |
| Environment variables | Blocked (`os.getenv` scrubbed) |
| Network / sockets | No raw sockets; `http.*` fail-closed via host EgressService + allow-list |
| Code mode (tools) | `tools.<name>` limited to Auto/non-destructive/non-exec tools; no recursion |
| Dynamic code / bytecode | Blocked (`load`/`require`/`string.dump` scrubbed) |
| Native / FFI / C modules | Blocked (no LuaJIT, no `package.loadlib`, no `debug`) |
| Memory exhaustion | Bounded (`set_memory_limit` 32 MiB) |
| CPU / wall-clock | Bounded for Lua code (hook + deadline); synchronous C ops contained to a blocking thread, best-effort timeout |
| Cross-tenant state | Blocked (fresh VM per invocation) |

## 16. Denial of Service (TM-DOS)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-DOS-001 | Large API request body | High | Input size limits on all fields; multipart upload capped at 101 MB | MITIGATED |
| TM-DOS-002 | Agent loop infinite iteration | High | Max 10 iterations per turn; configurable | MITIGATED |
| TM-DOS-003 | SSE connection exhaustion | Medium | Global (10k), per-org (1k), per-session (5) RAII connection limits via `SseConnectionTracker`; HTTP/2 flow control windows tuned (2 MB/stream, 16 MB/connection) with adaptive sizing; connection cycling with ±20% jitter prevents thundering herd; HTTP/2 PING keepalive detects dead connections | MITIGATED |
| TM-DOS-004 | Database connection pool exhaustion | Medium | sqlx connection pool with max_connections; timeouts on acquisition | MITIGATED |
| TM-DOS-005 | Session file storage abuse | Medium | Per-session and per-file byte quotas enforced at the application layer (see TM-FS-008) | MITIGATED |
| TM-DOS-006 | Durable task queue flooding | Medium | Per-workflow pending task limit (see TM-DURABLE-004) | MITIGATED |
| TM-DOS-007 | Nested JSON depth in API input | Medium | Input validation rejects deeply nested structures | MITIGATED |
| TM-DOS-008 | ReDoS via file grep endpoint | Medium | Regex pattern and path_pattern length capped (1 000 chars); NFA size capped via `RegexBuilder::size_limit` (512 KB); storage backends skip files > 512 KB before scanning; total request scan aborted above 5 MB | MITIGATED |
| TM-DOS-009 | Valkey unauthenticated access | Medium | Valkey listens on localhost:6379 by default; no AUTH configured in local/example compose | **CALLER RISK** |
| TM-DOS-010 | AG-UI SSE connection exhaustion | Medium | AG-UI app streams reuse the shared `SseConnectionTracker`, enforcing the same global/per-org/per-session limits as other SSE endpoints. App owners can also configure a per-app, per-IP request cap via `AgUiChannelConfig.rate_limit_per_minute`: in-memory backend is a per-minute governor quota; when `VALKEY_URL` is set it becomes a Valkey sliding-window counter shared across instances and fail-closed on Valkey errors | MITIGATED |
| TM-DOS-010 | Rate limit bypass via Valkey failure | Low | Fail-open design: if Valkey is down, requests are allowed without rate limiting | **ACCEPTED** |
| TM-DOS-011 | Authenticated personal access token sprawl | Low | Per-user cap enforced at creation (`max_personal_access_tokens_per_user`, default 25, via `RESOURCE_LIMIT_MAX_PERSONAL_ACCESS_TOKENS_PER_USER`); creation requires an authenticated user session and tokens remain user-owned/revocable. Operators tune the cap or clean up excessive tokens if they need stricter controls. | **ACCEPTED** |
| TM-DOS-012 | Source-backed Memory repository storage abuse | Medium | Memory source sync performs shallow clones without tags, skips symlinks, excludes `.git`, and enforces configurable file-count, per-file byte, and total byte limits before replacing `memory_files`. Failed sync keeps the previous readable snapshot. | MITIGATED |
| TM-DOS-013 | Hidden Agent snapshot storage growth | Medium | Automatic Agent draft snapshots are active only when `FEATURE_AGENT_VERSIONS` is enabled, skipped when the latest stored config hash already matches the draft, and pruned to a bounded per-Agent unpublished auto-snapshot window after each write. | MITIGATED |
| TM-DOS-014 | Tool output context growth | Medium | Read-like tools use windowed responses and truncation envelopes (`read_file`, `list_directory`, `grep_files`, browser DOM content); `grep_files` caps before/after context at 20 lines per side, paginates matches before expansion, merges overlapping windows, and caps returned text at 64 KiB; platform message reads cap message count and per-message content; non-image binary reads return metadata instead of base64 or lossy UTF-8; opted-in exec tools persist full output under `/outputs/` so the inline prompt payload can stay bounded and recoverable. | MITIGATED |
| TM-DOS-015 | Unbounded tool fan-out within an act batch | Medium | A single model turn can request an arbitrary number of tool calls; `ActAtom` previously executed them all concurrently with no bound. The engine `tool_scheduler` (`crates/engine/src/execution/tool_scheduler.rs`) caps simultaneously-executing calls with a semaphore (default 32, `EVERRUNS_ACT_MAX_TOOL_CONCURRENCY`), serializes same-`concurrency_class` mutations, and offloads `cpu_bound` tools to their own task so an in-process interpreter burst cannot starve the runtime worker. Does not bound calls across time/agents (see TM-TOOL-009). | MITIGATED |
| TM-DOS-016 | Mass resource creation via IP rotation | High | Per-org/per-user rate limits on expensive mutations (session create: 60/min per org; schedule create: 20/min per user; org create: 10/hr per user) via `OrgRateLimiter` (`crates/server/src/auth/rate_limit.rs`). Distributed when `VALKEY_URL` is set, in-memory otherwise. Fail-open on Valkey errors; DB-level resource caps (`max_orgs_per_user`, etc.) bound total consumption. Global per-IP `ApiRateLimiter` also uses Valkey when set. | MITIGATED |
| TM-DOS-025 | External eval import fan-out | Medium | Import requests are rejected before allocation or storage work when they exceed the configurable evals-per-import or cases-per-run limits. The generic HTTP request-body limit additionally bounds free-form metadata and strings. | MITIGATED |
| TM-DOS-016 | Mass resource creation via IP rotation | High | Per-org/per-user rate limits on expensive mutations (session create: 60/min per org; schedule create: 20/min per user; org create: 10/hr per user) via `OrgRateLimiter` (`crates/server/src/auth/rate_limit.rs`). Session creation and fork are enforced inside the transport-independent `CreateSession`/`ForkSession` commands, so REST and MCP dispatch share the same per-org bucket. The global-chat get-or-create service charges that bucket only on its actual create branch, leaving ordinary reuse unthrottled. Distributed when `VALKEY_URL` is set, in-memory otherwise. Fail-open on Valkey errors; DB-level resource caps (`max_orgs_per_user`, etc.) bound total consumption. Global per-IP `ApiRateLimiter` also uses Valkey when set. | MITIGATED |
| TM-DOS-017 | ReDoS / oversized config via guardrail checks | Medium | Guardrail `regex` rules (config-persisted and via `POST /v1/capabilities/guardrails/dry-run`) compile with `RegexBuilder::size_limit` (1 MB), so the linear-time `regex` engine cannot be wedged by a pathological pattern; check count, entries-per-check, entry length, and replacement length are capped at compile time, and dry-run input text is bounded to 64 KiB (`crates/core/src/guardrail_checks.rs`, `domains/capabilities/commands.rs`). Compilation runs synchronously in the streaming/tool path but is bounded; invalid persisted config is logged and treated as no checks rather than failing the turn. | MITIGATED |
| TM-DOS-018 | Eval run fan-out bypass via concurrent creation | Medium | `POST /runs` enforces per-org active-run caps and per-run case caps inside one storage critical section. PostgreSQL uses an org-scoped transaction advisory lock around active-run counting, case snapshot selection, run insertion, and result insertion; in-memory storage holds the run write lock across the same sequence. | MITIGATED |
| TM-DOS-019 | llm_judge prompt / content amplification via guardrail config | Medium | `llm_judge` check prompts are bounded to `MAX_JUDGE_PROMPT_LEN` (4 000 bytes) at compile time; tool content forwarded to the judge is capped at 2 000 bytes (truncated to the nearest UTF-8 char boundary); at most `MAX_JUDGE_CALLS_PER_INVOCATION` (4) judge calls per tool invocation, each with a hard `JUDGE_TIMEOUT` (10 s); invalid judge configs are caught at compile/validate time. An agent operator cannot use judge prompts or content size to increase per-call latency beyond 40 s. | MITIGATED |
| TM-DOS-020 | Guardrail `mcp` check latency / payload amplification | Medium | The `mcp` check `server`/`tool` references are bounded to `MAX_MCP_REF_LEN` at compile time; content forwarded to the endpoint is capped at 2 000 bytes (UTF-8 char-boundary safe); at most `MAX_MCP_CALLS_PER_INVOCATION` (4) mcp calls per tool invocation, each with a hard `MCP_CHECK_TIMEOUT` (10 s), maximum added latency 40 s for mcp checks alone; calls are serialized within a single hook and every failure mode fails open. The cap is per-check-type: `llm_judge` (TM-DOS-019) and `mcp` checks run serially in the same hook, so when both are configured on a stage the additive worst case is `(MAX_JUDGE_CALLS_PER_INVOCATION + MAX_MCP_CALLS_PER_INVOCATION) × 10 s` (80 s today), there is no shared cross-type budget yet; each type bounds itself independently. A slow or unresponsive external guardrail cannot wedge a turn beyond its own bounded timeout budget. | MITIGATED |
| TM-DOS-021 | Session-task retention prune wedges the reaper or destroys live data | Medium | The `session_task_reaper` retention pass (EVE-580) prunes terminal task records + messages + `result_path` artifacts on a global TTL. Work is bounded per tick by `retention_limit` (default 100); a backlog drains across ticks so the pass cannot wedge the reaper or exhaust memory (mirrors the orphan-scan and blob-GC bounds). The prune predicate is strictly `state IN ('succeeded','failed','canceled') AND finished_at < now - TTL`, so live/queued/running tasks and recently-finished terminal tasks can never be deleted (covered by store-level tests); a partial index on terminal `finished_at` (migration 075) keeps the scan cheap. Deletes are keyed on each task's own primary key so the global by-age query cannot cross-delete between orgs (TM-TENANT). `SESSION_TASK_RETENTION_TTL_SECONDS=0` disables pruning. | MITIGATED |
| TM-DOS-022 | Utility-LLM spend / connection exhaustion via agent analysis | Medium | `analyze_agent` requires `agent.manage` but makes paid utility-LLM calls. The command now acquires an analysis admission permit before previewing or calling the utility LLM: per-process rolling-window limits by org and caller, a small global semaphore for concurrent analyses, and a 429-style `rate_limited` command error with `retry_after_seconds`. `run_llm_checks` also rejects checker inputs above 128 KiB before any LLM call, while existing checker timeouts, output-token caps, and finding/message clamps bound responses. These are in-process controls; multi-instance deployments should still rely on platform/API limits for cross-instance coordination. | MITIGATED |
| TM-DOS-023 | Durable turn crash-loop re-billing (poison turn) | High | A durable turn that deterministically crashes mid-reason/act and is reclaimed on heartbeat timeout would otherwise re-run (re-spending tokens/billing) until it incidentally hits max-iterations or `max_attempts`. The forward-progress guard (EVE-534, `knowledge/operations/durable-execution-engine.md`) derives a per-turn progress token from the highest `durable_workflow_events.sequence_num` and seals the turn, marks the task `dead` → DLQ, non-retryable, after `N` consecutive no-progress recoveries (default 3, `DURABLE_NO_PROGRESS_SEAL_THRESHOLD`). The token is derived from durable facts and cannot be advanced by a non-progressing retry, so a crash-loop is stopped on *progress* rather than relying on attempt/iteration ceilings. Work-budget exhaustion (`HardLimitStopRule` balance ≤ 0) is likewise sealed (`reason=budget`) and routed straight to the DLQ instead of retrying. | MITIGATED |
| TM-DOS-024 | Unbounded per-org harness / agent / session creation | Medium | All three are `org_id`-scoped entities that previously had no absolute count cap (session create only had a 60/min per-org rate limit per TM-DOS-016, which bounds rate but not total). Per-org caps are now enforced in the `CreateHarness`/`CreateAgent`/`CreateSession` commands (covering HTTP, MCP, and gRPC entry paths) before insert, returning `409 CONFLICT`: `max_harnesses_per_org` (default 50, `RESOURCE_LIMIT_MAX_HARNESSES_PER_ORG`), `max_agents_per_org` (default 500, `RESOURCE_LIMIT_MAX_AGENTS_PER_ORG`), `max_sessions_per_org` (default 10000, `RESOURCE_LIMIT_MAX_SESSIONS_PER_ORG`). Counts are org-scoped (no cross-tenant leakage) and exclude soft-deleted harness/agent rows; the harness count also excludes system-seeded built-in harnesses so they cannot starve a user's budget; sessions are hard-deleted so only live rows count. SaaS tunes the caps per plan via env. | MITIGATED |
| TM-DOS-025 | Unbounded OAuth dynamic client registration | Low | `POST /oauth/register` (RFC 7591) is unauthenticated by design, so it could create unbounded `oauth_clients` rows (storage exhaustion). The endpoint now reuses the shared per-IP `AuthRateLimiter` `register` limit (5/min, `crates/server/src/auth/mcp_oauth.rs`), the same limiter and trusted-proxy client-IP extraction as the builtin signup endpoint (TM-AUTH-001). On breach it returns HTTP 429 with an OAuth-shaped `too_many_requests` error. Distributed when `VALKEY_URL` is set (fail-closed), in-memory per-instance otherwise. Residual risk: per-instance budget multiplies by N instances without Valkey; IP-rotation attackers still get one window's budget per IP, same residual as TM-AUTH-001. | MITIGATED |
| TM-DOS-026 | Oversized synchronous ATIF session export | Medium | `GET /v1/sessions/{id}/export?format=atif` folds the whole event log into one in-memory JSON document. The serialized body is capped at `ATIF_EXPORT_MAX_BYTES` (50 MiB, `crates/server/src/atif.rs`); over-cap documents are rejected with HTTP 413 instead of being buffered onto the response path. Image bytes are never included (parts flatten to `"[image]"` markers with locator records only), keeping the dominant payload class out of the document. Over-cap sessions have a recoverable path via segmented export (`&segmented=true`, TM-DOS-029) that bounds every response to the same cap. Residual: the fold still materializes the event list in memory before the size check; event logs are bounded by turn/iteration limits (TM-DOS-002). | MITIGATED |
| TM-DOS-029 | Segmented ATIF export cursor abuse (untrusted query params → unbounded work / cross-session read) | Medium | `GET /v1/sessions/{id}/export?format=atif&segmented=true[&cursor=…]` (knowledge/evaluation/atif-adoption.md) adds two attacker-controllable query params. Each segment response is byte-bounded to `ATIF_EXPORT_MAX_BYTES` by the greedy packer in `build_segment` (`crates/server/src/atif.rs`), so segmentation strictly *reduces* peak response/serialization size versus the whole-document path, it never buffers more than one bounded segment. The `cursor` is opaque `base64url(JSON)`: length-capped (`MAX_CURSOR_BYTES` = 4 KiB), decoded, version-checked, and bound to a session id that must equal the path session before loading events or folding ATIF steps; only offset bounds require the folded step count. A malformed, foreign, or out-of-range cursor is rejected with HTTP 400 (`atif_cursor_invalid`), never a panic (TM-API). Scope is unchanged from the whole-doc path: the session is resolved org-scoped from the path (TM-TENANT-001), and the cursor only selects a step offset *within that session*, it cannot widen scope or reference another org's data. The walk is naturally bounded by the session's own step count (each segment emits ≥1 step and offsets strictly increase), so a client cannot drive unbounded segments; a single step larger than the cap is emitted alone (documented caveat) rather than looping. Secret scrubbing runs per segment, same as the whole-doc path. | MITIGATED |
| TM-DOS-027 | Recursive or wide subagent delegation explosion | High | Subagent spawning computes the current session's delegation depth by walking `parent_session_id` only until the configured `max_subagent_depth` is exceeded, then rejects the spawn with a `ToolError` naming the attempted depth and cap. Default depth cap is 2 (top-level -> child -> grandchild); setting 0 blocks all subagent spawning. The same admission path walks the root session's subagent task tree and rejects new spawns before child creation when the root would exceed `max_active_descendant_tasks` (default 16 non-terminal descendants) or `max_total_descendant_tasks` (default 200 descendant task records). The model-facing `spawn_agent` tool shares a scheduler `concurrency_class`, so same-batch spawn calls are serialized before this count-before-create admission check. This bounds both recursive depth and shallow wide fan-out at the tool boundary. Detached peer spawns (`lifetime=detached`) reset depth by design but are capped separately against the same origin root, see TM-DOS-030. | MITIGATED |
| TM-DOS-028 | Nested subagent budget bypass via descendant session subjects | High | Budget scope resolution maps session-scoped budgets to `sessions.root_session_id` for every descendant, so child and grandchild LLM generations debit the same root session budget and `check_budget` reports the root pool. Usage journal and ledger rows still record the actual child session id for attribution. | MITIGATED |
| TM-DOS-030 | Detached-spawn governance side door (peer fan-out and budget escape) | High | A detached spawn remains a lifecycle-independent peer and resets nesting depth, but `spawn_create_and_wait` enforces three independent gates before creation: detached active/total task caps against the origin tree, host-provided session-creation authority (TM-AUTHZ-014), and an explicit origin-root budget override. Both storage backends canonicalize that override within the org, so detached chains debit the same root session budget and stop at its ceiling. Existing linked descendant accounting is unchanged. | MITIGATED |
| TM-DOS-031 | Unbounded in-process MCP tool-list cache | Low | `HttpTransport` caches `tools/list` results for the server-declared `ttlMs`, one entry per (server, credential) pair. Entries are far larger than the negotiation verdicts cached alongside them, so `store_tools` drops all expired entries before each insert, bounding the map to live entries rather than letting a long-lived control plane accumulate every catalog it has ever fetched. | MITIGATED |
| TM-DOS-032 | Provider outage traps a turn in layered retry loops | Medium | Provider request, first-stream reconnect, and reason-level recovery share strict attempt and elapsed-time budgets. Lower layers mark terminal retry decisions so an upper layer does not restart an already-exhausted policy. Backoff waits and retry attempts are charged to the elapsed budget; exhaustion returns a resumable terminal failure instead of continuing indefinitely. | MITIGATED |
| TM-DOS-033 | Unbounded async eval dataset exports | Medium | Dataset exports deduplicate identical `(org, run, request)` work through a database uniqueness constraint, reject admission when four exports are already active in a server process, and abort incremental NDJSON assembly at the shared 50 MiB artifact-export limit. The permit is acquired before a durable row is created and held by the background task through completion, bounding both concurrent reconstruction work and persisted body size. | MITIGATED |
| TM-DOS-034 | Sessions facet aggregates scan an org's whole session table | Low | `GET /v1/sessions/facets` (EVE-852) answers counts per status, source, and agent plus masthead metrics as SQL aggregates rather than by paging the list, so a single request touches every session row matching the caller's filters, an unfiltered request touches the org's entire `sessions` table. Bounded and measured rather than open-ended: the query is org-scoped in its base CTE (TM-TENANT-001), every branch resolves through an org-prefixed index, and the aggregate is index-only in the unfiltered worst case (`EXPLAIN (ANALYZE)` at 500k rows / 400k in-org: 156 ms, no heap access). Cost is therefore linear in the caller's own org size, which their existing list `COUNT(*)` already pays, and the read is subject to the global per-IP `ApiRateLimiter`. Residual: a very large org can make this endpoint the most expensive read on the API; revisit with a cached or projection-backed facet if org sizes outgrow the index-only plan. | MITIGATED |
| TM-DOS-035 | Framework history cursor, lifecycle-heavy replay, or local JSONL recovery drives unbounded allocation/work | Medium | Public history is explicit bounded paging (100 messages by default, 256 maximum); cursors are opaque, session-bound snapshot tokens capped at 4 KiB before decoding; each host projection caps examined canonical envelopes at 100 000 and returns a typed `HistoryTooLarge` error rather than scanning indefinitely. Lazy whole-history traversal retains only one page at a time. `JsonlEventLog::open` reads at most 128 MiB and indexes at most 1,000,000 events by default, returning `RecoveryLimitExceeded` before constructing an unbounded startup index; event identity in that index is a fixed SHA-256 digest rather than retained canonical JSON bytes. | MITIGATED |
| TM-DOS-036 | Framework live-session callers outpace a slow model and grow an unbounded in-memory mailbox | Medium | The application-facing session actor uses a bounded command channel and retains only a bounded deferred-command window while a turn reaches its terminal boundary. The host's between-reason steering inbox has its own fixed pending-message cap; overflow is rejected before acceptance with a typed Framework error. Turn iteration limits bound how often queued steering can extend one turn. This is an in-process application API rather than a new remote ingress boundary; transport-level request and payload limits remain owned by their transports. | MITIGATED |

### Mitigation Details

**TM-DOS-009, Valkey Network Exposure (CALLER RISK):**
Valkey (Redis-compatible) is used for distributed rate limiting. In local/dev compose, it runs without authentication on port 6379.
- **Production:** Deploy Valkey on a private network, not exposed to the internet. Use `rediss://` (TLS) URLs and AUTH passwords for cloud-managed instances (e.g., AWS ElastiCache, GCP Memorystore).
- **Blast radius if compromised:** Attacker can flush rate limit counters (bypassing rate limits) or inject fake counters (DoS via false rate-limit-exceeded). No sensitive data stored in Valkey.

**TM-DOS-010, Fail-Open Rate Limiting (ACCEPTED):**
By design, Valkey errors cause rate limiting to fail open (allow requests) for `ApiRateLimiter` and `OrgRateLimiter`. This prioritizes availability over strictness for general API traffic. Auth endpoints (`AuthRateLimiter`) remain fail-closed. See `crates/server/src/auth/rate_limit.rs`.

**TM-DOS-016, Per-identity rate limiting (MITIGATED):**
`OrgRateLimiter` (`crates/server/src/auth/rate_limit.rs`) adds per-identity velocity caps on expensive operations. Configurable via `RATE_LIMIT_ORG_SESSION_CREATE_PER_MINUTE` (default 60), `RATE_LIMIT_ORG_SCHEDULE_CREATE_PER_MINUTE` (default 20), and `RATE_LIMIT_USER_ORG_CREATE_PER_HOUR` (default 10). Uses Valkey when `VALKEY_URL` is set. Residual risk: without Valkey, limits are per-instance.

## 17. Daytona Cloud Sandbox (TM-DAYTONA)

Daytona sandboxes are remote Linux environments managed via REST API. The agent can create, exec commands, and manage files in these sandboxes. The `daytona_git_credentials` tool writes a GitHub token to disk inside the sandbox to enable git push/pull/fetch operations.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-DAYTONA-001 | Git token persisted on sandbox disk | Medium | Token written to `/tmp/.git-credentials`; lost on sandbox stop/delete; same trust boundary as `daytona_exec` (anyone who can exec can already read the file) | **ACCEPTED** |
| TM-DAYTONA-002 | Git token expiry, stale credentials | Low | GitHub App installation tokens expire in ~1 hour; tool hint tells agent to call `daytona_git_credentials` again to refresh | MITIGATED |
| TM-DAYTONA-003 | Git token scope, over-privileged access | Medium | Token scoped by GitHub App installation permissions; user controls repo access via GitHub App settings | **CALLER RISK** |
| TM-DAYTONA-004 | Daytona API key compromise | High | Stored in user connections (Settings > Connections); encrypted at rest via envelope encryption (AES-256-GCM) | MITIGATED |
| TM-DAYTONA-005 | Cross-session sandbox access | Critical | Daytona tools require session-owned sandbox IDs via leased-resource/session-resource ownership checks; persisted sandbox state stays session-scoped in `daytona_sandbox:{id}` | MITIGATED |
| TM-DAYTONA-006 | Sandbox not deleted, resource leak | Low | Auto-stop 5 min, auto-archive 30 min, auto-delete 60 min (Daytona-native); leased-resource cleanup 20 min (control plane); system prompt instructs agent to delete when done | MITIGATED |
| TM-DAYTONA-007 | Git credential helper persists after sandbox reuse | Low | Credential file in `/tmp` cleared on stop; sandbox stop resets environment | MITIGATED |
| TM-DAYTONA-008 | GitHub token leaked to lookalike clone host | High | `daytona_git_clone` and `daytona_git_credentials` only embed the GitHub token in HTTPS URLs whose host matches an operator-configured trusted-host allowlist (`trusted_github_hosts` / `is_trusted_github_https_host` in `integrations/daytona/src/tools.rs`). Default `["github.com"]`; operators extend via `EVERRUNS_DAYTONA_GITHUB_TRUSTED_HOSTS` (comma-separated, exact case-insensitive match, no wildcards). Malformed env entries (`/`, `@`, whitespace, `..`) are rejected with a warning; the default is always preserved so misconfig cannot silently disable public-GitHub auth. Unit tests cover lookalike rejection (`evil-github.acme.com`, `github.acme.com.evil.example`). | MITIGATED |
| TM-DAYTONA-009 | Cross-session recovery-volume access | Critical | Managed sandboxes share one Daytona Volume but mount only `sessions/<session_id>` through Daytona's FUSE-enforced `subpath`; the stable binding is session-scoped encrypted state, and invalid persisted mount/subpath values are rejected before use | MITIGATED |
| TM-DAYTONA-010 | Recovery archives retain workspace secrets after physical cleanup | Medium | Connection credentials remain outside `/workspace`; checkpoints exclude common caches, retain ten immutable revisions by default, and explicit logical sandbox deletion clears the isolated volume subpath. Daytona-native or lease cleanup intentionally preserves recovery state, so orphan cleanup remains part of graduating the experimental sandbox resource | **ACCEPTED** |

### Mitigation Details

**TM-DAYTONA-001, Git Token on Disk (ACCEPTED):**
The `daytona_git_credentials` tool writes `https://oauth2:<token>@github.com\n` to `/tmp/.git-credentials` and configures `git config --global credential.helper 'store --file=/tmp/.git-credentials'`. This is the same pattern used by GitHub Actions and other CI systems.

Accepted because:
- The sandbox is an isolated environment, same trust boundary as exec access
- Any agent that can call `daytona_exec` can already run arbitrary commands
- Token is in `/tmp`, lost on sandbox stop/delete
- Token is short-lived (~1 hour GitHub App installation token)
- Alternative (API-proxied credential helper) deferred as future improvement

**TM-DAYTONA-003, Token Scope (CALLER RISK):**
The GitHub token's scope depends on the GitHub App installation permissions. Users must review which repositories the GitHub App has access to in their GitHub settings. Everruns does not enforce per-repo restrictions at the application level.

**TM-DAYTONA-005, Cross-Session Isolation:**
```
Session A stores: daytona_sandbox:sb_abc → {sandbox_id, workspace_path, started_at}
Session B stores: daytona_sandbox:sb_xyz → {sandbox_id, workspace_path, started_at}

Session A cannot access sb_xyz because tool-side ownership checks reject non-owned sandbox IDs
before the Daytona API call, and persisted sandbox state is still scoped by session_id.
```

## 17A. Deno Sandbox (TM-DENO)

Deno sandboxes are remote Linux microVMs managed over a websocket + REST control plane. Everruns creates sandboxes with a fixed timeout, reconnects for each tool call, and deletes them via leased-resource cleanup.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-DENO-001 | Service-wide token misuse via env fallback | High | Env fallback removed; credential resolves exclusively from user connection (`connection_resolver`); tools return `ConnectionRequired` when no connection is configured | MITIGATED |
| TM-DENO-002 | Cross-session sandbox access | Critical | Deno tools require session-owned sandbox IDs via leased-resource/session-resource ownership checks; persisted sandbox state remains session-scoped in `deno_sandbox:{id}` | MITIGATED |
| TM-DENO-003 | Sandbox leak from default `session` timeout | Medium | `deno_create_sandbox` forbids `timeout="session"`; Everruns always uses explicit TTLs plus leased-resource cleanup | MITIGATED |
| TM-API-015 | Lease metadata exposure | Medium | Deno lease metadata stores only non-secret routing/debug fields (`region`, optional `org`, workspace path, timestamps); tokens still resolve from connections/env at cleanup time | MITIGATED |
| TM-DENO-004 | Network probing from remote sandbox | High | Capability is Admin-gated; residual exposure depends on operator egress controls | **ACCEPTED** |

## 17B. Cursor Cloud Agents (TM-CURSOR)

Cursor Cloud Agents are third-party asynchronous coding agents that clone GitHub repositories, run commands in Cursor-managed remote environments, push branches, and may create PRs. Everruns calls Cursor's REST API with a user or operator-provided Cursor Cloud Agents API key.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-CURSOR-001 | Cursor API key captured in chat history | High | Cursor tools return `ConnectionRequired` when no key is configured, causing the inline connection dialog; user connection credentials are encrypted at rest. Operator env fallback is for Doppler-backed deployments/tests. | MITIGATED |
| TM-CURSOR-002 | Over-scoped Cursor/GitHub repository access | High | Cursor GitHub app access and Cloud Agents API key scope are controlled in Cursor/GitHub; Everruns docs tell users to grant only intended repos. | **CALLER RISK** |
| TM-CURSOR-003 | Prompt injection or malicious repo content causes remote command/data exfiltration | High | Capability is high-risk/Admin-gated via capability assignment policy; prompts should provide bounded task scope. Residual risk belongs to Cursor's remote agent environment and repository/secret configuration. | **ACCEPTED** |
| TM-CURSOR-004 | Cost/resource runaway from launching too many Cursor agents | Medium | Cursor enforces account limits; Everruns marks launch as long-running/external and seed prompts tell agents to triage first. Operators/users must monitor Cursor usage. | **CALLER RISK** |
| TM-CURSOR-005 | Sensitive webhook secret stored in tool call history | High | First release intentionally omits webhook secret support from tool schema; add only with a dedicated secret flow. | MITIGATED |
| TM-CURSOR-006 | Large prompt or identifier payload causes API/resource abuse | Medium | Tool-side length validation bounds prompt, agent id, ref, model, and branch fields before outbound requests. | MITIGATED |
| TM-CURSOR-007 | Repository enumeration and rate-limit abuse | Low | `cursor_list_repositories` is read-only but documented as heavily rate-limited; seed prompt instructs agents to prefer explicit repository URLs and use the endpoint sparingly. | MITIGATED |

## 17C. GitHub Scout (TM-GITHUB)

GitHub Scout is a blueprint-only integration. It gives the child agent private read-only GitHub REST tools for code search, file reads, and issue or pull request search. Credentials resolve from the existing GitHub user connection, with `GITHUB_TOKEN` session secret fallback.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-GITHUB-001 | GitHub token captured in chat history | High | Tools return `ConnectionRequired` when no GitHub connection or session secret is available; they never ask for tokens in chat and never include tokens in tool output. | MITIGATED |
| TM-GITHUB-002 | Over-scoped repository read access | Medium | Access is bounded by the user's GitHub App installation/token scope. `github_scout` is read-only, but Everruns does not enforce per-repository policy beyond optional `repos` config and GitHub's own authorization. | **CALLER RISK** |
| TM-GITHUB-003 | Outbound request bypasses session network policy | Medium | Tool execution checks the session network access list before calling `https://api.github.com/`. | MITIGATED |
| TM-GITHUB-004 | Repository path confusion in file reads | Low | `read_github_file` validates `owner/repo` segments and rejects leading slash, empty, dot, and dot-dot file path segments before constructing the GitHub contents API URL. Remaining file path bytes are percent-encoded. | MITIGATED |

## 18. E2B Cloud Sandbox (TM-E2B)

E2B sandboxes are remote Linux environments managed through the E2B Management API plus per-sandbox envd runtime endpoints. Users bring their own E2B API key via the connection provider; no platform-owned or environment-variable fallback exists. Per-sandbox envd access tokens are stored in session-scoped secrets.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-E2B-001 | E2B API key captured in chat history | High | Tools return `ConnectionRequired` when no connection is configured, triggering the inline connection dialog; user connection credentials are encrypted at rest; key never emitted in tool output | MITIGATED |
| TM-E2B-002 | envd access token disclosure | Medium | envd access token stored only in encrypted session secrets and sent only to E2B runtime headers | MITIGATED |
| TM-E2B-003 | Cross-session sandbox access | Critical | E2B tools require session-owned sandbox IDs via leased-resource/session-resource ownership checks; envd state remains session-scoped under `e2b_sandbox:{id}` | MITIGATED |
| TM-E2B-004 | Sandbox not deleted or paused, resource leak | Low | E2B timeout + auto-pause on create/resume, plus Everruns leased-resource cleanup | MITIGATED |
| TM-E2B-005 | Full-network sandbox misuse | High | Capability is high-risk/Admin-gated via capability assignment policy; residual network exposure depends on deployment egress isolation | **CALLER RISK** |

### Mitigation Details

**TM-E2B-003, Cross-Session Isolation:**
```
Session A stores: e2b_sandbox:sb_abc → {sandbox_id, sandbox_domain, envd_access_token, ...}
Session B stores: e2b_sandbox:sb_xyz → {sandbox_id, sandbox_domain, envd_access_token, ...}

Session A cannot access sb_xyz because tool-side ownership checks reject non-owned sandbox IDs
before the E2B API call, and storage lookups remain scoped by session_id.
```

## 19. Client-Side Tools (TM-CLIENT)

Client-side tools pause server execution and wait for client to submit results via API. Attack surface includes tool call ID spoofing and timeout abuse.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-CLIENT-001 | Tool call ID spoofing | Medium | Submitted `tool_call_id` values must exactly match pending requests; mismatches rejected | MITIGATED |
| TM-CLIENT-002 | Tool result size explosion | Medium | Per-result size capped at 100 KB | MITIGATED |
| TM-CLIENT-003 | Client timeout abuse | Low | Default 5 min timeout; session transitions to failed state on expiry | MITIGATED |
| TM-CLIENT-004 | Client-side tool shadowing of MCP guardrail endpoints | High | Session and agent `tools[]` deserialization rejects `client_side` definitions whose names use the reserved `mcp_` prefix before runtime tool deduplication, preventing user-authored metadata from replacing worker-executable MCP tool definitions that `ScopedMcpToolInvoker` relies on for guardrail scope checks. | MITIGATED |

## 20. Brave Search (TM-LLM)

Search results from Brave Search are returned as tool results. Adversarial content in search results could influence LLM behavior.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-LLM-008 | Search result prompt injection | Medium | Results returned as `tool_result` role; inherent LLM limitation (same as TM-TOOL-005) | **ACCEPTED** |
| TM-LLM-009 | Search query privacy | Low | Queries sent to Brave Search (third party); caller responsibility to assess data classification | **CALLER RISK** |

## 21. Container Sandbox (TM-SANDBOX)

Self-hosted container sandboxes via Docker Engine REST API. Agents create, exec, and manage containers per-session. Containers run on the same infrastructure as the server/worker, unlike cloud sandboxes (Daytona, E2B, Deno) which run on third-party infrastructure.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-SANDBOX-001 | Container escape via kernel vulnerability | High | Configurable runtime (`runtime` config field): default empty = plain `runc`; operator chooses a hardened runtime (`sysbox-runc`, `gvisor`, `kata`) for untrusted/multi-tenant workloads | **ACCEPTED** |
| TM-SANDBOX-002 | Resource exhaustion (memory/CPU/PIDs) | High | cgroup limits enforced via Docker create flags (`memory_limit`, `cpu_limit`, `pids_limit`); defaults: 2 GiB, 1 CPU, 256 PIDs | MITIGATED |
| TM-SANDBOX-003 | Network attacks / SSRF / metadata access from sandbox | High | Per-sandbox isolated Docker bridge network limits cross-sandbox reachability, but **no egress / private-IP / cloud-metadata (169.254.169.254) filtering is implemented**; default bridge networking can route to RFC1918 and metadata endpoints. Egress restriction is the operator's responsibility at the network/firewall layer | **ACCEPTED** |
| TM-SANDBOX-004 | Cross-session container access | Critical | Container names include `session_id`; all Docker API queries filtered by `session` + `managed-by` labels; sandbox state stored in session-scoped secrets | MITIGATED |
| TM-SANDBOX-005 | Image supply chain attack | Medium | Image allowlist in capability config; only pre-approved images can be pulled | MITIGATED |
| TM-SANDBOX-006 | Docker socket exposure inside sandbox | High | Docker socket never mounted into containers; no `--privileged` flag | MITIGATED |
| TM-SANDBOX-007 | Stale container not cleaned up | Medium | Leased resource scheduler with 20-minute lease duration; system prompt instructs agent to remove when done | MITIGATED |
| TM-SANDBOX-008 | Cross-tenant sandbox access | Critical | Tool scoping via `ToolContext.session_id` + per-sandbox network + Docker label filters; container names derived from session UUID, never user input | MITIGATED |
| TM-SANDBOX-009 | Cross-tenant network reachability | High | Each sandbox gets its own isolated Docker bridge network (`sandbox-{org}-{session}`); sole member is the sandbox container | MITIGATED |
| TM-SANDBOX-010 | Tenant resource starvation | High | Per-sandbox cgroups + per-org concurrent sandbox limits via leased resources | MITIGATED |
| TM-SANDBOX-011 | Docker control-plane exposure via insecure host | High | Default Docker host is the host-local unix socket (`unix:///var/run/docker.sock`), never an unauthenticated plaintext TCP endpoint. The client fails closed when it cannot reach a usable daemon rather than defaulting to a network endpoint; TCP daemons must be set explicitly and protected with mTLS | MITIGATED |

### Mitigation Details

**TM-SANDBOX-001, Container Escape (ACCEPTED):**
Container isolation depends on the kernel and runtime. The default (empty `runtime`) is plain `runc`, which provides namespace + cgroup isolation but shares the host kernel. This default is intentional so the capability works on stock Docker and in CI. For untrusted or multi-tenant workloads, the deploying operator is responsible for configuring a hardened runtime: `sysbox-runc` (adds user namespaces, procfs/sysfs virtualization) or `kata`/`gvisor` for stronger isolation. The runtime is a deployment-time config field (`runtime` in `ContainerSandboxConfig`), not baked into code.

**TM-SANDBOX-003, Network Egress (ACCEPTED):**
Each sandbox runs on its own per-session Docker bridge network, which limits cross-sandbox reachability (TM-SANDBOX-009). However, no application-level egress filtering is implemented: a sandboxed container reaches whatever its bridge can route to, including RFC1918 private ranges and the cloud metadata endpoint (169.254.169.254), so SSRF / internal-network probing from inside a sandbox is possible (see TM-AGENT-019). The `network_mode` config field is parsed but not yet consulted at container creation. Restricting egress (blocking RFC1918 + metadata) is the operator's responsibility at the network/firewall layer until in-product egress controls land.

**TM-SANDBOX-011, Docker Host Transport (MITIGATED):**
The Docker Engine API is the control plane for the host's containers; an unauthenticated, plaintext TCP endpoint (e.g. `http://localhost:2375`) reachable over a network grants full Docker control and therefore host escape. The default host is the local unix socket (`unix:///var/run/docker.sock`), resolved via config → `CONTAINER_SANDBOX_DOCKER_HOST` env → default. The reqwest-based client does not speak the unix transport yet, so the default fails closed at request time with an actionable error rather than silently falling back to a network endpoint. Operators that need a TCP daemon must set `CONTAINER_SANDBOX_DOCKER_HOST` explicitly to an `http(s)://` URL and protect any non-loopback address with mTLS; plaintext TCP must never be exposed on a network. CI/dind sets this env explicitly to `http://localhost:2375` (loopback only), so the secure default does not affect tests.

**TM-SANDBOX-004, Cross-Session Isolation:**
```
Session A creates: sandbox-{org}-{session_a} → container + network
Session B creates: sandbox-{org}-{session_b} → container + network

Session A cannot access Session B's container:
  - Docker API queries include label filter: session={session_a}
  - Container name includes session_a UUID
  - Sandbox state stored in session-scoped secrets
```

**TM-SANDBOX-008, Cross-Tenant Isolation (6 layers):**
1. Tool scoping: container name derived from `ToolContext.session_id`, never user input
2. Per-sandbox Docker network: `sandbox-{org}-{session}`, sole member = the sandbox
3. Label-filtered API calls: all queries include `session` + `managed-by` labels
4. Per-org limits: max concurrent sandboxes checked at create time via leased resources
5. Runtime isolation: configurable (sysbox adds user-ns + procfs virtualization)

Note: egress filtering (blocking private IPs + cloud metadata) is **not** implemented in-product; it is the operator's network-layer responsibility (see TM-SANDBOX-003, TM-AGENT-019).

## 22. A2A Channel (TM-A2A)

App-scoped Agent2Agent (A2A) protocol ingress. JSON-RPC 2.0 endpoint authenticated by a per-channel API key. Mitigations live in `crates/server/src/api/app_a2a.rs` and `crates/server/src/domains/apps/commands.rs`. See `knowledge/integrations/a2a-channel.md`.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-A2A-001 | API key brute force | Medium | Keys are 32 random bytes (256-bit entropy) prefixed `evra2a_`; stored only as SHA-256 hex; plaintext returned exactly once at create / regenerate; the published Agent Card never includes the key or hash | MITIGATED |
| TM-A2A-002 | Timing oracle on key compare | Medium | Constant-time byte comparison of the SHA-256 hash digests via the canonical `security::constant_time_eq` (called from `app_a2a`) before any session creation | MITIGATED |
| TM-A2A-003 | Plaintext key persistence / log leak | High | Plaintext is never persisted: only hash + non-secret prefix go into `channel_config`. `Authorization` headers are not surfaced into template context (A2A invocations only template `payload`, `a2a.*`, and app metadata, request headers are not exposed) | MITIGATED |
| TM-A2A-004 | Anonymous ingress to draft / disabled channels | High | Published-app + enabled-channel checks run before key validation; the Agent Card endpoint mirrors the same gate and 404s otherwise | MITIGATED |
| TM-A2A-005 | A2A method abuse beyond the supported set | Medium | Method allowlist of four (`message/send`, `message/stream`, `tasks/get`, `tasks/cancel`); all other methods return JSON-RPC `-32601 Method not found` without touching the session pipeline. Each handler reuses the same auth + channel + method gate before any session work | MITIGATED |
| TM-A2A-006 | Empty / non-text part injection | Low | The endpoint requires at least one non-empty `text` part; otherwise returns `-32602 Invalid params`. This prevents triggering an empty / whitespace-only user message into the session | MITIGATED |
| TM-A2A-007 | Cross-org session reuse via tag spoofing | High | Inherits the webhook channel mitigation: shared sessions are matched by both org + owner principal + tag set, so a user cannot pre-seed an `app:`/`app_channel:` tagged session and have an A2A invocation reuse it | MITIGATED |
| TM-A2A-008 | API key rotation does not invalidate old keys | Medium | `regenerate_a2a_app_channel_key` overwrites both `api_key_hash` and `api_key_prefix` in the same row, so the previous key fails constant-time comparison on the next request | MITIGATED |
| TM-A2A-009 | Agent Card discloses sensitive metadata | Low | Card shape is fixed (`name`, `description`, `supportedInterfaces`, capabilities, security schemes, public skill); never echoes the API key, hash, prefix, internal channel UUID, or owner principal | MITIGATED |
| TM-A2A-010 | Replay of captured request | Medium | Opt-in Slack-derived HMAC signing on the A2A channel via `A2aChannelConfig::signing_secret` (`crates/platform/src/app.rs`). When enabled, requests must carry `X-Everruns-A2A-Timestamp` + `X-Everruns-A2A-Signature` headers; the server verifies HMAC-SHA256 over `v0:{timestamp}:{channel_scope}:{raw_body}` (where `channel_scope` is the literal `{app_id}:{channel_id}`) using constant-time compare in `crates/server/src/api/a2a_signing.rs`. Including the channel scope inside the signed basestring also prevents cross-channel replay when operators share the same `signing_secret` across multiple A2A channels, without it, a captured request for channel A could be forwarded to channel B because the per-channel-keyed replay store would not catch the cross-channel reuse. A symmetric 5-minute timestamp window plus signature-keyed dedup (scope `app_id:channel_id`) mean a captured request can only be replayed once and only inside that window. Two backends mirror the rate limiter, in-memory HashMap with on-insert TTL pruning for single-instance/dev, Valkey `SET ... NX EX` for distributed. Check runs after primary authentication (API key or endpoint-auth) so unauthenticated callers cannot probe channel existence or grow the in-memory store. Plaintext secret is encrypted at rest via the existing `channel_config` envelope encryption, redacted on read with `signing_secret_configured: bool`, and preserved across PATCH. Channels without `signing_secret` keep the existing auth-only behavior, so existing deployments keep working. Same fail-open behavior on Valkey outage as TM-DOS-010 / TM-A2A-013. Defense-in-depth still applies: HTTPS (TM-AUTH-005) and rotation remain available | MITIGATED |
| TM-A2A-011 | `message/stream` SSE leaks events from unrelated sessions or holds resources after auth fails | Medium | The streaming branch reuses the same auth + channel + method gate as `message/send` and only subscribes to `EventDelivery` after the per-call session is resolved. The translator filters by `event.session_id == session_id` before emitting any frame, only translates an allowlist of session events (`output.message.completed`, `turn.completed`, `turn.failed`, `turn.cancelled`), and closes the stream after the first terminal status frame. A dropped subscription emits a synthetic `failed` final frame so the client does not hang | MITIGATED |
| TM-A2A-012 | `tasks/get` / `tasks/cancel` cross-channel reads or destructive actions | Medium | Both handlers reuse the same auth + channel + method gate as `message/send`, look up the underlying session org-scoped via `get_session(auth.org_id, ...)`, and additionally verify the session belongs to the authenticated app + channel via routing tags (`app:<public_id>` and `app_channel:<public_id>`) before returning or modifying anything. An API key from one A2A channel cannot read or cancel tasks for sessions created by another channel even if both share the same org. Sessions that fail the binding check return `-32001 Task not found` rather than leaking existence. State derivation only consults turn lifecycle events; raw prompts, tool args, and LLM outputs are never echoed back. Cancelling an already-terminal task is idempotent, `cancel_a2a_session_turn` re-derives state after `cancel_run` and skips the synthetic `turn.cancelled` emission if the workflow has reached a terminal state, so derived state cannot race-flip a `completed` task to `canceled` | MITIGATED |
| TM-A2A-013 | DoS via runaway A2A client | Medium | App owners can configure a per-app, per-IP request cap via `A2aChannelConfig::rate_limit_per_minute` (`crates/platform/src/app.rs`), enforced in `crates/server/src/api/app_a2a.rs::authenticate_request` after API key verification so an unauthenticated caller cannot grow the limiter cache or learn channel existence from rate-limit signals. Backed by the shared `ChannelRateLimiter` primitive (`crates/server/src/api/channel_rate_limit.rs`), in-memory governor for single-instance/dev, Valkey sliding-window when `VALKEY_URL` is set; namespaces (`agui` / `a2a`) keep keys disjoint. A2A scope is `app_id:channel_id` (not just `app_id`) so multiple A2A channels on the same app keep independent buckets, sharing an `app_id`-only key would let an attacker alternate between channels with different limits to flush the cached limiter (replace-on-limit-change) and bypass the stricter cap. Server caps the field at `1_000_000` so a typo cannot silently disable the limit. `0` / `None` disables the per-channel cap and falls back to the global API limit. Same fail-open behavior on Valkey outage as TM-DOS-010 | MITIGATED |
| TM-A2A-014 | Agent Card advertises stale or wrong auth scheme | Medium | Agent Card security metadata is derived from the same effective `A2aChannelConfig.auth` used by `authenticate_request`. Legacy channels continue to advertise bearer API key; OIDC/Google emit `openIdConnect`, HTTP Basic emits `http/basic`, OAuth2 introspection emits generic HTTP bearer rather than fabricating OAuth token-flow metadata, and mTLS emits `mutualTLS`. The card remains published only for live apps with enabled A2A channels and never includes secrets. | MITIGATED |

### Mitigation Details

**TM-A2A-001, API Key Generation:**
```rust
// crates/server/src/domains/apps/commands.rs (actual implementation)
pub fn generate_a2a_api_key() -> (String, String, String) {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let hex = hex::encode(bytes);
    let plaintext = format!("evra2a_{hex}");
    let hash = hash_a2a_api_key(&plaintext);
    let prefix = format!("evra2a_{}...", &hex[..8]);
    (plaintext, hash, prefix)
}

pub fn hash_a2a_api_key(plaintext: &str) -> String {
    hex::encode(sha2::Sha256::digest(plaintext.as_bytes()))
}
```
The plaintext is returned in the `add_a2a_app_channel` and `regenerate_a2a_app_channel_key` command responses and never read back from storage, `channel_config` only holds the SHA-256 hash and the display prefix. Agent Card responses derive from the same row but explicitly select non-secret fields.

**TM-A2A-005, Method Gate:**
The handler dispatches strictly on `method == "message/send"`. Any other JSON-RPC method short-circuits with a structured `-32601` response *before* any session work. This keeps the surface narrow until streaming and task lifecycle features are implemented.

**TM-A2A-007, Tag-spoof Hardening:**
A2A reuses `find_app_session_by_tags_and_owner` (already mitigates the webhook variant TM-AUTHZ-006). Sessions matched for `shared_session` mode require the requesting app's `org_id` *and* `owner_principal_id` to line up, so a user-created session sharing the same surface tags is rejected.

## 23. FCP Channel (TM-FCP)

App-scoped Free Communication Protocol ingress. Text-first HTTP endpoint with a deliberately minimal auth stack (anonymous + optional shared bearer token) and a dedicated `ChannelRateLimiter` namespace. Mitigations live in `crates/server/src/api/fcp.rs` and `crates/server/src/domains/apps/commands.rs`. See `knowledge/integrations/fcp-channel.md`.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-FCP-001 | Anonymous FCP request reaches draft, disabled, or wrong-type app | High | `resolve_context` in `crates/server/src/api/fcp.rs` rejects any of (unknown app id, status != Published, no `fcp` channel, channel disabled) with the same generic 404 Markdown body. Caller cannot distinguish operator state, so the endpoint cannot be used as a probe oracle | MITIGATED |
| TM-FCP-002 | Timing oracle on token compare | Medium | `constant_time_eq` (`crates/server/src/api/fcp.rs`) bit-mixes the entire byte slice before returning, mirroring the A2A and AG-UI implementations. Token validation runs only after the request reaches the FCP handler, so timing differences for unknown apps are absorbed by the upstream 404 path | MITIGATED |
| TM-FCP-003 | Configured bearer token leaked in response body or logs | High | The 401 and handshake bodies are static templates that never interpolate the configured token. The token is redacted from `GET /v1/apps/{id}` reads (`redact_channel_config` in `crates/server/src/domains/apps/commands.rs` strips `token` and surfaces `token_configured: true`) and is held only in encrypted `channel_config` storage. Tracing does not log the token | MITIGATED |
| TM-FCP-004 | Cross-channel auth verifier reuse expands FCP attack surface | Medium | FCP deliberately does NOT call `AppEndpointAuthVerifier`. `normalize_and_validate_channel_config` rejects `channel_config.auth` for `ChannelType::Fcp`, so OIDC/HTTP-Basic/mTLS verifier code paths cannot run for FCP requests. New auth modes require an explicit code change, not a config flag | MITIGATED |
| TM-FCP-005 | Privileged message-role injection from public client | High | The wire format is opaque text or `{"message": "..."}` JSON. Only a `MessageRole::User` message is constructed by the handler (`crates/server/src/api/fcp.rs::message`); no path lets a caller forge `system`, `developer`, `assistant`, or `tool` roles into the LLM context | MITIGATED |
| TM-FCP-006 | DoS via runaway FCP client | Medium | App owners can configure a per-app, per-IP cap via `FcpChannelConfig::rate_limit_per_minute`. Counted in a dedicated `ChannelRateLimiter::with_valkey("fcp", ...)` namespace so buckets cannot be shared with AG-UI/A2A. Server caps the field at `1_000_000`; `0`/`None` disables the per-app cap and falls back to the global API limit. Same fail-open behavior on Valkey outage as TM-DOS-010 | MITIGATED |
| TM-FCP-007 | DoS via oversized body | Medium | `MAX_FCP_BODY_BYTES = 256 KiB`; the handler short-circuits to `413` with a sanitized Markdown body before doing any database, session, or runtime work. The limit is enforced after the size check on `axum::body::Bytes`, so the worst case is a single 256 KiB allocation | MITIGATED |
| TM-FCP-008 | Cross-org session reuse via FCP cookie spoofing | High | Session reuse keys on `(app.org_id, app.internal_id, [fcp:app:<app_public_id>])` via `find_app_session_by_tags`. A `fcp_session` cookie pointing at a UUID that does not match the looked-up row is silently ignored and a fresh session is created. The session adopts `app.owner_principal_id`, which is the same invariant used by AG-UI/A2A and is verified by the matching unit tests | MITIGATED |
| TM-FCP-009 | Stale-cookie strand or replay across operator config changes | Low | Unknown cookies fall through to fresh session creation rather than 4xxing the client. Expired sessions (after `session_expiration_seconds`) return `410 Gone` with a Markdown body instructing the client to drop the cookie and POST again | MITIGATED |
| TM-FCP-010 | Indefinite hang on slow / unavailable agent | Medium | Every POST is wrapped in `tokio::time::timeout(response_timeout_seconds.max(1))`. On elapsed budget the handler returns `504` with a Markdown body that names the configured timeout, sets the same `fcp_session` cookie so the client can retry the same conversation, and releases the event subscription. Server validates the field at 1–600 s | MITIGATED |
| TM-FCP-011 | Internal vocabulary or provider details leaked through error bodies | Medium | All `turn.failed` causes pass through `PublicError::from_internal_code` (`crates/server/src/api/public.rs`). The mapping returns one of four sanitized public messages (`InternalError`, `RateLimited`, `ServiceUnavailable`, `RequestTooLarge`), provider names, stack traces, and internal codes never reach the wire. Tested in `crates/server/src/api/fcp.rs::tests::turn_error_response_body_is_sanitized` | MITIGATED |
| TM-FCP-012 | Handshake (GET) used to probe operator existence | Low | The handshake is intentionally open per FCP SPEC, but `resolve_context` returns the same generic 404 body for unknown / draft / wrong-channel apps. The per-app rate limiter (when configured) also applies to GETs so a single client cannot hammer the handshake either | MITIGATED |

### Mitigation Details

**TM-FCP-003, Token redaction across read paths:**
```rust
// crates/server/src/domains/apps/commands.rs
ChannelType::Fcp => {
    if map.remove("token").is_some() {
        map.insert("token_configured".to_string(), Value::Bool(true));
    }
}
```
And on update, the preserved-secrets merger reinjects the existing encrypted token if the caller did not provide a new one, so PATCHing other fields does not clear the configured token by accident.

**TM-FCP-008, Session ownership invariant:**
`SessionService::create_from_app` sets `session.owner_principal_id = app.owner_principal_id` and tags every session with `fcp:app:<app_public_id>`. The reuse path (`find_app_session_by_tags`) requires the `org_id`, `app.internal_id`, AND the routing tag to all match, so even an attacker who guesses a victim app's id and forges a cookie cannot adopt a session owned by a different org or app.

## 24. User Hooks (TM-HOOK)

User-authored shell commands run at lifecycle and tool events via the
`user_hooks` capability and any capability that returns `UserHookSpec`
entries from `user_hooks()`. See `knowledge/runtime-resources/user-hooks.md`.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-HOOK-001 | Hook-as-injection-amplifier: model-controlled file at the hook command path lets prompt-injected agent influence hook behavior | High | Hooks execute through `bashkit_shell` against the session VFS, identical FS isolation as the `bash` tool. Operators who reference scripts from agent-writable paths (`/workspace`) opt in to that risk; the recommended pattern is to inline the command or read scripts from read-only capability mounts | MITIGATED |
| TM-HOOK-002 | Hook-as-exfil-channel: a hook command makes outbound network calls to leak session state | High | The hook dispatcher builds its own `Bash` (`integrations/bashkit/src/hook_dispatch.rs`) without `.network(...)` or an HTTP transport, so `curl`/`wget` have no network path in the hook vocabulary even though the integration compiles bashkit's `http_client` feature. Outbound HTTP exists only on the `bashkit_shell` tool path, gated by per-capability `enable_http` config and routed through `EgressService` (see TM-BASH-003). Re-evaluate if hook dispatch ever opts into `configure_http` | MITIGATED |
| TM-HOOK-003 | Stdout poisoning: a long-running or malicious hook fills stdout with bogus JSON or floods to deny tool execution | Medium | Per-hook timeout (default 5 s, max 30 s) + 64 KiB combined stdout/stderr cap (reuses `OutputHardLimitHook` ceiling) + `on_error` policy (`block`/`allow`/`warn`). Overrun is treated as an executor error, not a decision | MITIGATED |
| TM-HOOK-004 | Privilege escalation via capability contribution: a built-in capability ships hooks that exfiltrate or block | High | The built-in `user_hooks` capability is permanently `High` and admin-gated on assignment via `check_high_risk_caps`. Capability-contributed hooks are surfaced in audit logs with their `{capability_id}:{name}` `HookId` so operators can locate and mute them via the `disabled_contributions` list on a sibling `user_hooks` config. Declarative-capability-contributed hook bundles (with the matching auto-elevation rule) are **not yet implemented**: see `knowledge/runtime-resources/user-hooks.md` for the deferred path | MITIGATED |
| TM-HOOK-005 | Hook chain DoS via fan-out across many configured hooks | Medium | Per-hook timeout caps wall-clock; hook execution is serial within a single event firing; combined chain wall-clock for `pre_tool_use` is bounded by `Σ timeout_ms` which is itself bounded by `(MAX_HOOK_TIMEOUT_MS × N hooks)`. Operators set the contributing capability list, capping `N` in practice | MITIGATED |
| TM-HOOK-006 | Future risk: declarative-capability hook bundles bypass the admin gate | High | Declarative `user_hooks` are deferred (no field on `DeclarativeCapabilityDefinition` today). The path is reserved: when added, the declarative-capability write API must compute effective risk including `user_hooks` and force `RiskLevel::High` when the array is non-empty. Threat tracked here so the contract lands with the feature | **OPEN** (deferred) |

### Mitigation Details

**TM-HOOK-001, Path trust model:** Hook commands are interpreted as
bash command lines. When the command references a script from the
session VFS (e.g. `bash /workspace/scripts/fmt.sh`), the operator is
trusting the contents of that path. Recommended patterns:

1. Inline the command directly in `executor.command`.
2. Mount scripts read-only via a capability `mounts()` declaration so
   the agent cannot rewrite them mid-session.
3. Reference scripts from a known-safe path that the agent has no
   tools to write to (e.g. `/.agents/hooks/...` under a capability
   read-only mount).

**TM-HOOK-002, Egress inheritance:** `BashHookExecutor` does not
construct a separate `NetworkAccessList`; the session sandbox supplies
the same policy `bashkit_shell` honors. There is no way to "opt out" a
hook command from session egress controls without explicit operator
action on the agent.

**TM-HOOK-004, Capability-contributed hooks:** Today the only
in-tree contributor surface is the built-in `user_hooks` capability,
which carries `RiskLevel::High` and is gated on admin assignment via
`check_high_risk_caps`. Capability authors that override
`Capability::user_hooks` / `user_hooks_with_config` to ship hook bundles
also ride the trust gate of having their capability assigned to an agent.
The runtime collection path (`finalize_hook_specs`) stamps every
non-`user_hooks` contribution into the `{capability_id}:` `HookId`
namespace, capability authors cannot forge the `user:` namespace, and
drops any contribution whose id appears in a sibling `user_hooks`
capability's `disabled_contributions` list, so an operator can always
mute a bundled hook without removing the contributing capability.

**TM-HOOK-006, Deferred contract:** When declarative
`user_hooks` lands, the capability-write API must compute effective
risk *including* `user_hooks` and force `RiskLevel::High` when the
array is non-empty. The elevation must be unconditional and not
downgradeable by the author. The Linear ticket linked from the
`knowledge/runtime-resources/user-hooks.md` follow-up list tracks this work.

## 25. CI / Build Pipeline (TM-CI)

GitHub Actions workflows in `.github/workflows/` are part of the trust boundary because they execute on push to `main` and on fork pull requests once the first-time-contributor approval is granted. After that one-time approval, every subsequent PR from the same fork runs CI automatically; the only barrier between attacker-controlled code and the repo's secrets is the per-workflow secret scoping.

Key GitHub guarantee: for `pull_request` triggers, the workflow YAML is read from the BASE branch, so a fork PR cannot directly add an exfiltration step to a workflow. The risk is indirect, workflow YAML in the base branch may execute PR-controlled code (cargo build/test runs `build.rs`, proc-macros, and test bodies; PR-built server/worker/CLI binaries execute) with secrets injected into the process env. Any secret available to that code is exfiltratable.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-CI-001 | Workflow-level `DOPPLER_TOKEN` exfil via PR-controlled cargo build/test | Critical | `DOPPLER_TOKEN` is no longer declared at workflow `env:` in `ci.yml`, `brave-search-integration.yml`, `cursor-integration.yml`, or `sprites-integration.yml`. It is injected only into jobs/steps gated on `github.event_name == 'push'` (live-test jobs in ci.yml + per-integration live-test jobs). The Slack step and the `Run LLM integration tests` step in `integration-test` both inject `DOPPLER_TOKEN` via step-scoped env only, each guarded by `if: github.event_name == 'push'` | MITIGATED |
| TM-CI-002 | LLM provider keys (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY`) exfil from `integration-test` job env via PR-controlled cargo test | High | No provider keys are declared at `integration-test` job env. The single step that needs them (`Run LLM integration tests`) gates on `github.event_name == 'push'` and obtains keys from Doppler at runtime via a step-scoped `DOPPLER_TOKEN` (`doppler run`), so the keys never enter a runner executing PR-controlled code | MITIGATED |
| TM-CI-003 | LLM provider keys exfil from PR-built `everruns-server`/`everruns-worker` binaries in `workflow-test` | High | Vector eliminated: the dead `ANTHROPIC/OPENAI/GEMINI_API_KEY` job-env refs were removed (`workflow_test.rs` runs on LlmSim, no keys needed). Job remains `github.event_name == 'push'`-gated; compile-time PR coverage preserved via `build-binaries` | MITIGATED |
| TM-CI-004 | LLM provider keys exfil via `DEFAULT_*_API_KEY` from PR-built CLI in `cli-e2e-test` | High | Vector eliminated: the dead `DEFAULT_*_API_KEY` job-env refs were removed (`cli-e2e-test` runs under `DEV_MODE`/LlmSim). Job remains `github.event_name == 'push'`-gated; PR-side CLI coverage continues via `unit-test`'s CLI integration tests | MITIGATED |
| TM-CI-005 | Brave Search live tests exposing Doppler vault on fork PRs | High | `brave-search-integration.yml` split into a PR-safe `unit-test` job (no secrets) and a push-only `live-test` job. Weekly `integration-live-sweep.yml` backstops shared-crate regression coverage | MITIGATED |
| TM-CI-006 | First-time-contributor approval grants persistent CI access | Medium | GitHub setting "Require approval for all outside collaborators" recommended at the org/repo level. Not enforced in this repo's workflows; orthogonal to the per-secret scoping above | **OPERATIONAL** |
| TM-CI-007 | `GITHUB_TOKEN` exfil on PR | Low | GitHub scopes the fork-PR `GITHUB_TOKEN` to read-only by default. `docker-publish.yml` uses it only on `push`/tag jobs; PR-validation jobs do not log in to GHCR | MITIGATED |
| TM-CI-008 | Shell injection through release tag inputs in write-scoped publish jobs | High | `cli-binaries.yml` and `docker-publish.yml` pass dispatch/tag values through step environment variables and quote every shell expansion; no attacker-controlled `${{ inputs.* }}` or tag expression is interpolated directly into `run:` scripts. Release/tag validation remains ahead of credentialed publication. Third-party actions in these write-scoped workflows are pinned to reviewed commit SHAs. | MITIGATED |

### Mitigation Details

**TM-CI-001 / TM-CI-002 / TM-CI-003 / TM-CI-004, `pull_request` vs `push` gating:**
The four workflows touched in this category previously declared secrets at workflow `env:` or at job `env:` on jobs that ran on `pull_request`. After the fix, every step that has any secret in its env satisfies one of:
- The enclosing job condition includes `github.event_name == 'push'` (or `workflow_dispatch`), or
- The step itself has an `if: github.event_name == 'push'` guard, and the secret is set at step `env:` only.

This ensures GitHub never instantiates the secret value into a runner that is executing fork-PR-controlled code (build.rs, proc-macros, test bodies, or PR-built binaries).

**TM-CI-006, Outside-collaborator approval gate:**
Even with per-secret scoping, an attacker who controls a previously-approved fork can submit malicious PRs that the workflow base-branch YAML still runs. The remaining defense layer is the GitHub org setting "Require approval for all outside collaborators" under Actions → General → Fork pull request workflows. This is a repo/org-level operational control, not a workflow change, and is therefore tracked here for visibility.

## 26. Plugins (TM-PLUGIN)

Installed plugins (`knowledge/integrations/plugins.md`) compile third-party remote content into agent context: marketplace catalogs and plugin directories are fetched from external sources and become system prompt text, skills, and scoped MCP config. This is a supply-chain and prompt-injection surface layered on the declarative-capability model; everything compiled passes the same declarative validation (size/count limits, text-only files, traversal rejection, scoped-MCP URL checks).

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-PLUGIN-001 | Prompt injection via marketplace plugin content (agents/skills/commands compiled into system prompt and skill mounts) | High | Same class as TM-AGENT-001/002, no complete defense. Adding marketplaces and installing plugins are admin-gated org actions; content is pinned at install time and reviewable; install warnings surface dropped components | **ACCEPTED** |
| TM-PLUGIN-002 | Supply-chain mutation of an installed plugin (upstream force-push or tag move) | High | GitHub installs pin a commit SHA at install time; the running capability is the persisted compiled definition, never re-fetched implicitly; updates are explicit, re-fetched at the marketplace's current synced SHA, and recompiled through full validation | MITIGATED |
| TM-PLUGIN-003 | SSRF via `url` marketplace source or plugin-declared MCP servers | High | `url` sources are HTTPS-only and SSRF-validated before fetch; portable `mcp.json` and legacy `.mcp.json` pass the same SSRF-safe scoped-MCP validation as agent/harness `mcpServers` (TM-MCP) | MITIGATED |
| TM-PLUGIN-004 | Resource exhaustion via oversized catalog or plugin archive (zip bomb) | Medium | Size cap on fetched `marketplace.json`; plugin tarballs have bounded compressed downloads, bounded whole-archive decompression and entry iteration before subdirectory filtering, and per-file (128 KB), retained-total (4 MB), and retained-file-count (256) caps; symlink/hardlink entries are dropped; compiled contributions retain their stricter declarative limits | MITIGATED |
| TM-PLUGIN-005 | Code-execution smuggling via plugin components | High | v1 compiles data-only contributions; `hooks`, `lspServers`, `monitors` and other executable components are dropped with install warnings and never executed server-side; MCP tools execute remotely under existing TM-MCP controls | MITIGATED |
| TM-PLUGIN-006 | Server filesystem read via `local_path` marketplace source | High | `local_path` is rejected unless the deployment is dev-grade (`DeploymentGrade::from_env().is_dev()`); production deployments only accept `github`/`url` sources | MITIGATED |
| TM-PLUGIN-007 | Typosquatting / spoofed plugin names in an org's marketplaces | Medium | Marketplace registration is admin-gated; plugin and marketplace names are unique per org; no global plugin namespace exists in v1, so impersonation requires an admin to register the hostile marketplace | **ACCEPTED** |
| TM-PLUGIN-008 | Plugin binds an OAuth MCP server to another provider's tokens (e.g. `github`) or smuggles key material | High | The compiler drops any plugin-supplied `oauth_provider_id` and `api_key` with a warning; the provider id is assigned server-side at install from a host-created anchor `mcp_servers` row (`plugin:` install path), so plugin content can only mark a server as OAuth, never choose whose tokens it reads | MITIGATED |
| TM-PLUGIN-009 | Active SVG content or remote icon metadata executes script, loads tracking resources, or creates a stored-XSS surface | High | Plugin icons must be relative bundled UTF-8 SVG files within existing file-size limits; compilation rejects traversal, remote/data URLs, scripts, event handlers, active elements, styles, and external references, then embeds accepted bytes as a bounded data URL. The UI only renders that exact embedded SVG media type in an `img` context and falls back to a local neutral glyph for every other value | MITIGATED |
| TM-PLUGIN-010 | Uninstall/reinstall silently rebinds stale agent assignments to different plugin content sharing the same manifest name | High | Server-managed capability refs use the org-scoped plugin installation public ID, not the reusable manifest name. Explicit updates preserve the row and ID; uninstall/reinstall creates a new ID, and validation/hydration resolve only the exact active installation public ID | MITIGATED |

### Mitigation Details

**TM-PLUGIN-002, Pinned compile-at-install model:**
The capability that agents execute is the compiled `definition` JSONB persisted in `plugin_installs` at install/update time. Upstream changes to the source repository have no effect on running agents until an admin explicitly updates, at which point the content is re-fetched at the marketplace's current synced SHA and re-validated end to end. This is the same trust shape as a lockfile: sync moves the candidate version; update moves the installed one.

## 27. Agentic Resource Discovery (`resource_discovery`)

The `resource_discovery` capability (`crates/ard/SPEC.md`, crate `everruns-ard`) lets a running agent discover external capabilities from ARD registries (the [ARD spec](https://agenticresourcediscovery.org/spec/)) and attach them mid-session. `discover_resources` proxies the registry `POST /search` (outside model context, results cached in session KV `ard_disco:`); `attach_resource` resolves a cached entry, runs a trust gate, and persists an attachment to session KV `ard_attach:`. During turn-context assembly, `everruns_core::ard_attachment::apply_session_attachments` folds attachments into the session config layer in **both** the server `GetTurnContext` and the in-process runtime `load_resolved_turn`: MCP entries become a session-scoped `mcpServers` record (tools appear next turn, prefixed `mcp_<name>__*`, subject to `tool_search`), and A2A entries merge into the session's `a2a_agent_delegation` config so the existing `spawn_agent` flow can target them. The capability is experimental/Dev-only and `risk_level` High. Registry-returned text is untrusted external data (same class as TM-TOOL-005 / TM-AGENT-002).

This section reuses the existing TM-API, TM-TOOL, TM-AGENT, and TM-DOS categories rather than introducing a new prefix.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-API-021 | SSRF to internal services via a registry-resolved artifact or endpoint URL | High | The model never supplies a URL, it selects a `registry_id` from the capability's `registries` allowlist (`{id,url,federation}`), and attach targets resolve from cached registry entries. Every resolved registry, MCP server, and A2A endpoint URL passes `validate_url_dns_pinned` / `validate_safe_url` (loopback, RFC1918, link-local, cloud metadata blocked; resolve-then-check with DNS pinning) before any outbound call. `allow_local_urls` (default false) relaxes this only for tests/dev. | MITIGATED |
| TM-API-022 | Registry allowlist bypass via model-supplied registry URL | High | `discover_resources` accepts only an optional `registry_id` that must match an entry in the configured `registries` allowlist; raw URLs in tool arguments are rejected. No code path lets the model introduce a registry the operator did not configure. | MITIGATED |
| TM-TOOL-022 | Forged or spoofed attachment via direct KV write | High | ARD attachments live under reserved KV prefixes `ard_attach:` / `ard_disco:`. `is_internal_session_kv_key` blocks these prefixes from the user-facing `kv_store` tool, so the agent cannot fabricate an `ArdAttachment` that `apply_session_attachments` would later fold into the session config. Only `attach_resource` (after the trust gate) writes them. | MITIGATED |
| TM-TOOL-023 | trustManifest spoofing, attaching an MCP/A2A resource whose publisher identity does not match its URN | High | Before attach, the trust gate binds the URN publisher FQDN to the `trustManifest` identity domain (e.g. `spiffe://acme.com/...` must match identity domain `acme.com`) and enforces the capability's `require_trust` attestations (e.g. `["soc2"]`). Domain↔URN mismatch, missing required attestation, or missing manifest when required rejects the attach. `allow_attach_types` further restricts which artifact media types (`application/mcp-server+json`, `application/a2a-agent-card+json`) may be attached, and the value-or-reference envelope is validated before resolution. | MITIGATED |
| TM-AGENT-025 | Prompt-injection-driven attach storm (untrusted registry text steers the agent into attaching many or hostile resources) | Medium | Registry search results are untrusted external data returned via the `tool_result` role (no complete defense, same class as TM-AGENT-002). `max_attachments` (default 5) bounds the number of resources attachable per session; `attach_resource` is idempotent per URN so repeated injected requests cannot inflate the count; the trust gate and allowlist constrain *what* can attach. The agent loop, iteration limits, and `tool_search` deferral are unchanged. | MITIGATED |
| TM-DOS-019 | Resource exhaustion via unbounded attachments or oversized discovery caches | Medium | `max_attachments` caps attachments per session; cached discovery and attachment entries are bounded by normal session-KV limits; `discover_resources` runs outside model context so large registry responses do not inflate the prompt directly. | MITIGATED |

## 28. App API Keys / api_endpoint Channel (TM-APIKEY)

App-scoped, execution-only API keys (`evr_app_...`) over native session routes
mounted under the app (`/v1/apps/{app_id}/api/{channel_id}/...`). Structurally
execution-only: the key authenticates only these app-mounted routes and has no
path to any management API. Mitigations live in
`crates/server/src/api/app_api.rs` and
`crates/server/src/domains/apps/commands.rs`. See `knowledge/integrations/app-api-keys.md`.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-APIKEY-001 | API key brute force / timing oracle / plaintext leak | Medium | Keys are 32 random bytes (256-bit entropy) prefixed `evr_app_`; stored only as SHA-256 hex; plaintext returned exactly once at create / regenerate and never persisted; the hash is redacted from channel read responses (`redact_channel_config`). Inbound keys are hashed and compared constant-time (canonical `security::constant_time_eq`) after the published-app + enabled-channel gate, before any session work. Rotation (`regenerate_api_endpoint_app_channel_key`) overwrites both hash and prefix so the previous key fails on the next request | MITIGATED |
| TM-APIKEY-002 | Cross-app session access via a leaked session id | High | Every read / message / cancel against a `{session_id}` verifies the session carries this channel's `app:<public_id>` + `app_channel:<public_id>` routing tags (`session_has_app_channel_tags`); a mismatch returns `404` (not `403`) so existence is not leaked. An execution key for one app cannot read or drive another app's sessions. On create, the session is bound to the app's own Harness + Agent, the caller cannot select an arbitrary agent or pass management-only fields | MITIGATED |
| TM-APIKEY-003 | DoS via runaway execution-key client | Medium | App owners configure a per-app, per-IP cap via `ApiEndpointChannelConfig::rate_limit_per_minute`, enforced in `app_api::authenticate_request` after the key comparison so an unauthenticated caller cannot grow the limiter cache or probe channel existence. Backed by the shared `ChannelRateLimiter` with a dedicated `apikey` namespace (disjoint from `agui` / `a2a` / `fcp`); scope is `app_id:channel_id` so multiple keys on one app keep independent buckets. Field capped at `1_000_000`; `0` / `None` falls back to the global API limit. Same fail-open behavior on Valkey outage as TM-A2A-013 | MITIGATED |
| TM-APIKEY-004 | Execution key reads raw internal tool detail | Medium | `GET .../sessions/{id}` returns only completed, non-Commentary assistant messages (`output.message.completed`) plus a derived turn status. Raw tool names, arguments, results, and internal event bodies are never returned to the key, the same safe projection AG-UI applies to public streams, achieved here by returning final messages rather than a raw event feed | MITIGATED |
| TM-APIKEY-005 | Anonymous ingress to draft / disabled channels | High | Published-app + enabled-channel + channel-type checks run before key validation in `authenticate_request`; draft/disabled/unknown all collapse to generic 403/404 so the endpoint is not a probe oracle | MITIGATED |
| TM-APIKEY-006 | Cross-org session reuse via tag spoofing | High | Inherits the shared app-invocation mitigation (TM-A2A-007): shared sessions are matched by org + owner principal + tag set in `find_app_session_by_tags_and_owner`, so a user cannot pre-seed an `app:` / `app_channel:` tagged session for an execution-key invocation to reuse | MITIGATED |

## Vulnerability Summary

### Open Threats (Require Action)

| ID | Threat | Severity | Recommendation |
|----|--------|----------|----------------|
| ~~TM-API-008~~ | ~~WebFetch SSRF to internal services~~ | ~~High~~ | Mitigated: fetchkit v0.1.2 DnsPolicy blocks private IPs via resolve-then-check |
| ~~TM-API-009~~ | ~~WebFetch cloud metadata access~~ | ~~Critical~~ | Mitigated: fetchkit v0.1.2 blocks 169.254.0.0/16; IMDSv2 recommended as defense-in-depth |
| ~~TM-API-010~~ | ~~WebFetch internal DNS probing~~ | ~~Medium~~ | Mitigated: fetchkit v0.1.2 resolve-then-check blocks private IP resolution |
| ~~TM-API-011~~ | ~~WebFetch internal port scanning~~ | ~~Medium~~ | Mitigated: fetchkit v0.1.2 blocks private IP ranges |
| ~~TM-API-012~~ | ~~WebFetch DNS rebinding~~ | ~~Medium~~ | Mitigated: fetchkit v0.1.2 DNS pinning prevents rebinding |
| ~~TM-AUTH-001~~ | ~~No rate limiting on login~~ | ~~High~~ | Mitigated: Per-IP rate limiting on auth endpoints with dual backend (in-memory/Valkey) |
| TM-AUTH-007 | OAuth state not validated | High | Store state in cookie; validate in callback |
| ~~TM-AUTH-015~~ | ~~JWT secret insecure default~~ | ~~High~~ | Mitigated: startup panics if `AUTH_JWT_SECRET` unset in admin/full/external mode; random per-process secret in none mode; hardcoded `insecure-dev-secret-change-me` fallback removed |
| ~~TM-DURABLE-002~~ | ~~gRPC auth optional, no org scoping~~ | ~~High~~ | Mitigated: Bearer token required in production + optional mTLS; workers cross-org by design |
| ~~TM-DURABLE-010~~ | ~~Durable API endpoints unauthenticated~~ | ~~High~~ | Mitigated: All endpoints require AuthUser extractor |
| ~~TM-TENANT-008~~ | ~~User listing cross-org~~ | ~~High~~ | Mitigated: `GET /v1/users` uses `ResolvedOrg` and `list_users_by_org` |
| ~~TM-DOS-008~~ | ~~ReDoS via file grep endpoint~~ | ~~Medium~~ | Mitigated: pattern length capped at 1 000 chars; NFA compiled size capped at 512 KB via `RegexBuilder::size_limit`; per-file scan skipped above 512 KB; total scan aborted above 5 MB per request (`grep_session_files`, `service.rs`) |
| TM-AGENT-007 | No per-iteration tool call limit | Medium | Cap tool calls per LLM response |
| ~~TM-AGENT-012~~ | ~~Tool result size amplification~~ | ~~Medium~~ | Mitigated: 64 KiB hard limit via `OutputHardLimitHook` (EVE-225) |
| ~~TM-FS-008~~ | ~~No session storage quota~~ | ~~Medium~~ | Mitigated: per-session (500 MB) and per-file (100 MB) byte quotas enforced in `WorkspaceFileService` and `DirectWorkerAdapters`; env-configurable (EVE-510) |
| TM-TOOL-008 | Tool approval not enforced | Low | Implement HITL approval for requires_approval policy |
| ~~TM-TOOL-009~~ | ~~No tool rate limiting~~ | ~~Medium~~ | Mitigated: per-org 1000 RPM via `OutboundToolRateLimiter` in `ActAtom` (EVE-514) |
| TM-DOS-003 | SSE connection exhaustion | Medium | Global (10k), per-org (1k), per-session (5) limits enforced |
| TM-AGENT-016 | Plaintext secrets in chat history | Medium | Prefer Settings UI; phase out in-chat secret collection |
| TM-AGENT-017 | Agent-initiated entity management | High | Add RBAC for platform management; audit logging; recursion depth limits |
| ~~TM-AGENT-018~~ | ~~No outbound URL filtering on web_fetch~~ | ~~Medium~~ | Mitigated: `NetworkAccessList` layers + system allowlist enforced at the egress boundary; outbound call audit logging still open |

### Accepted Risks

| ID | Threat | Rationale |
|----|--------|-----------|
| TM-AUTH-010 | Admin password in env var | Limited to development mode; documented |
| TM-AUTH-011 | Auth bypass in none mode | By design for local development; gated to dev deployments + unknown AUTH_MODE rejected at startup (EVE-621) |
| ~~TM-API-008~~ | ~~WebFetch SSRF~~ | Reclassified to **MITIGATED**: fetchkit v0.1.2 DnsPolicy blocks private IPs |
| TM-FS-006 | File content unencrypted at rest | Relies on infrastructure encryption |
| TM-FS-007 | No file access audit log | Privacy tradeoff; not compliance-required |
| TM-LLM-007 | Indirect prompt injection | Inherent LLM limitation; mitigated by role separation |
| TM-AGENT-001 | Direct prompt injection | Inherent LLM limitation; role separation + iteration limits |
| TM-AGENT-002 | Indirect prompt injection via tool results | Tool results role-separated; no complete defense exists |
| TM-AGENT-003 | MCP tool description poisoning | MCP servers org-configured; descriptions used as schemas only |
| TM-AGENT-013 | Data exfiltration via web_fetch | Opt-in capability; org members trusted; intended functionality |
| TM-AGENT-019 | Internal network probing via high-risk execution capabilities | High-risk capabilities are Admin-gated; residual exposure depends on deployment network isolation |
| TM-DURABLE-006 | DLQ growth | Tasks preserved for debugging; manual cleanup |
| TM-DAYTONA-001 | Git token on sandbox disk | Same trust boundary as exec; `/tmp` cleared on stop; short-lived token |
| TM-DENO-004 | Network probing from Deno sandbox | Same residual risk as other remote execution capabilities; requires Admin + operator egress controls |
| TM-E2B-005 | Full-network sandbox misuse | Same residual risk class as other cloud sandboxes; require deployment egress isolation where needed |
| TM-SANDBOX-001 | Container escape via kernel vulnerability | Configurable runtime; operator chooses isolation level (sysbox/kata/gvisor for production) |
| TM-SANDBOX-003 | SSRF / metadata / internal-network access from sandbox | No in-product egress filtering; operator must restrict egress (block RFC1918 + 169.254.169.254) at the network/firewall layer |

### Caller Responsibilities

| Responsibility | Related Threats | Description |
|---------------|-----------------|-------------|
| Enable TLS/HTTPS | TM-AUTH-005, TM-LLM-006 | All production traffic must use HTTPS |
| Database TLS | TM-API-001 | Use `sslmode=require` in `DATABASE_URL` for production; no code-level enforcement |
| Secure env vars | TM-AUTH-002, TM-CRYPTO-001 | Never commit secrets to source control |
| Configure CORS | TM-API-007, TM-WEB-007 | Set explicit allowed origins in production |
| Network isolation | TM-DURABLE-002 | Keep gRPC port 9001 on private network; set `WORKER_GRPC_AUTH_TOKEN`; configure mTLS via `WORKER_GRPC_TLS_*` for production |
| Evaluate Braintrust | TM-OBS-001 | Assess data classification before enabling |
| Secure OTLP endpoint | TM-OBS-003 | Use trusted internal infrastructure only |
| OAuth provider trust | TM-AUTH-012 | Verify email ownership at OAuth providers |
| Review agent capabilities | TM-AGENT-005, TM-AGENT-013, TM-AGENT-019 | High-risk capabilities require Admin role; audit capability assignments for org admin accounts |
| System prompt review | TM-AGENT-004 | Review agent system prompts for jailbreak patterns before deployment |
| Block cloud metadata | TM-API-009 | Defense-in-depth: enable IMDSv2 (AWS), metadata concealment (GCP), or equivalent; fetchkit v0.1.2 blocks 169.254.0.0/16 at application level |
| Worker network isolation | TM-API-008, TM-API-010, TM-API-011 | Defense-in-depth: restrict worker container egress; fetchkit v0.1.2 blocks private IPs at application level |
| Sandbox/container egress isolation | TM-AGENT-019, TM-E2B-005 | Restrict Daytona, E2B, and any Docker-backed execution environment from reaching internal networks unless explicitly intended |
| Review GitHub App permissions | TM-DAYTONA-003 | Audit which repositories the GitHub App installation can access; Everruns does not enforce per-repo restrictions |

## Security Controls Matrix

| Control | Category | Implementation |
|---------|----------|----------------|
| Authentication | TM-AUTH | JWT (15 min), personal access tokens (SHA-256), OAuth, Argon2id passwords |
| Authorization | TM-TENANT, TM-AUTHZ | Org-scoped queries, ResolvedOrg extractor, 404 on cross-org; `Command::run` enforcement via `policy.evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)`, role→permission mapping |
| Encryption at rest | TM-CRYPTO | AES-256-GCM envelope encryption for API keys |
| Encryption in transit | TM-LLM | HTTPS for all external communication |
| Input validation | TM-API | Size limits, path validation, regex constraints |
| SQL injection prevention | TM-API | sqlx prepared statements (parameterized queries) |
| SQLite sandboxing | TM-SQL | Authorizer callback, VFS isolation, resource limits |
| Bash sandboxing | TM-BASH | Bashkit WASM-like isolation, VFS adapter, resource limits |
| Session isolation | TM-FS, TM-SQL | FK constraints, session-scoped storage |
| Agent loop controls | TM-AGENT | Max iterations, tool registry, session-scoped tools, no self-modification |
| Error sanitization | TM-API, TM-OBS | Generic error messages, server-side logging only |
| Cookie security | TM-WEB | HTTP-only, SameSite=Lax, Secure flag in production |
| Tool validation | TM-TOOL | Registry-based validation, defensive MCP parsing, skill archive validation |
| Resource limits | TM-DOS, TM-BASH | Input sizes, iteration limits, query timeouts, bash limits |
| Task ownership | TM-DURABLE | Verified on completion, heartbeat-based reclaim |
| Daytona sandbox isolation | TM-DAYTONA | Session-scoped secrets, encrypted API key, auto-stop, short-lived git tokens |
| E2B sandbox isolation | TM-E2B | Session-scoped secrets, envd access tokens, timeout refresh, leased-resource cleanup |
| Slack webhook forgery | TM-SLACK-001 | HMAC-SHA256 signing secret verification, 5-min replay window |
| Slack bot loop | TM-SLACK-002 | Skip events with `bot_id` or `subtype` to prevent infinite loops |
| Slack signing secret exposure | TM-SLACK-003 | Stored in `channel_config` (org-scoped access), not logged |
| A2A API key forgery | TM-A2A-001, TM-A2A-002 | SHA-256 hashed at rest, constant-time compare, 128-bit entropy |
| A2A method abuse | TM-A2A-005 | Allowlist of one (`message/send`); other methods rejected before session creation |
| A2A Agent Card disclosure | TM-A2A-009 | Card never echoes API key / hash / internal IDs; only published while app is live and channel enabled |
| A2A runaway client DoS | TM-A2A-013 | Configurable per-app, per-IP cap (`A2aChannelConfig::rate_limit_per_minute`) enforced after API key check via shared `ChannelRateLimiter` (in-memory governor or Valkey) |
| A2A replay of captured request | TM-A2A-010 | Opt-in scope-bound HMAC signing (`A2aChannelConfig::signing_secret`), basestring `v0:{ts}:{app_id}:{channel_id}:{body}` so signatures are non-reusable across channels; 5-minute timestamp window plus signature-keyed dedup; in-memory or Valkey backend |
| ARD discovery/attach controls | TM-API-021, TM-API-022, TM-TOOL-022, TM-TOOL-023, TM-AGENT-025, TM-DOS-019 | Registry allowlist (no model-supplied URLs), `validate_url_dns_pinned`/`validate_safe_url` on every resolved URL, trustManifest domain↔URN binding + `require_trust` gate, reserved KV prefixes (`is_internal_session_kv_key`), `max_attachments` bound, untrusted registry text role-separated |
| CI secret scoping | TM-CI-001..005 | No `secrets.*` at workflow `env:`; secrets are injected only into jobs/steps gated on `github.event_name == 'push'` (or `workflow_dispatch`) so fork-PR-controlled cargo/binary code cannot read them from process env |

## References

- `knowledge/security/security-testing.md`, Security testing process (threat-model tests, fail-rs failure injection, DeepSec scanning, supply-chain checks)
- `SECURITY.md`, Vulnerability disclosure policy
- `knowledge/security/authentication.md`, Authentication modes, JWT, personal access tokens, OAuth
- `knowledge/security/encryption.md`, Envelope encryption design
- `knowledge/security/multitenancy.md`, Org-based isolation model
- `knowledge/runtime-resources/workspace.md`, Session file storage and path validation
- `knowledge/runtime-resources/session-sqldb.md`, SQLite sandbox and VFS design
- `knowledge/execution/tool-execution.md`, Tool types and execution flow
- `knowledge/integrations/mcp-servers.md`, MCP server integration
- `knowledge/foundations/llm-drivers.md`, LLM provider abstraction
- `knowledge/operations/durable-execution-engine.md`, Workflow engine and worker communication
- `knowledge/operations/scheduled-tasks.md`, Cron-based task scheduling
- `knowledge/operations/observability.md`, OpenTelemetry and Braintrust observability providers
- `knowledge/execution/apis.md`, HTTP API endpoints and error handling
- `knowledge/execution/capabilities.md`, Agent capabilities system
- `knowledge/execution/bashkit-requirements.md`, Bashkit integration requirements
- `integrations/daytona/SPEC.md`, Daytona cloud sandbox integration
- `integrations/deno/SPEC.md`, Deno sandbox integration
- `integrations/e2b/SPEC.md`, E2B cloud sandbox integration
- `knowledge/execution/client-side-tools.md`, Client-side tools for API/SDK consumers
- `knowledge/integrations/apps.md`, Apps system (agent deployment to channels)
- `crates/server/specs/slack-integration.md`, Slack bot integration
- `integrations/brave-search/SPEC.md`, Brave Search web search integration
- `crates/ard/SPEC.md`, Agentic Resource Discovery (`resource_discovery`) client capability
- `knowledge/runtime-resources/infinity-context.md`, Unlimited conversation length via context management
- [fetchkit v0.1.2 source](https://crates.io/crates/fetchkit), SSRF protection (resolve-then-check, DNS pinning, DnsPolicy), URL prefix blocking, fetch options, fetcher registry
