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

**Trust boundary 1 — User → API:** All user input is untrusted. Authentication, authorization, input validation applied here.

**Trust boundary 2 — Control Plane → Workers:** Workers are stateless executors with no database credentials. Communication via gRPC with bearer token auth (required) and optional mutual TLS (mTLS). Workers are intentionally cross-org.

**Trust boundary 3 — Workers → External Services:** LLM providers and MCP servers are external. API keys transmitted over HTTPS. MCP responses parsed defensively.

**Trust boundary 4 — LLM → Agent Tools:** The LLM decides which tools to call and with what arguments. The agent loop executes LLM-chosen tool calls within sandboxed capabilities. The LLM is semi-trusted: it operates within registered tools and iteration limits, but its outputs (tool arguments, text) are not validated for intent.

## 1. Authentication (TM-AUTH)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-AUTH-001 | Brute force login | High | Per-IP rate limiting on auth endpoints (login 10/min, register 5/min, refresh 30/min); dual backend: in-memory governor or Valkey distributed sliding-window | MITIGATED |
| TM-AUTH-002 | JWT secret compromise | Critical | Stored in env var `AUTH_JWT_SECRET`; min 32 bytes recommended; never logged | MITIGATED |
| TM-AUTH-003 | Token replay after logout | Medium | Refresh tokens stored in DB, revocable via DELETE; access tokens short-lived (15 min) | MITIGATED |
| TM-AUTH-004 | Weak password | Medium | Minimum 8 characters enforced **server-side** in `register` (`crates/server/src/auth/routes.rs`, before account lookup or creation), independent of the UI's `minLength={8}`. Argon2id hashing on storage. Covered by `test_register_rejects_short_password_via_api`. | MITIGATED |
| TM-AUTH-005 | Personal access token exposure in transit | High | HTTPS required in production; tokens prefixed `evr_pat_` for scanning | MITIGATED |
| TM-AUTH-006 | Personal access token brute force | Medium | Tokens stored as SHA-256 hashes; 128-bit entropy makes brute force infeasible | MITIGATED |
| TM-AUTH-007 | OAuth state fixation | High | State generated in `oauth_redirect`, stored in HttpOnly/Secure/SameSite=Lax cookie (`oauth_state`), validated and consumed (single-use) in `oauth_callback`; mismatch or missing cookie returns 401 | MITIGATED |
| TM-AUTH-008 | Session fixation via cookie | Medium | New tokens issued on login; HTTP-only, SameSite=Lax cookies | MITIGATED |
| TM-AUTH-009 | Refresh token theft | High | Stored hashed in DB; HTTP-only cookie; revocable | MITIGATED |
| TM-AUTH-010 | Admin password in env var | Low | Limited to admin mode; documented risk; shell history exposure possible | **ACCEPTED** |
| TM-AUTH-011 | Auth bypass in `none` mode | Info | By design for local development; anonymous user gets admin role | **BY DESIGN** |
| TM-AUTH-012 | OAuth account linking collision | Medium | Accounts linked by email; if attacker controls email at provider, they gain access | **CALLER RISK** |
| TM-AUTH-013 | Expired personal access token still in use | Medium | Expiration checked on every request via DB lookup; `last_used_at` tracked | MITIGATED |
| TM-AUTH-014 | Account enumeration via registration | Medium | Returns generic "Registration failed" for existing emails; password hash computed first for timing consistency | MITIGATED |
| TM-AUTH-015 | JWT secret insecure default | High | Falls back to hardcoded `insecure-dev-secret-change-me` if `AUTH_JWT_SECRET` unset; no startup check in production | **OPEN** |
| TM-AUTH-016 | OSS harness reseeding via public signup | High | The signup safety net in `register` / `oauth_callback` uses `state.platform_definition.built_in_harnesses()` via `initialize_org_harnesses_with_definitions`, **not** `oss_built_in_harnesses()`. An operator's custom `PlatformDefinition` is the source of truth — public signup cannot reintroduce OSS harnesses that were removed. Original concern tracked by PR #1462; the safety-net semantics re-added in EVE-390 preserve pre-seed correctness without re-opening the override path. | MITIGATED |
| TM-AUTH-017 | Google OAuth identity bypass | High | After `exchange_code` and before user lookup or creation, `oauth_callback` calls `oauth_identity_rejection_reason` (`crates/server/src/auth/routes.rs`). Rejects `email_verified=false` and, when `AUTH_GOOGLE_ALLOWED_DOMAINS` is set, rejects email domains not in the list (case-insensitive). Applied to both first-time and returning OAuth users. Failure path emits `auth.oauth.failure` audit with a reason and returns `403`. | MITIGATED |
| TM-AUTH-018 | Refresh-token rotation race | Medium | The previous `refresh_token` handler (`crates/server/src/auth/routes.rs`) read the refresh-token row via `get_refresh_token_by_hash` and then issued a separate `delete_refresh_token`, allowing two concurrent refreshes with the same token to both pass the read before either delete committed. The MCP OAuth refresh handler had the same get-then-delete shape for `oauth_refresh_tokens`. Both paths now use atomic consume helpers: PostgreSQL `DELETE … WHERE token_hash = $1 AND expires_at > NOW() RETURNING …` and in-memory equivalents under a single write lock. Single-use rotation is restored even under concurrency. Covered by `test_refresh_concurrent_requests_only_one_succeeds`, `test_oauth_refresh_token_rotates_and_rejects_reuse`, and `test_oauth_refresh_token_concurrent_retries_are_single_use`. | MITIGATED |
| TM-AUTH-019 | Account enumeration via login error differences | Medium | Backend `login` (`crates/server/src/auth/routes.rs`) previously returned `"Password login not available for this account"` when an OAuth-only user attempted password login, distinguishable from the unknown-email and bad-password paths. Now all credential failure branches return the same generic `Invalid email or password`. UI `apps/ui/src/app/(auth)/login/page.tsx` no longer renders raw server messages on a 401 — it shows a fixed `Invalid email or password.` so a future regression cannot leak the difference through the UI. Covered by `test_login_oauth_only_account_returns_generic_error`. | MITIGATED |
| TM-AUTH-020 | Public App endpoint auth bypass | High | AG-UI and A2A can now carry inline `channel_config.auth` with Google/OIDC JWT bearer, OAuth2 introspection, HTTP Basic, or trusted-header mTLS policy. Both ingress handlers resolve the published app + enabled channel first, then call the shared `AppEndpointAuthVerifier` before session lookup, image upload, task polling, cancellation, or message dispatch. Missing/invalid credentials return generic 401/403-style failures and do not expose provider details. Legacy AG-UI token and A2A API-key behavior remains the default only when `auth` is absent. | MITIGATED |
| TM-AUTH-021 | mTLS identity header spoofing | High | `verify_mtls` requires BOTH a configured identity header (set by the trusted reverse proxy after client-cert verification) AND a `proxy_secret`/`proxy_secret_header` shared secret that proves the request came through the trusted TLS terminator. Header-only configs (no `proxy_secret`) fail closed with `Misconfigured`. The proxy secret is stored write-only and redacted in GET responses. (EVE-545) | MITIGATED |
| TM-AUTH-022 | JWKS / OIDC discovery abuse or poisoning | High | Inline OIDC auth validates issuer/JWKS/introspection URLs with `validate_safe_url`, rejects symmetric JWT algorithms for OIDC, requires issuer + audience + exp claims, validates `nbf`, and caches discovery/JWKS for a bounded 15 minutes. Provider fetch failures fail closed before session creation. | MITIGATED |

### Mitigation Details

**TM-AUTH-001 — Rate Limiting (MITIGATED):**
Per-IP rate limiting implemented on all auth endpoints via `AuthRateLimiter` (`crates/server/src/auth/rate_limit.rs`):
- Login: 10 requests/min per IP
- Register: 5 requests/min per IP
- Refresh: 30 requests/min per IP
- **Dual backend**: In-memory (governor crate, per-instance) when `VALKEY_URL` not set; Valkey distributed sliding-window counter when set. Fail-open on Valkey errors (availability > strictness).
- **Residual risk**: Without Valkey, rate limits are per-instance. With N instances behind a load balancer, an attacker gets N× the budget. Set `VALKEY_URL` for coordinated limits in multi-instance deployments.

**TM-AUTH-002 — JWT Secret:**
```
AUTH_JWT_SECRET=<secure-random-32+-bytes>
AUTH_JWT_ACCESS_TOKEN_LIFETIME=900      # 15 min
AUTH_JWT_REFRESH_TOKEN_LIFETIME=2592000 # 30 days
```
JWT signed with HMAC-SHA256 via `jsonwebtoken` crate. Secret must never appear in logs, error messages, or source control.

**TM-AUTH-006 — Personal Access Token Storage:**
```
User sees: evr_pat_<full-random-token>    (shown once at creation)
DB stores: SHA-256(evr_pat_<full-token>)  (irreversible)
Display:   evr_pat_<first-8-chars>...      (prefix for identification)
```

## 2. Cryptography (TM-CRYPTO)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-CRYPTO-001 | KEK compromise | Critical | Stored in env var `SECRETS_ENCRYPTION_KEY`; never in source control | MITIGATED |
| TM-CRYPTO-002 | Nonce reuse in AES-GCM | Critical | Fresh 12-byte random nonce per encryption; 2^96 space | MITIGATED |
| TM-CRYPTO-003 | Ciphertext tampering | High | GCM authentication tag detects modification | MITIGATED |
| TM-CRYPTO-004 | Known-plaintext attack | Medium | Unique DEK per encryption; same plaintext produces different ciphertext | MITIGATED |
| TM-CRYPTO-005 | Stale encryption key | Medium | Key rotation supported (primary + previous KEK); key_id in payload | MITIGATED |
| TM-CRYPTO-006 | Re-encryption job missing | Low | CLI tool `reencrypt_secrets` implemented with batch processing, dry-run mode, and key rotation detection | MITIGATED |
| TM-CRYPTO-007 | Limited encryption scope | Medium | LLM API keys encrypted; system prompt encryption reverted (PII should not be in prompts) | **OPEN** |
| TM-CRYPTO-008 | Machine-payment wallet key exposure | Critical | Wallet private keys are accepted only on payment account create/update, encrypted with the server envelope encryption service, never returned from API responses, decrypted only inside `ServerPaymentAuthority` immediately before native rail signing, and never sent to workers | MITIGATED |

### Mitigation Details

**TM-CRYPTO-001 — Envelope Encryption Architecture:**
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

**TM-CRYPTO-006 — Re-encryption (MITIGATED):**
Re-encryption CLI tool implemented at `crates/server/src/bin/reencrypt_secrets.rs`. Features:
- Batch processing with configurable batch size
- Dry-run mode for safety
- Per-table filtering
- Key rotation detection via `is_current_key()`
- Full UPDATE statements to write re-encrypted data back

**TM-CRYPTO-008 — Machine-Payment Wallet Custody (MITIGATED):**
Payment accounts store wallet signing material in `credential_encrypted`, protected by the same envelope encryption service used for other secrets. The public `PaymentAccount` model intentionally omits this field. Native x402 signing happens only in `ServerPaymentAuthority`; external workers call the control-plane `ExecuteMachinePayment` RPC and never receive private keys.

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
| TM-TENANT-010 | Cross-org resource→org oracle via `/v1/resolve-org` | Medium | Endpoint requires `AuthUser` and answers only when the owning org is a membership of the caller (`is_organization_member` check before returning any identity). Unknown ids, unknown prefixes, and non-member owners all produce 404 — identical to what the entity APIs would return. Attacker learns nothing they couldn't already learn by manually switching between their own orgs. See specs/multitenancy.md (Cross-Org Resource Resolution). | MITIGATED |

### Mitigation Details

**TM-TENANT-001 — Query Isolation:**
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

**TM-TENANT-002 — Error Response Strategy:**
```rust
// Cross-org access returns 404, not 403
ApiError::NotFound("Agent not found")    // ✓ No information leakage
ApiError::Forbidden("No access")         // ✗ Reveals resource exists
```

## 3b. Permissions / Authorization (TM-AUTHZ)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-AUTHZ-001 | Default Owner role grants full access | Medium | By design for phase 1; all users are Owners. Future phases will assign roles via admin UI/invitation flow | **BY DESIGN** |
| TM-AUTHZ-002 | Policy bypass via internal Caller | Medium | `Caller::internal()` bypasses policies with Owner role; only used in gRPC service (worker ↔ server), not HTTP-accessible | MITIGATED |
| TM-AUTHZ-003 | Policy error reveals permission names | Low | 403 response includes policy ID and required permission; acceptable for debugging, no internal state leaked | **ACCEPTED** |
| TM-AUTHZ-004 | Missing policy on mutating command | Medium | Every caller (HTTP/MCP/gRPC/platform) routes through `Command::run` which evaluates `Command::policy()`. Inventory coverage test (`crates/server/tests/command_policy_enforcement_test.rs`) asserts every non-GET command declares a policy — a missing declaration fails the build | MITIGATED |
| TM-AUTHZ-005 | Anonymous app channel reaches draft, disabled, or protected app config | High | Public AG-UI ingress requires `AppStatus::Published` and an enabled `ag_ui` channel before any request work. Legacy configs then require `anonymous=true` plus the configured token when present. New inline endpoint auth (`channel_config.auth`) bypasses the legacy anonymous flag only after the shared verifier accepts the configured credential policy. Failures return before session creation or image upload. | MITIGATED |
| TM-AUTHZ-006 | Anonymous webhook reaches draft or disabled app channel | High | Public webhook ingress requires `AppStatus::Published`, a `webhook` channel, `enabled=true`, and the per-channel token before creating or reusing a session | MITIGATED |
| TM-AUTHZ-007 | HTTP callers bypass declared command policy | High | Before the command runner, HTTP adapters called `Command::execute` directly, skipping `Command::policy()`. Now all adapters call `Command::run`, which enforces policy using `ctx.permission_resolver.evaluate_with`. Tests: `run_blocks_member_from_manage_command`, `dispatch_blocks_member_from_manage_command` | MITIGATED |
| TM-AUTHZ-008 | SaaS custom `PermissionResolver` bypassed during enforcement | High | Legacy `#[policy]` macro calls `Policy::evaluate(caller)` which hardcodes `DefaultPermissionResolver`, ignoring custom resolvers. `Command::run` now threads `ctx.permission_resolver` (from `AuthState`) into `evaluate_with`, so billing-tier / per-user grant resolvers apply uniformly. Tests: `run_honors_custom_resolver_denying_owner_write`, `dispatch_honors_custom_resolver` | MITIGATED |
| TM-AUTHZ-009 | External caller spoofs `app:`/`app_channel:` session tag to attach to another app's budget | Medium | App-scoped budgets cascade onto sessions via `app:<id>` / `app_channel:<id>` tags (see specs/budgeting.md). `SessionService::create` now rejects these prefixes from non-internal callers, mirroring the existing `__internal:` reservation. Only the apps domain (which routes through `Caller::internal`) can stamp them, so an org member cannot opt their personal session into a sibling app's budget cap. | MITIGATED |
| TM-AUTHZ-010 | Disabled LLM model still reachable through resolution paths | High | `llm_models.enabled = FALSE` is enforced at every model-resolution read: `Database::get_default_llm_model`, `get_llm_model_by_model_id`, and `get_llm_model` (UUID lookup used by agent execution and validation) all add `AND m.enabled = TRUE`; the in-memory backend mirrors the same gate. Admin listing (`list_all_llm_models`) intentionally returns disabled rows so operators can re-enable them through the management UI. Test: `test_disabled_model_is_not_resolvable_or_default_postgres` (`crates/server/tests/repository_integration_test.rs`) | MITIGATED |

### Mitigation Details

**TM-AUTHZ-001 — Default Owner Role (BY DESIGN):**
Phase 1 assigns `OrgRole::Owner` as the default for all users. This means no permission-based restrictions are active in practice. This is intentional to avoid breaking existing workflows while the role assignment infrastructure is built in phase 2.

**TM-AUTHZ-002 — Internal Caller:**
`Caller::internal(org_id)` is used exclusively in `grpc_service.rs` for worker-to-server calls. The gRPC endpoint requires a bearer token in production (`TM-DURABLE-002`). HTTP handlers always construct `Caller` from `ResolvedOrg` with the user's actual role.

**TM-AUTHZ-004 — Command Runner as Single Enforcement Point:**
`Command::run` (`crates/server/src/domains/common.rs`) evaluates `Command::policy()` against the active `PermissionResolver` before dispatching to `execute`. HTTP adapters call `run`; MCP and gRPC `ExecuteCommand` route through `dispatch()` which calls `run`. Coverage is enforced by iterating `inventory::iter::<CommandDescriptor>` in a test, so new mutating commands that forget `policy()` fail the build. The legacy `#[policy]` attribute macro was removed — service-layer checks were redundant with `Command::run` and hardcoded `DefaultPermissionResolver`, re-introducing `TM-AUTHZ-008`.

**TM-AUTHZ-007 / TM-AUTHZ-008 — Historical gap:**
Prior to the command runner, only MCP/gRPC `dispatch` evaluated `Command::policy()`, and the evaluation used `Policy::evaluate` (default resolver only). HTTP adapters called `Command::execute` directly, so role-based restrictions and SaaS custom resolvers were not enforced on HTTP writes for fully-migrated domains. The runner closes both gaps in a single code path.

## 4. API Security (TM-API)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-API-001 | SQL injection | Critical | All queries use sqlx prepared statements (parameterized) | MITIGATED |
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
| TM-API-013 | LLM provider base URL SSRF | High | `url_validation::validate_url()` blocks private IPs, loopback, link-local, cloud metadata, non-HTTPS on provider create/update (EVE-69) | MITIGATED |
| TM-API-014 | Search query SQL wildcard injection | Low | LIKE wildcards (`%`, `_`, `\`) in `?search=` input are escaped; tokens capped at 8 to prevent query amplification from long inputs | MITIGATED |
| TM-API-015 | Provider secret leakage via leased-resource metadata | High | Leased-resource metadata is explicitly non-secret; cleanup reconstructs provider auth from user connections/session secrets, and session resources stay org/session scoped | MITIGATED |
| TM-API-016 | Public-endpoint internal error and tool-detail leakage | High | AG-UI streaming `RUN_ERROR` payloads route every payload-phase error through `crates/server/src/api/public.rs::PublicError`, mapping internal codes to a stable public set (`rate_limited`, `service_unavailable`, `request_too_large`, `internal_error`); raw provider strings, model IDs, HTTP status codes, quota state, and stack traces never reach the wire. Public AG-UI tool activity is translated at the endpoint boundary according to `AgUiChannelConfig.tool_visibility` (`none`, `generic`, `narrated`) and never emits raw tool names, arguments, results, or internal tool call IDs. Universal fallback is `internal_error`. Pre-stream HTTP rejections (`bad_request`, `forbidden`, `not_found`, generic 500) keep their existing texts but already avoid internal detail. Other public endpoints (Slack webhook + manifest) inherit the same contract for any payload-phase errors they add. See `specs/public-endpoints.md` | MITIGATED |
| TM-API-017 | Public AG-UI image upload abuse: oversize writes, MIME spoofing, decompression bombs | High | The public `/v1/apps/{app_id}/ag-ui/images` route caps body size at 10 MB (router `DefaultBodyLimit` plus in-handler check), validates the uploaded bytes match the declared content type via `image::guess_format` (rejecting MIME spoofing), and decodes thumbnails through `image::ImageReader` with explicit `Limits` (max width/height 20_000 px, max alloc 160 MB) so a crafted image cannot exhaust CPU or memory. Authenticated `/v1/images` retains the larger 100 MB cap behind authentication and rate limits | MITIGATED |
| TM-API-018 | Memory source credential leakage | High | Source-backed Memory creation stores only non-secret repository coordinates in `memories.source_config`; GitHub credentials resolve from user/identity connections at sync time, and generic Git URLs with inline credentials are rejected before storage. Sync failures must sanitize `last_sync_error`. | MITIGATED |
| TM-API-019 | CSV formula injection in report exports | Medium | Reporting CSV exports prefix formula-like cells (`=`, `+`, `-`, `@`, tab, CR, LF) with an apostrophe before RFC 4180 quoting so spreadsheet clients treat exported values as data, not formulas | MITIGATED |

### Mitigation Details

**TM-API-002 — Input Size Limits:**
```
Agent system_prompt: < 2 KB
Session title:       < 1 KB
Message content:     < 10 KB per part
Image upload:        < 100 MB
```
Returns `400 Bad Request: "Input exceeds allowed limits"` (generic, no detail leakage).

**TM-API-003 — Path Validation:**
```
✓ /src/main.rs
✓ /folder/file.txt
✗ ../etc/passwd        (traversal blocked)
✗ /src//main.rs        (double slash blocked)
✗ /src/\0hidden.rs     (null byte blocked)
```
Enforced at both application layer and database constraint (`session_files_path_check`).

**TM-API-008 — WebFetch SSRF (MITIGATED — fetchkit v0.1.2):**

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

**TM-API-009 — Cloud Metadata Access (MITIGATED — fetchkit v0.1.2):**
Cloud metadata at `http://169.254.169.254/` is blocked by fetchkit's DnsPolicy which blocks the entire `169.254.0.0/16` link-local range.

- **Defense-in-depth:** Enable IMDSv2 (AWS), metadata concealment (GCP), or equivalent cloud-level protections. Block 169.254.0.0/16 egress at cloud firewall.

**TM-API-010 — Internal DNS Probing (MITIGATED — fetchkit v0.1.2):**
Fetchkit's resolve-then-check validates all resolved IP addresses against blocked ranges before connecting. If an internal service name (e.g., `postgres`, `server`) resolves to a private IP, the request is blocked with `BlockedUrl` error. Error messages no longer distinguish between DNS failures and blocked addresses in a way that enables enumeration.

**TM-API-011 — Internal Port Scanning (MITIGATED — fetchkit v0.1.2):**
Private IP ranges are blocked at the DNS resolution layer. Agents cannot reach internal hosts regardless of port, eliminating the port scanning attack vector.

**TM-API-012 — DNS Rebinding (MITIGATED — fetchkit v0.1.2):**
Fetchkit v0.1.2 implements DNS pinning: the hostname is resolved once, all resolved IPs are validated against blocked ranges, and the first non-blocked IP is pinned via `reqwest::resolve()`. A second DNS lookup cannot return a different IP because the connection is pinned to the validated address.

**TM-API-015 — Leased-Resource Metadata Secrets (MITIGATED):**
The session Resources API returns leased-resource metadata to users and the UI, so this feature must not persist provider bearer tokens or equivalent secrets in `metadata`.

- Lease registration stores only non-secret metadata needed for cleanup and debugging.
- Cleanup handlers reconstruct provider auth from the original user connection or session secret store at execution time.
- Session resources are still gated by the existing org/session ownership check before rows are listed.

Code references:
- [`crates/core/src/leased_resource.rs`](/Users/mykhailochalyi/.codex/worktrees/baaf/everruns/crates/core/src/leased_resource.rs)
- [`integrations/browserless/src/session_tools.rs`](/Users/mykhailochalyi/.codex/worktrees/baaf/everruns/integrations/browserless/src/session_tools.rs)
- [`integrations/daytona/src/state.rs`](/Users/mykhailochalyi/.codex/worktrees/baaf/everruns/integrations/daytona/src/state.rs)

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
| TM-FS-009 | CLI `initial_files` hidden-path exfiltration | High | Three-layer policy in `crates/cli/src/commands/agents.rs`: hard-deny floor (`DENIED_DOT_ENTRIES`) blocks `.env`, `.ssh`, `.aws`, `.gnupg`, `.git`, etc. unconditionally; built-in `ALLOWED_DOT_ENTRIES` permits common dev assets (`.github`, `.vscode`, `.claude`, `.mcp.json`, etc.); per-agent `initial_files_allow_hidden` manifest field extends the allowlist but cannot bypass the hard-deny floor. Skipped paths emit a stderr warning. See `specs/cli.md` (Initial Files Hidden Path Policy). | MITIGATED |

### Mitigation Details

**TM-FS-001 — Defense in Depth:**
Path validated at three layers:
1. **Application:** Path parsing rejects traversal patterns
2. **Database constraint:** `session_files_path_check` CHECK constraint
3. **Unique constraint:** `(session_id, path)` prevents collision

**TM-FS-008 — Storage Quota:**
Per-session and per-file byte quotas are enforced at the application layer in both the HTTP API path (`WorkspaceFileService::create_file` / `update_file`) and the agent tool path (`DirectWorkerAdapters::write_file`). Limits are configurable via env:
- `SESSION_FILE_MAX_BYTES` — total bytes per session (default 500 MB)
- `SESSION_FILE_SINGLE_MAX_BYTES` — per-file ceiling (default 100 MB)

Writes that would exceed either limit fail with a clear error before any DB insert.

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

**TM-SQL-001 / TM-SQL-002 — Authorizer Callback:**
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

**TM-SQL-003 — Size Limits:**
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
| TM-TOOL-020 | Skill `` !`command` `` activation RCE on worker host | High | `ActivateSkillFromVfsTool::execute_with_context` never invokes `preprocess_command_injections` — the trust gate is forced off because no non-user-spoofable provenance signal exists on `SessionFile` today. Expansion is also capped at `MAX_COMMAND_PLACEHOLDERS_PER_SKILL` (32) with concurrency ≤ 4 in the expansion function itself. See EVE-388. | MITIGATED |
| TM-TOOL-021 | Agent handoff credential leakage or unauthorized target delegation | High | `agent_handoff` delegates only to configured target Agent ids, requires server-side `UserConnectionResolver` checks before start, never accepts credentials in tool args/config/resource metadata, records only non-secret provider/scope labels in `session_resources`, and rejects follow-up messages unless the child session belongs to the current parent session. Provider tools must still enforce scoped grants before real external writes. | MITIGATED |

### Mitigation Details

**TM-TOOL-002 — Defensive MCP Parsing:**
MCP tool execution flow:
1. Parse tool name: extract server name and tool name from `mcp_<server>__<tool>` format
2. Validate server exists and is not disabled
3. POST JSON-RPC `tools/call` to server URL with decrypted API key
4. Parse response (JSON or SSE format); malformed responses become tool errors
5. Convert MCP content types to internal format
6. Return to LLM as tool result

**TM-TOOL-005 — Prompt Injection Boundary:**
Tool results occupy the `tool_result` message role in the conversation. They are not concatenated into the system prompt. The LLM processes them as structured tool outputs, not instructions. However, LLMs may still be influenced by adversarial content in tool results (inherent limitation of current LLM architecture).

**TM-TOOL-010 — Skill Instruction Injection Boundary:**
When `activate_skill` is called, the full SKILL.md body is returned as a tool result wrapped in `<skill name="...">` XML tags. This maintains the tool_result role boundary. Only skill names and descriptions appear in the system prompt (via `<available_skills>` XML block), limiting the injection surface to metadata validated during upload.

**TM-TOOL-020 — Skill Command Injection Trust Gate:**
SKILL.md content may contain `` !`command` `` placeholders that, if expanded by `preprocess_command_injections`, spawn shell processes on the worker host. This is RCE against the worker if the SKILL.md body is attacker-controlled.

1. The trust signal must be a non-user-controllable provenance indicator for the SKILL.md entry read from the session VFS — for example, an origin field populated only by the capability/registry mount layer. `SessionFile::is_readonly` is **not** such a signal: both the session-files HTTP API (create/update) and `InitialFile` configuration accept `is_readonly = true` from user input.
2. Because no such provenance signal exists on `SessionFile` today, the enforcement point in `ActivateSkillFromVfsTool::execute_with_context` keeps `is_trusted_source = false` for every source. `preprocess_command_injections` is never reached at runtime.
3. The function itself is preserved (full implementation, unit-test coverage) with bounded fan-out: at most `MAX_COMMAND_PLACEHOLDERS_PER_SKILL` (32) placeholders expanded per activation, at most 4 shells concurrently. These bounds protect a future re-enable from per-activation CPU / process exhaustion.
4. SKILL.md content originating from user-facing session/file creation or update flows — including the session-files API, `initial_files`, and runtime `write_file` calls — stays untrusted regardless of metadata.
5. The single enforcement point is `ActivateSkillFromVfsTool::execute_with_context` in `crates/core/src/capabilities/skills.rs`. `preprocess_command_injections` in `crates/core/src/skill.rs` assumes the caller has already performed the trust check.
6. Command execution MUST target the session sandbox (bashkit shell) against the session virtual filesystem, not the worker host shell. The current `ProcessCommandExecutor` (which spawns host `bash -c`) is dormant scaffolding only; re-enabling command substitution without also routing it through the session sandbox would still be RCE against the worker host. Any re-enable PR must both (a) introduce the provenance signal in (1) AND (b) replace host-bash execution with a sandbox-backed executor before flipping the gate.

Follow-up work (tracked on EVE-388): (a) add a platform-controlled provenance field — e.g. a `mount_capability_id` column on `session_files` populated only by mount application code and rejected on all user-facing API paths, AND (b) replace `ProcessCommandExecutor` with a session-sandbox-backed executor (`bashkit` / managed session sandbox) so execution is confined to the session VFS. Both must land before the gate is flipped. See `specs/skills-registry.md` "Activation Substitution Pipeline" for the source/outcome matrix.

**TM-TOOL-021 — Agent Handoff Delegation Gate:**
`agent_handoff` is a high-risk orchestration tool because one agent can start a
child session using another configured Agent's tools and data. The mitigation is
to keep authority explicit and non-secret:

1. Source agents can only target ids listed in their `agent_handoff` config.
2. Required provider connections are resolved server-side through
   `UserConnectionResolver`; bearer tokens are never accepted in tool arguments,
   capability config, system prompt context, or session resource metadata.
3. Handoff resources store only non-secret target id, target agent id, provider
   ids, scope labels, and mode. Full task text and credentials are excluded.
4. `message_agent_handoff` verifies the child session's `parent_session_id`
   matches the current session before sending follow-up input.
5. This gate proves the user has the required connection before delegation.
   Real provider write tools still need scoped grant enforcement before mutating
   external infrastructure.

**TM-TOOL-011/012 — Skill Archive Validation:**
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
| TM-MCP-001 | Everruns MCP server tenant escape | Critical | The first-party `/mcp` endpoint resolves org context before every tool call; `query` exposes only read-only inventory commands; `execute` and tier-1 agent/session tools dispatch through org-scoped domain commands and `Command::run` policy checks. Regression coverage in `crates/server/tests/mcp_endpoint_test.rs` chains `discover`, `query`, `execute`, `agent_run`, `session_get_status`, and `session_send_message` against cross-org bait and verifies no read/write escape; resources/read is also covered against cross-org bait. | MITIGATED |
| TM-MCP-002 | Mutating command exposed through read-only `query` | High | `query` builds a read-only command toolset from `Command::read_only()`. Inventory coverage in `crates/server/tests/command_policy_enforcement_test.rs` allows only a small reviewed set of POST-style read helpers to override `read_only() == true`. | MITIGATED |
| TM-MCP-003 | Card HTML XSS via entity-controlled fields | High | Every interpolation in `crates/server/src/api/mcp_endpoint/cards.rs` flows through a single `escape_html` helper (covered by `escapes_all_html_specials` and `agent_card_renders_and_escapes` unit tests); the rendered document carries an inline `Content-Security-Policy` meta tag (`default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; connect-src 'none'`); host-side rendering uses an iframe with `sandbox="allow-scripts"` (no `allow-same-origin`/`allow-forms`/`allow-popups`/`allow-top-navigation`) and `referrerpolicy="no-referrer"`; `srcdoc` populates the iframe directly so the document is never fetched over HTTP. See `specs/mcp-cards.md`. | MITIGATED |
| TM-MCP-004 | Card-driven CSRF or unauthorized mutation | High | Cards in `cards.rs` are read-only and contain no out-of-band write path. The phased action protocol routes button clicks through host `postMessage` → host-issued `tools/call`, so Everruns's normal MCP auth, `Command::run` policy checks, and per-call `organization_id` resolution are re-applied to every action. Hosts MUST validate `MessageEvent.source` against the iframe `contentWindow` (enforced by `apps/ui/src/components/mcp/mcp-card-iframe.tsx`) and apply user confirmation for tools whose annotations are not `read_only_hint: true`. | MITIGATED |
| TM-MCP-005 | Card-induced denial of service via oversized HTML | Medium | `cards::render_html` enforces a 64 KiB rendered-document cap (`MAX_CARD_BYTES`), rejecting (rather than truncating) oversized cards (covered by `render_caps_size`). Card tool timeouts (`10s`) and `count_sessions_for_agent` single-COUNT queries bound server-side cost. Host-side iframe rate limiting in `mcp-card-iframe.tsx` (10 messages/sec token bucket) bounds inbound `postMessage` storms. | MITIGATED |

### Mitigation Details

**TM-MCP-001 — Everruns MCP Server Tenant Escape:**
The first-party MCP endpoint is both a discovery surface and an execution surface: `discover` publishes the command catalog, `query` runs bashkit scripts over read-only builtins, `execute` runs scripts over the full command set, and tier-1 tools (`agent_run`, `session_send_message`, `session_get_status`) compose session and message commands directly. The threat is a caller using catalog discovery plus scripted control flow, guessed IDs, or `organization_id` overrides to read or mutate another organization.

Mitigations are layered:
- The request `ResolvedOrg` is derived from authenticated org membership; per-tool `organization_id` overrides are resolved against fresh membership before dispatch.
- API-key callers cannot use per-tool org overrides to switch away from the org selected by request auth.
- `query` receives a read-only toolset built from inventory descriptors whose `read_only()` flag is true.
- `execute` and tier-1 tools dispatch through domain commands, preserving repository org filters and `Command::run` policy checks.
- MCP `resources/read` routes through policy-gated list commands instead of raw storage reads.
- Bashkit runs the scripted surface with parser, input, command-count, loop, function-depth, AST-depth, and timeout limits.

Regression coverage: `test_mcp_adversarial_tool_chain_cannot_escape_org_scope` creates real cross-org bait, discovers the relevant agent/session operations, then attacks the wrong org with `query`, `execute`, `agent_run`, `session_get_status`, `session_send_message`, and a non-member `organization_id` override. `test_mcp_resources_read_cannot_escape_org_scope` verifies resource reads do not leak cross-org agent summaries. Both tests assert no data leak and no mutation.

**TM-MCP-002 — Read-Only Query Catalog Drift:**
The MCP `query` tool is intentionally safer than `execute`, but that safety depends on the inventory metadata for every command. By default, only `GET` commands are read-only. POST-style helpers must explicitly override `read_only() == true`; each such override is reviewed in `mcp_query_read_only_overrides_are_allowlisted` to prevent a future mutating command from becoming available through `query`.

**TM-TOOL-015 — Browserless URL Validation (MITIGATED):**
Browserless tools now reuse the shared `validate_safe_url()` policy from core. This blocks:
- loopback and `localhost`
- RFC1918 private ranges
- link-local and cloud metadata endpoints
- IPv6 loopback/link-local/private ranges

Validation runs for:
- direct Browserless tool URLs (`navigate`, `content`, `screenshot`, `scrape`)
- `browserless_open_browser` initial navigation
- nested `navigate` actions inside `browserless_interact`

**TM-TOOL-018 — MCP Server SSRF (MITIGATED):**
MCP server URLs are validated twice:
1. On create/update in the control plane — static check via `validate_safe_url` rejects unsafe schemes, loopback, RFC1918, link-local, and cloud metadata targets.
2. At execution time (before each tool call and `tools/list` fetch) via `validate_url_dns_pinned` — performs the same static checks then resolves the hostname via `tokio::net::lookup_host` and verifies every returned IP against the blocked ranges. This closes the DNS-rebinding gap: an attacker cannot register a public hostname that initially resolves to a safe IP but later rebinds to an internal address, because the IP is re-checked on every outbound request.

## 8. LLM Integration (TM-LLM)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-LLM-001 | API key at rest exposure | Critical | Encrypted via envelope encryption (AES-256-GCM); stored in `llm_providers.api_key_encrypted` | MITIGATED |
| TM-LLM-002 | API key in logs | High | Never logged; tracing filters sensitive fields; generic error messages only | MITIGATED |
| TM-LLM-003 | API key in error messages | High | Provider errors sanitized before returning to user; full errors logged server-side | MITIGATED |
| TM-LLM-004 | API key lifetime in memory | Medium | Decrypted per-request; key dropped after LLM call completes | MITIGATED |
| TM-LLM-005 | Token quota exhaustion | Medium | Token usage tracked in usage events; retry with backoff honors `retry-after` headers | MITIGATED |
| TM-LLM-006 | Provider MITM | High | HTTPS required for all LLM provider communication | MITIGATED |
| TM-LLM-007 | Indirect prompt injection | High | Tool results and user messages are role-separated; no complete mitigation exists for LLM-level prompt injection | **ACCEPTED** |
| TM-LLM-008 | Cost runaway via agent loop | Medium | Max 10 iterations per agent turn; configurable | MITIGATED |
| TM-LLM-020 | Client-supplied privileged message roles in AG-UI input | Medium | Anonymous AG-UI/CopilotKit clients could send `role: "system"` / `developer` / `tool` messages that flow into the LLM context alongside the agent's real system prompt. Mitigated in `crates/server/src/api/ag_ui.rs::validate_input_messages` by rejecting any non-{user,assistant} role at the runtime trust boundary with a generic 400 `invalid_request`, and by rejecting duplicate message ids. | MITIGATED |
| TM-LLM-021 | Utility LLM key exposed through agent model configuration | High | The utility LLM uses deployment env secret `UTILITY_OPENAI_API_KEY`, is carried as a host service on `PlatformDefinition`, and is threaded only into capability `ToolContext`. It is not stored in provider records, exposed through model selection, or accepted from session/agent config. | MITIGATED |
| TM-LLM-022 | Tenant execution silently spending platform env keys | High | `LlmResolverService::resolve_provider_api_key` and `resolve_provider_credentials` are fail-closed: they return `None` when no database key is found rather than falling back to `DEFAULT_*_API_KEY` env vars. Callers surface a "no provider configured" error. Env var helpers remain available only for explicit dev/CLI entrypoints. For single-tenant/dev convenience, `seed::seed_default_provider_keys_from_env` may materialize `DEFAULT_*_API_KEY` into the **default org's** provider rows at startup (encrypted), gated by `SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV` (defaults to `DeploymentGrade::is_dev()`). Non-dev opt-in is ignored while built-in signup or built-in OAuth can self-provision users into `DEFAULT_ORG_ID`, so open-registration deployments cannot seed platform keys into an org that untrusted users can join and spend from. See `specs/llm-drivers.md` (Key Resolution Contract). | MITIGATED |
| TM-LLM-023 | Provider credentials exposed through the capability command contract | High | The `CommandHost` facilities (specs/commands.md, EVE-543) give capability `execute_command` implementations access to the session's turn context and a tool-less completion against the session's resolved model. `CommandTurnContext` is a deliberately credential-free view (model name and provider type only); driver creation and `ModelWithProvider` (decrypted key, base URL) stay inside the host-owned `StoreCommandHost`. Per-invocation model overrides resolve through the same org-scoped `LlmProviderStore` as a main turn. Completions are out-of-band: nothing is persisted to messages or events. | MITIGATED |
| TM-LLM-024 | Provider error detail leaking to untrusted session viewers | Medium | Session error disclosure is governed by the `error_disclosure` capability (`specs/error-disclosure.md`). `detailed` mode (provider error text in a `detail` field) is operator-opt-in per harness/agent; per-message `controls.error_disclosure` is clamped to the capability-configured ceiling so clients can narrow but never widen disclosure; `generic` mode collapses all blocking errors for public-facing agents. Public endpoints sanitize independently via `PublicError` (TM-LLM-003 unchanged: provider error bodies never include API keys). | MITIGATED |

### Mitigation Details

**TM-LLM-001 — Key Retrieval Flow:**
```
Worker needs LLM key
    → gRPC GetTurnContext (no key material in worker config)
    → Control plane fetches llm_providers row
    → EncryptionService.decrypt(api_key_encrypted)
    → Key returned in gRPC response (in-memory only)
    → Worker creates ChatDriver with key
    → LLM API call over HTTPS
    → Key dropped when driver goes out of scope
```

Workers never have direct database access or encryption keys. Key material exists in worker memory only during the LLM call.

**TM-LLM-007 — Prompt Injection (ACCEPTED):**
Indirect prompt injection via tool results or user messages is an inherent LLM limitation. Mitigations:
- Role separation (system, user, assistant, tool_result)
- Max iteration limit prevents infinite loops
- No automatic code execution without registered tool
- Monitoring via usage tracking

**TM-LLM-021 — Utility LLM Service:**
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

**TM-DURABLE-001 — Task Ownership:**
```
Worker A claims task → heartbeat timeout → task reclaimed by Worker B
Worker A finishes late → CompleteDurableTask → TaskNotOwned error
Worker B continues execution → task completes correctly
```
Prevents duplicate activity execution when workers lose connectivity.

**TM-DURABLE-002 — gRPC Security (MITIGATED):**
Workers authenticate to control plane gRPC (port 9001) via two layered mechanisms:

1. **Bearer token auth** (`WORKER_GRPC_AUTH_TOKEN` env var) — required in production (server panics on startup if unset in non-dev mode)
   - Server: `GrpcAuthInterceptor` validates `authorization: Bearer <token>` on every request
   - Client: `GrpcClientAuth` injects the bearer token into every outgoing request
2. **Mutual TLS (mTLS)** — optional, configured via `WORKER_GRPC_TLS_*` env vars
   - Server presents its certificate (`WORKER_GRPC_TLS_CERT`/`WORKER_GRPC_TLS_KEY`) and verifies client certs against `WORKER_GRPC_TLS_CA_CERT`
   - Worker presents its client certificate and verifies the server against the CA
   - Provides encryption + mutual identity verification at the transport layer
   - Bearer token auth remains active as defense-in-depth even when mTLS is enabled

**Design decision:** Workers are intentionally cross-org. They are stateless task executors that process work from any organization's queue. Org-scoping is enforced at the application layer (HTTP API), not the internal gRPC transport.

**TM-DURABLE-010 — Durable API Endpoints (MITIGATED):**
All `/v1/durable/*` HTTP endpoints require explicit platform-user auth. The auth backend returns `AuthUser.is_platform_user`, HTTP caller construction preserves it through `ResolvedOrg`, and `/v1/durable/config` exposes the same policy result for UI gating.

## 10. Scheduled Tasks (TM-SCHED)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-SCHED-001 | Malicious schedule creation / resource abuse | Medium | Only platform users can create or manage durable schedules; schedule channels enforce a minimum cron interval (`SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS`, default 300 s) and an org-level cap on enabled schedule channels (`SCHEDULE_CHANNEL_MAX_PER_ORG`, default 10) | MITIGATED |
| TM-SCHED-002 | Catch-up explosion on restart | High | `max_catch_up` limits catch-up runs (default: 1); prevents hundreds of executions on restart | MITIGATED |
| TM-SCHED-003 | Concurrent execution overload | Medium | `max_concurrent` field enforced; trigger skipped if limit reached | MITIGATED |
| TM-SCHED-004 | Invalid cron expression DoS | Low | Cron parser validates expression at creation time; invalid expressions rejected | MITIGATED |
| TM-SCHED-005 | Scheduler crash leaves tasks untriggered | Medium | Durable execution ensures tasks are created; if executor crashes, tasks auto-reclaimed via heartbeat | MITIGATED |

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

### Mitigation Details

**TM-OBS-001 — Braintrust Data Flow:**
```
Agent turn → events emitted → BraintrustEventListener (async)
    → Convert to OpenAI format
    → POST /v1/project_logs/{project_id}/insert
    → Fire-and-forget (no retry)
```
Full conversation data (user messages, LLM responses, tool results) is transmitted. Organizations must evaluate whether Braintrust integration is appropriate given their data classification requirements.

**TM-OBS-007 — Security Audit Logging (MITIGATED):**
- `audit_logs` PostgreSQL table (migration 005) stores structured events with: org_id, actor_id, event_type, ip_address, metadata, created_at.
- Event types follow `domain.action.outcome` convention: `auth.login.success`, `auth.login.failure`, `auth.register.success`, `auth.token_refresh.success`, `auth.personal_access_token.created`, `auth.personal_access_token.deleted`, `auth.oauth.success`, `auth.oauth.failure`.
- Fire-and-forget writes via `auth::audit::emit()` — audit failures never block auth operations.
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
| TM-WEB-008 | Open redirect via login page `return_to` | Medium | `sanitizeReturnTo` (`apps/ui/src/lib/auth-redirect.ts`) restricts `return_to` to relative paths on the frontend origin: must start with `/`, never `//` (protocol-relative), never `/\` (browser-normalized), never an absolute URL. Applied in login page and main layout (sessionStorage consumer). CLI auth start emits only relative `return_to` paths. See `specs/authentication.md` "Login Page Contract". | MITIGATED |
| TM-WEB-A2UI-01 | XSS via `javascript:`/`data:` URL in A2UI `open_url` action or `Image.src` | High | A2UI JSON is LLM-emitted. `isSafeUrl` in `apps/ui/src/components/chat/a2ui-renderer.tsx` restricts action URLs and image sources to `http:`/`https:`/`mailto:` schemes; `window.open` also uses `noopener,noreferrer`. React auto-escapes all text props. See `specs/a2ui.md`. | MITIGATED |
| TM-WEB-009 | XSS via SVG file preview (`<script>`, `on*` handlers, `javascript:` URLs, `<foreignObject>` HTML) | High | `SVGPreview` (`apps/ui/src/components/files/file-previews.tsx`) renders SVG inside an `<iframe sandbox="" srcDoc=...>` carrying a strict CSP meta tag (`default-src 'none'; style-src 'unsafe-inline'; img-src data:`). Empty `sandbox` denies all flags (scripts, forms, popups, top-nav, same-origin); CSP is defense-in-depth. SVG bytes are NOT sanitized server-side — the gate is the iframe boundary. `getPreviewType` routes `.svg` to this path for both `text` and `base64` encodings; no `<img src=data:image/svg+xml>` path remains. Regression tests in `apps/ui/src/__tests__/file-previews.test.tsx` exercise script, on-handler, javascript-URL, and foreignObject payloads. See EVE-389. | MITIGATED |

### Mitigation Details

**TM-WEB-004 / TM-WEB-005 — Security Headers (MITIGATED):**
Applied via `SetResponseHeaderLayer` (`if_not_present`) in `app_builder.rs`:
- `X-Frame-Options: DENY` — prevents clickjacking
- `X-Content-Type-Options: nosniff` — prevents MIME sniffing
- `Referrer-Policy: strict-origin-when-cross-origin` — limits referrer leakage
- `Permissions-Policy` disables unused browser device APIs. Default: `camera=(), microphone=(), geolocation=()`. When the `voice` feature flag is enabled, microphone is narrowed to same-origin use: `camera=(), microphone=(self), geolocation=()`.
- `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'`

## 13. AI Agent Behavior (TM-AGENT)

The agent loop is a core trust boundary: an LLM decides which tools to call with what arguments. The system prompt, user messages, tool results, and MCP tool descriptions all influence LLM behavior. Agents are semi-trusted within organizational scope — the agent creator (org member) is trusted, but the LLM's runtime decisions are not fully controllable.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-AGENT-001 | Direct prompt injection via user message | High | Role separation (user vs system); LLM providers apply safety training; no complete defense | **ACCEPTED** |
| TM-AGENT-002 | Indirect prompt injection via tool results | High | Tool results use `tool_result` role, not `system`; LLM may still follow adversarial instructions in results | **ACCEPTED** |
| TM-AGENT-003 | Indirect prompt injection via MCP tool descriptions | Medium | MCP tool names/descriptions fed to LLM as tool schema; adversarial descriptions could influence behavior | **ACCEPTED** |
| TM-AGENT-004 | Agent jailbreak via system prompt | Medium | System prompt set by org member at agent creation; no sanitization of prompt content | **BY DESIGN** |
| TM-AGENT-005 | Capability escalation via agent creation | High | RiskLevel enum on Capability trait; high-risk capabilities (`a2a_agent_delegation`, `docker_container`, `daytona`, `e2b`, `deno`, `bashkit_shell`, `web_fetch`) require Admin role to assign via API; gate is at create/update only, member-owned agents that already had high-risk capabilities are grandfathered (see `specs/capabilities.md` "Admin-Only Tier Decision") | MITIGATED |
| TM-AGENT-006 | Cost runaway — unbounded LLM calls | High | Max iterations per turn (default 100); configurable per agent | MITIGATED |
| TM-AGENT-007 | Cost runaway — many tools per iteration | Medium | No per-iteration tool call limit; agent can invoke many tools in a single LLM response | **OPEN** |
| TM-AGENT-008 | Context window poisoning | Medium | Auto-compaction via `llm_driver.compact()` on `RequestTooLarge`; older messages compressed | MITIGATED |
| TM-AGENT-009 | Agent self-modification | Medium | Agents with `platform_management` capability can modify agents/sessions via tools; capability must be explicitly assigned; org-scoped | **OPEN** |
| TM-AGENT-010 | Agent spawning agent chains | Medium | Agents with `platform_management` capability can create agents/sessions; capability must be explicitly assigned; no recursive depth limit | **OPEN** |
| TM-AGENT-011 | Sensitive data in system prompt | Medium | PII must not be placed in system prompts; no encryption at rest for prompts | **OPEN** |
| TM-AGENT-012 | Tool result size amplification | Medium | 64 KiB hard limit on tool results via `OutputHardLimitHook` (EVE-225); always-on final hook in ActAtom | MITIGATED |
| TM-AGENT-013 | Exfiltration via web_fetch | Medium | Agent with web_fetch capability can send session data to arbitrary URLs | **ACCEPTED** |
| TM-AGENT-014 | Confused deputy — tool call with wrong session | Low | Tool context includes session_id; tools scoped to active session only | MITIGATED |
| TM-AGENT-015 | Dangling tool calls cause LLM confusion | Low | Patched with synthetic "cancelled" results before LLM call; prevents API errors | MITIGATED |
| TM-AGENT-016 | Plaintext secrets in chat history | Medium | When agent asks user for API key in chat, plaintext value stored in events table as message content; session secrets encrypt separately but chat retains plaintext | **OPEN** |
| TM-AGENT-017 | Agent-initiated entity management | High | Agents with `platform_management` can create/update/delete harnesses, agents, sessions org-wide; no fine-grained RBAC within org; capability must be explicitly assigned | **OPEN** |
| TM-AGENT-018 | Outbound URL filtering on web_fetch | Medium | Per-layer `NetworkAccessList` (harness ∩ agent ∩ session, narrow-only merge) plus optional deployment-wide system allowlist, both enforced at the `EgressService` boundary; web_fetch routes through egress with per-redirect-hop re-validation | MITIGATED |
| TM-AGENT-019 | Internal network probing via high-risk execution capabilities | High | `daytona` and `e2b` provide full network access by design; `docker_container` uses host networking in dev mode; all rely on Admin-only assignment plus infrastructure egress isolation | **ACCEPTED** |
| TM-AGENT-020 | Cross-session resource reuse via stale or guessed external IDs | Critical | Provider-owned resource IDs are checked against the active session's leased-resource/session-resource ownership before tool execution; raw sandbox list endpoints are filtered to owned IDs only | MITIGATED |
| TM-AGENT-021 | System prompt regurgitation | Medium | Opt-in `prompt_canary_guardrail` capability runs a streaming output guardrail that replaces the assistant message when the first sentence of the system prompt appears verbatim in the model output; original tokens are dropped and never persisted. Catches verbatim leaks only — paraphrased or partial leaks pass through. See `specs/capabilities.md` § Output Guardrails | MITIGATED (partial, opt-in) |
| TM-AGENT-022 | Agent-initiated machine-payment spend | High | Paid capabilities cannot directly sign or submit arbitrary paid HTTP requests. They call `PaymentAuthority`, which selects an active policy matching the session/agent/agent identity/user/org, capability, target host, rail, and per-request limit before signing; attempts are audited | MITIGATED |
| TM-AGENT-023 | Redirect-based payment host-allowlist bypass | High | The payment authority validates only the original request URL host against the active policy's `allowed_hosts`. The outbound `reqwest` client used by `ServerPaymentAuthority::send_http_request` disables redirect following (`redirect::Policy::none()`), so 30x responses cannot be used to redirect a paid request to an unvalidated/internal host or downgrade from HTTPS | MITIGATED |
| TM-AGENT-024 | A2A outbound delegation egress / SSRF bypass | High | Outbound A2A delegation respects the merged `ToolContext.network_access` ACL. `enforce_network_access` (in `crates/core/src/capabilities/a2a_delegation.rs`) validates the configured `base_url` and every resolved `AgentCard` interface URL against the runtime ACL before the A2A client is built; `submit_run`, `wait_for_run`, and `cancel_task` all flow through this gate, so configured or AgentCard-discovered endpoints cannot bypass egress controls | MITIGATED |

### Mitigation Details

**TM-AGENT-001 / TM-AGENT-002 — Prompt Injection (ACCEPTED):**
Prompt injection is an inherent limitation of current LLM architecture. Defense-in-depth:
1. **Role separation:** System, user, assistant, tool_result messages are distinct roles
2. **Iteration limits:** Max turns prevents infinite manipulation loops
3. **Tool registry:** LLM can only call registered tools (no arbitrary code execution)
4. **Session isolation:** Even if manipulated, agent is confined to its session
5. **No auto-escalation:** Agent cannot grant itself new capabilities
6. **Instruction hierarchy:** Generic harness system prompt includes an explicit instruction hierarchy statement directing the LLM to prioritize system instructions over content in tool results, user messages, or agent instructions files

There is no reliable way to prevent an LLM from following adversarial instructions embedded in tool results or user messages. This is an industry-wide limitation.

**TM-AGENT-004 — System Prompt Trust Model:**
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

**TM-AGENT-005 — Capability Escalation (MITIGATED):**
Each capability declares a `RiskLevel` (Low, Medium, High) via the `Capability` trait. High-risk capabilities (`docker_container`, `daytona`, `e2b`) require `OrgRole::Admin` to assign. The check runs in create/update/upsert/import agent API handlers, returning 403 if a non-admin user attempts to assign a high-risk capability. The `risk_level` field is exposed in the capabilities list API for UI display.

**TM-AGENT-006 — Iteration Limit:**
```rust
// Turn state machine enforces max iterations
if self.current_iteration >= self.max_iterations {
    TurnAction::Complete(TurnOutcome::MaxIterationsReached { ... })
}
```
Default: 100 iterations. Each Reason→Act cycle counts as one iteration. Configurable per agent.

**TM-AGENT-008 — Context Window Poisoning (MITIGATED):**
When the message history exceeds the LLM's context window, the `ReasonAtom` catches `RequestTooLarge` errors and calls `llm_driver.compact()` to compress older messages. This prevents unbounded context growth. Adversarial early messages are still present but may be summarized during compaction.

**TM-AGENT-013 — Exfiltration via web_fetch (ACCEPTED):**
An agent with `web_fetch` capability can:
1. Read session files via `read_file` tool
2. Send file contents to external URL via `web_fetch` tool

This is accepted because:
- Agent capabilities are chosen by org members (trusted)
- `web_fetch` is an opt-in capability, not default
- The intended use case requires external HTTP access
- Removing this would break legitimate functionality

**TM-AGENT-016 — Plaintext Secrets in Chat History (OPEN):**
When an agent tool (e.g., Daytona) doesn't find an API key, it may instruct the user to provide one in chat. The user types the key as a chat message, which is stored as plaintext in the `events` table (message content). The `session_secrets` table encrypts the value separately, but the original chat message retains plaintext indefinitely.

- **Impact:** API keys visible in session history, event exports, and any observability pipeline that captures events.
- **Recommendation:** Prefer Settings UI for credential entry (user connections). Phase out in-chat secret collection. For tools that need credentials, guide users to Settings > Connections instead of requesting secrets in chat.
- **Priority:** High

**TM-AGENT-017 — Agent-Initiated Entity Management (OPEN):**
Agents with the `platform_management` capability can create, update, and delete harnesses, agents, and sessions within their organization. They can also send messages to any session and read responses.

- **Impact:** An agent could escalate privileges by creating a new agent with dangerous capabilities, modify other agents' system prompts, or spawn session chains. No fine-grained RBAC exists within the org scope.
- **Current mitigations:** (1) Capability must be explicitly assigned by an org member. (2) All operations are org-scoped — cross-org access blocked by tenant isolation (TM-TENANT-001). (3) Platform tool execution resolves the owning session's user into a real `Caller` and evaluates command policy with the active `PermissionResolver`, so member-owned Platform Chat sessions do not inherit internal/owner bypass. (4) Both in-process (`DirectPlatformStore`) and gRPC (`ExecuteCommand`) platform paths route mutating operations through the normal command/policy boundary instead of raw storage writes. (5) `WorkerAdapters::platform_store(org_id, session_id)` receives the session's actual org_id and session_id from activity context, preventing cross-org access via hardcoded defaults and preserving session-owner authorization.
- **Recommendation:** Add audit logging for all platform management tool calls. Consider RBAC (e.g., "can only manage own sessions") and approval workflows for dangerous operations (creating agents with `bashkit_shell`). Add recursion depth limits for agent-spawned session chains.
- **Code:** `// THREAT[TM-AGENT-017]` at `PlatformManagementCapability` registration and `DirectPlatformStore` implementation.
- **Priority:** High

**TM-AGENT-018 — Outbound URL Filtering on web_fetch (MITIGATED):**
An agent influenced by prompt injection (via tool results or user messages) could chain data access tools with `web_fetch` to exfiltrate sensitive session data. While TM-AGENT-013 accepts this risk for legitimate use by trusted org members, prompt injection (TM-AGENT-001, TM-AGENT-002) can cause the agent to act against the user's intent.

- **Attack chain:** Injected instruction in tool result → agent reads sensitive file → agent calls `web_fetch` with file contents to attacker-controlled URL
- **Current mitigations:** (1) Per-layer `NetworkAccessList` (allowed/blocked patterns) on harness, agent, and session, merged narrow-only (intersection on `allowed`, union on `blocked`) — see `specs/network-access.md`; configurable via API and the agent/harness edit UI. (2) Optional deployment-wide system allowlist of curated public hosts, AND-ed as a hard ceiling — see `specs/system-allowlist.md`. (3) Both are enforced at the `EgressService` boundary; `web_fetch` routes through egress (`crates/core/src/capabilities/web_fetch/egress_transport.rs`) with the list re-checked on every redirect hop.
- **Residual risk:** Defaults are open — with no `NetworkAccessList` configured and the system allowlist disabled, outbound destinations are unrestricted (TM-AGENT-013 ACCEPTED). Outbound calls are not yet audit-logged with URL + payload size.
- **Complements:** SSRF protection blocks private IPs with DNS pinning on the egress path (`validate_url_dns_pinned`, TM-API-008/TM-TOOL-018).
- **Priority:** Medium

**TM-AGENT-019 — Internal Network Probing via High-Risk Execution Capabilities (ACCEPTED):**
Some execution capabilities intentionally originate network traffic outside the worker process:
- `daytona` sandboxes have full Linux and network access by design
- `e2b` sandboxes have full Linux and network access by design
- `docker_container` uses host networking and is experimental/dev-only

This means an agent with one of these capabilities can probe whatever network the sandbox/container can reach. Current mitigations are:
- Admin-only assignment for high-risk capabilities (TM-AGENT-005)
- `docker_container` is gated to development-grade deployments

Residual risk remains with the deployment topology. Production operators must enforce egress filtering and network segmentation for any execution environment that can reach internal services.

**TM-AGENT-020 — Cross-Session Resource Reuse (MITIGATED):**
Tools that accept provider-owned external IDs (`sandbox_id`, raw Daytona toolbox paths, and
similar handles) resolve ownership through the active session's leased resources before calling
the backend. The session resource registry carries the same external-ID metadata for runtimes
that only expose the generic registry. Raw sandbox list calls are filtered to the IDs owned by
the active session before results are returned to the agent.

**TM-AGENT-022 — Agent-Initiated Machine-Payment Spend (MITIGATED):**
Agents can invoke paid capabilities such as Parallel search/extract/task, but V1 deliberately has
no generic `paid_http_request` tool. The capability submits a typed payment request to
`PaymentAuthority`; the server selects only active spend policies scoped to the current session,
agent, agent identity, user, or organization and checks capability allowlist, host allowlist, rail
preference, and per-request maximum before creating any rail-specific signature. Every attempt is
persisted with status, amount, target URL, and receipt/error. Registration of any money-spending
capability is additionally gated by the `machine_payments` feature flag
(`FEATURE_MACHINE_PAYMENTS`), off by default on every grade, so spend tools are never offered
unless deliberately enabled.

## 14. Voice Sessions (TM-VOICE)

Voice Sessions add browser microphone capture and provider realtime sessions.
See [voice.md](voice.md) for the feature contract.

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
| TM-BASH-001 | Workspace boundary escape | Critical | `SessionFileSystemAdapter` rejects paths outside `/workspace`; returns `PermissionDenied` | MITIGATED |
| TM-BASH-002 | Read host /etc/passwd or system files | Critical | No real filesystem; all I/O goes through `SessionFileSystemAdapter` → session file store | MITIGATED |
| TM-BASH-003 | Network access from bash | Critical | Bashkit has no network builtins; curl/wget not available (no real process execution) | MITIGATED |
| TM-BASH-004 | Fork bomb / process spawning | Critical | No real process execution; `exec`, subprocesses, background processes not implemented (exit 127) | MITIGATED |
| TM-BASH-005 | Infinite loop CPU exhaustion | High | `max_loop_iterations: 10000`; `max_commands: 1000`; parser timeout 5s | MITIGATED |
| TM-BASH-006 | Deep recursion stack overflow | High | `max_function_depth: 100`; `max_ast_depth: 100` | MITIGATED |
| TM-BASH-007 | Large script input DoS | High | `max_input_bytes: 1_000_000` (1 MB) | MITIGATED |
| TM-BASH-008 | Execution timeout | High | Default 30s, max 60s; enforced by tool executor | MITIGATED |
| TM-BASH-009 | Environment variable leak | Medium | Controlled env: only HOME, SHELL, PATH, WORKSPACE; hardcoded username/hostname ("everruns") | MITIGATED |
| TM-BASH-010 | Symlink escape | Medium | `SessionFileSystemAdapter.symlink()` returns `Error (unsupported)` | MITIGATED |
| TM-BASH-011 | Path traversal via bash | High | Paths normalized by bashkit; `to_session_path()` rejects paths outside `/workspace` | MITIGATED |
| TM-BASH-012 | Privilege escalation (sudo, su) | Low | No privilege commands implemented; sandboxed interpreter only | MITIGATED |
| TM-BASH-013 | eval/bash re-invocation escape | Medium | `eval` and `bash`/`sh` commands re-invoke the sandboxed interpreter, not real shell | MITIGATED |
| TM-BASH-014 | File permission bypass | Low | `chmod` is a no-op; session filesystem has no permission model | **BY DESIGN** |
| TM-BASH-015 | Host information disclosure | Low | `hostname` → "everruns"; `whoami` → "everruns"; `uname` returns sandboxed values | MITIGATED |
| TM-BASH-016 | Write amplification via bash | Medium | Per-session and per-file byte quotas enforced in `DirectWorkerAdapters::write_file` (see TM-FS-008) | MITIGATED |

### Mitigation Details

**TM-BASH-001 / TM-BASH-011 — Workspace Boundary:**
```rust
fn to_session_path(path: &Path) -> Option<String> {
    // /workspace       → /
    // /workspace/foo   → /foo
    // /tmp/foo         → None  → PermissionDenied
    // /etc/passwd      → None  → PermissionDenied
}
```
All bashkit filesystem operations go through `SessionFileSystemAdapter`, which maps paths rooted at `/workspace` to session file store paths. Anything outside `/workspace` returns `PermissionDenied`.

**TM-BASH-005 — Resource Limits:**
```rust
ExecutionLimits::new()
    .max_commands(1000)
    .max_loop_iterations(10000)
    .max_function_depth(100)
    .max_input_bytes(1_000_000)
    .max_ast_depth(100)
    .parser_timeout(Duration::from_secs(5))
```

**TM-BASH-009 — Controlled Environment:**
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

**TM-BASH-013 — Sandboxed Re-invocation:**
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

Experimental sandboxed Lua execution capability (`crates/core/src/capabilities/lua.rs`,
`specs/lua-execution.md`). Engine: **mlua** (vendored Lua 5.4, never LuaJIT),
behind the `lua` cargo feature. High risk, admin-gated (same gates as
`bashkit_shell`), and runtime-gated by `FEATURE_LUA`. One fresh VM per invocation,
never shared across sessions/tenants. All hardening is on by default — no
configuration knobs.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-LUA-001 | Arbitrary code execution | High | Admin-gated assignment (High risk tier); only string/table/math/os/utf8 libs loaded; dangerous globals scrubbed | MITIGATED |
| TM-LUA-002 | CPU / wall-clock exhaustion | High | Instruction-count hook (every 100k ops) enforces an instruction budget + wall-clock deadline; outer tokio timeout backstop. The VM runs on a dedicated blocking thread, so a pathological *synchronous* op (e.g. catastrophic Lua pattern in C, which the hook cannot interrupt) occupies one blocking-pool thread instead of stalling a shared runtime worker. **Residual:** such an op is not force-killable in-process — robust fix is out-of-process execution. | MITIGATED (best-effort for synchronous C ops) |
| TM-LUA-003 | Memory exhaustion | High | `Lua::set_memory_limit` hard 32 MiB cap (over-budget alloc → Lua error). Host-side reads bounded by `SessionFileSystem` quotas (TM-FS-008). | MITIGATED |
| TM-LUA-004 | Filesystem escape / cross-tenant access | High | All paths route through `LuaVfs` → session-scoped `SessionFileSystem`; `/workspace`-rooted, traversal/outside-workspace rejected; `io` library not loaded | MITIGATED |
| TM-LUA-005 | Network egress / SSRF / exfiltration | High | No socket library. `http.get/post` is **fail-closed**: routed only through the host `EgressService` (the central egress boundary) AND requires a non-empty `network_access` allow-list that permits the URL — checked before the request. Absent either, `http.*` is not even defined. Response bodies capped at 1 MiB. | MITIGATED (allow-listed egress) |
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
| TM-DOS-014 | Tool output context growth | Medium | Read-like tools use windowed responses and truncation envelopes (`read_file`, `list_directory`, `grep_files`, browser DOM content); platform message reads cap message count and per-message content; non-image binary reads return metadata instead of base64 or lossy UTF-8; opted-in exec tools persist full output under `/outputs/` so the inline prompt payload can stay bounded and recoverable. | MITIGATED |
| TM-DOS-015 | Unbounded tool fan-out within an act batch | Medium | A single model turn can request an arbitrary number of tool calls; `ActAtom` previously executed them all concurrently with no bound. The `tool_scheduler` (`crates/core/src/atoms/tool_scheduler.rs`) now caps simultaneously-executing calls with a semaphore (default 32, `EVERRUNS_ACT_MAX_TOOL_CONCURRENCY`), serializes same-`concurrency_class` mutations, and offloads `cpu_bound` tools to their own task so an in-process interpreter burst cannot starve the runtime worker. Does not bound calls across time/agents (see TM-TOOL-009). | MITIGATED |
| TM-DOS-016 | Mass resource creation via IP rotation | High | Per-org/per-user rate limits on expensive mutations (session create: 60/min per org; schedule create: 20/min per user; org create: 10/hr per user) via `OrgRateLimiter` (`crates/server/src/auth/rate_limit.rs`). Distributed when `VALKEY_URL` is set, in-memory otherwise. Fail-open on Valkey errors; DB-level resource caps (`max_orgs_per_user`, etc.) bound total consumption. Global per-IP `ApiRateLimiter` also uses Valkey when set. | MITIGATED |
| TM-DOS-017 | ReDoS / oversized config via guardrail checks | Medium | Guardrail `regex` rules (config-persisted and via `POST /v1/capabilities/guardrails/dry-run`) compile with `RegexBuilder::size_limit` (1 MB), so the linear-time `regex` engine cannot be wedged by a pathological pattern; check count, entries-per-check, entry length, and replacement length are capped at compile time, and dry-run input text is bounded to 64 KiB (`crates/core/src/guardrail_checks.rs`, `domains/capabilities/commands.rs`). Compilation runs synchronously in the streaming/tool path but is bounded; invalid persisted config is logged and treated as no checks rather than failing the turn. | MITIGATED |

### Mitigation Details

**TM-DOS-009 — Valkey Network Exposure (CALLER RISK):**
Valkey (Redis-compatible) is used for distributed rate limiting. In local/dev compose, it runs without authentication on port 6379.
- **Production:** Deploy Valkey on a private network, not exposed to the internet. Use `rediss://` (TLS) URLs and AUTH passwords for cloud-managed instances (e.g., AWS ElastiCache, GCP Memorystore).
- **Blast radius if compromised:** Attacker can flush rate limit counters (bypassing rate limits) or inject fake counters (DoS via false rate-limit-exceeded). No sensitive data stored in Valkey.

**TM-DOS-010 — Fail-Open Rate Limiting (ACCEPTED):**
By design, Valkey errors cause rate limiting to fail open (allow requests) for `ApiRateLimiter` and `OrgRateLimiter`. This prioritizes availability over strictness for general API traffic. Auth endpoints (`AuthRateLimiter`) remain fail-closed. See `crates/server/src/auth/rate_limit.rs`.

**TM-DOS-016 — Per-identity rate limiting (MITIGATED):**
`OrgRateLimiter` (`crates/server/src/auth/rate_limit.rs`) adds per-identity velocity caps on expensive operations. Configurable via `RATE_LIMIT_ORG_SESSION_CREATE_PER_MINUTE` (default 60), `RATE_LIMIT_ORG_SCHEDULE_CREATE_PER_MINUTE` (default 20), and `RATE_LIMIT_USER_ORG_CREATE_PER_HOUR` (default 10). Uses Valkey when `VALKEY_URL` is set. Residual risk: without Valkey, limits are per-instance.

## 17. Daytona Cloud Sandbox (TM-DAYTONA)

Daytona sandboxes are remote Linux environments managed via REST API. The agent can create, exec commands, and manage files in these sandboxes. The `daytona_git_credentials` tool writes a GitHub token to disk inside the sandbox to enable git push/pull/fetch operations.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-DAYTONA-001 | Git token persisted on sandbox disk | Medium | Token written to `/tmp/.git-credentials`; lost on sandbox stop/delete; same trust boundary as `daytona_exec` (anyone who can exec can already read the file) | **ACCEPTED** |
| TM-DAYTONA-002 | Git token expiry — stale credentials | Low | GitHub App installation tokens expire in ~1 hour; tool hint tells agent to call `daytona_git_credentials` again to refresh | MITIGATED |
| TM-DAYTONA-003 | Git token scope — over-privileged access | Medium | Token scoped by GitHub App installation permissions; user controls repo access via GitHub App settings | **CALLER RISK** |
| TM-DAYTONA-004 | Daytona API key compromise | High | Stored in user connections (Settings > Connections); encrypted at rest via envelope encryption (AES-256-GCM) | MITIGATED |
| TM-DAYTONA-005 | Cross-session sandbox access | Critical | Daytona tools require session-owned sandbox IDs via leased-resource/session-resource ownership checks; persisted sandbox state stays session-scoped in `daytona_sandbox:{id}` | MITIGATED |
| TM-DAYTONA-006 | Sandbox not deleted — resource leak | Low | Auto-stop 5 min, auto-archive 30 min, auto-delete 60 min (Daytona-native); leased-resource cleanup 20 min (control plane); system prompt instructs agent to delete when done | MITIGATED |
| TM-DAYTONA-007 | Git credential helper persists after sandbox reuse | Low | Credential file in `/tmp` cleared on stop; sandbox stop resets environment | MITIGATED |
| TM-DAYTONA-008 | GitHub token leaked to lookalike clone host | High | `daytona_git_clone` and `daytona_git_credentials` only embed the GitHub token in HTTPS URLs whose host matches an operator-configured trusted-host allowlist (`trusted_github_hosts` / `is_trusted_github_https_host` in `integrations/daytona/src/tools.rs`). Default `["github.com"]`; operators extend via `EVERRUNS_DAYTONA_GITHUB_TRUSTED_HOSTS` (comma-separated, exact case-insensitive match, no wildcards). Malformed env entries (`/`, `@`, whitespace, `..`) are rejected with a warning; the default is always preserved so misconfig cannot silently disable public-GitHub auth. Unit tests cover lookalike rejection (`evil-github.acme.com`, `github.acme.com.evil.example`). | MITIGATED |

### Mitigation Details

**TM-DAYTONA-001 — Git Token on Disk (ACCEPTED):**
The `daytona_git_credentials` tool writes `https://oauth2:<token>@github.com\n` to `/tmp/.git-credentials` and configures `git config --global credential.helper 'store --file=/tmp/.git-credentials'`. This is the same pattern used by GitHub Actions and other CI systems.

Accepted because:
- The sandbox is an isolated environment — same trust boundary as exec access
- Any agent that can call `daytona_exec` can already run arbitrary commands
- Token is in `/tmp`, lost on sandbox stop/delete
- Token is short-lived (~1 hour GitHub App installation token)
- Alternative (API-proxied credential helper) deferred as future improvement

**TM-DAYTONA-003 — Token Scope (CALLER RISK):**
The GitHub token's scope depends on the GitHub App installation permissions. Users must review which repositories the GitHub App has access to in their GitHub settings. Everruns does not enforce per-repo restrictions at the application level.

**TM-DAYTONA-005 — Cross-Session Isolation:**
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
| TM-E2B-004 | Sandbox not deleted or paused — resource leak | Low | E2B timeout + auto-pause on create/resume, plus Everruns leased-resource cleanup | MITIGATED |
| TM-E2B-005 | Full-network sandbox misuse | High | Capability is high-risk/Admin-gated via capability assignment policy; residual network exposure depends on deployment egress isolation | **CALLER RISK** |

### Mitigation Details

**TM-E2B-003 — Cross-Session Isolation:**
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
| TM-SANDBOX-001 | Container escape via kernel vulnerability | High | Configurable runtime: default `runc`, production `sysbox-runc` (user namespaces + procfs virtualization); operator chooses isolation level | **ACCEPTED** |
| TM-SANDBOX-002 | Resource exhaustion (memory/CPU/PIDs) | High | cgroup limits enforced via Docker create flags (`memory_limit`, `cpu_limit`, `pids_limit`); defaults: 2 GiB, 1 CPU, 256 PIDs | MITIGATED |
| TM-SANDBOX-003 | Network attacks from sandbox | High | Per-sandbox isolated Docker bridge network; egress filtering blocks private IPs and cloud metadata (169.254.0.0/16) | MITIGATED |
| TM-SANDBOX-004 | Cross-session container access | Critical | Container names include `session_id`; all Docker API queries filtered by `session` + `managed-by` labels; sandbox state stored in session-scoped secrets | MITIGATED |
| TM-SANDBOX-005 | Image supply chain attack | Medium | Image allowlist in capability config; only pre-approved images can be pulled | MITIGATED |
| TM-SANDBOX-006 | Docker socket exposure inside sandbox | High | Docker socket never mounted into containers; no `--privileged` flag | MITIGATED |
| TM-SANDBOX-007 | Stale container not cleaned up | Medium | Leased resource scheduler with 20-minute lease duration; system prompt instructs agent to remove when done | MITIGATED |
| TM-SANDBOX-008 | Cross-tenant sandbox access | Critical | Tool scoping via `ToolContext.session_id` + per-sandbox network + Docker label filters; container names derived from session UUID, never user input | MITIGATED |
| TM-SANDBOX-009 | Cross-tenant network reachability | High | Each sandbox gets its own isolated Docker bridge network (`sandbox-{org}-{session}`); sole member is the sandbox container | MITIGATED |
| TM-SANDBOX-010 | Tenant resource starvation | High | Per-sandbox cgroups + per-org concurrent sandbox limits via leased resources | MITIGATED |

### Mitigation Details

**TM-SANDBOX-001 — Container Escape (ACCEPTED):**
Container isolation depends on the kernel and runtime. Default `runc` provides namespace + cgroup isolation but shares the host kernel. For production multi-tenant deployments, operators should configure `sysbox-runc` (adds user namespaces, procfs/sysfs virtualization) or `kata`/`gvisor` for stronger isolation. The runtime is a deployment-time config field (`CONTAINER_SANDBOX_RUNTIME`), not baked into code.

**TM-SANDBOX-004 — Cross-Session Isolation:**
```
Session A creates: sandbox-{org}-{session_a} → container + network
Session B creates: sandbox-{org}-{session_b} → container + network

Session A cannot access Session B's container:
  - Docker API queries include label filter: session={session_a}
  - Container name includes session_a UUID
  - Sandbox state stored in session-scoped secrets
```

**TM-SANDBOX-008 — Cross-Tenant Isolation (6 layers):**
1. Tool scoping: container name derived from `ToolContext.session_id`, never user input
2. Per-sandbox Docker network: `sandbox-{org}-{session}`, sole member = the sandbox
3. Label-filtered API calls: all queries include `session` + `managed-by` labels
4. Per-org limits: max concurrent sandboxes checked at create time via leased resources
5. Egress filtering: block private IPs + cloud metadata from sandbox bridges
6. Runtime isolation: configurable (sysbox adds user-ns + procfs virtualization)

## 22. A2A Channel (TM-A2A)

App-scoped Agent2Agent (A2A) protocol ingress. JSON-RPC 2.0 endpoint authenticated by a per-channel API key. Mitigations live in `crates/server/src/api/app_a2a.rs` and `crates/server/src/domains/apps/commands.rs`. See `specs/a2a-channel.md`.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-A2A-001 | API key brute force | Medium | Keys are 32 random bytes (256-bit entropy) prefixed `evra2a_`; stored only as SHA-256 hex; plaintext returned exactly once at create / regenerate; the published Agent Card never includes the key or hash | MITIGATED |
| TM-A2A-002 | Timing oracle on key compare | Medium | Constant-time byte comparison of the SHA-256 hash digests in `app_a2a::constant_time_eq` before any session creation | MITIGATED |
| TM-A2A-003 | Plaintext key persistence / log leak | High | Plaintext is never persisted: only hash + non-secret prefix go into `channel_config`. `Authorization` headers are not surfaced into template context (A2A invocations only template `payload`, `a2a.*`, and app metadata — request headers are not exposed) | MITIGATED |
| TM-A2A-004 | Anonymous ingress to draft / disabled channels | High | Published-app + enabled-channel checks run before key validation; the Agent Card endpoint mirrors the same gate and 404s otherwise | MITIGATED |
| TM-A2A-005 | A2A method abuse beyond the supported set | Medium | Method allowlist of four (`message/send`, `message/stream`, `tasks/get`, `tasks/cancel`); all other methods return JSON-RPC `-32601 Method not found` without touching the session pipeline. Each handler reuses the same auth + channel + method gate before any session work | MITIGATED |
| TM-A2A-006 | Empty / non-text part injection | Low | The endpoint requires at least one non-empty `text` part; otherwise returns `-32602 Invalid params`. This prevents triggering an empty / whitespace-only user message into the session | MITIGATED |
| TM-A2A-007 | Cross-org session reuse via tag spoofing | High | Inherits the webhook channel mitigation: shared sessions are matched by both org + owner principal + tag set, so a user cannot pre-seed an `app:`/`app_channel:` tagged session and have an A2A invocation reuse it | MITIGATED |
| TM-A2A-008 | API key rotation does not invalidate old keys | Medium | `regenerate_a2a_app_channel_key` overwrites both `api_key_hash` and `api_key_prefix` in the same row, so the previous key fails constant-time comparison on the next request | MITIGATED |
| TM-A2A-009 | Agent Card discloses sensitive metadata | Low | Card shape is fixed (`name`, `description`, `supportedInterfaces`, capabilities, security schemes, public skill); never echoes the API key, hash, prefix, internal channel UUID, or owner principal | MITIGATED |
| TM-A2A-010 | Replay of captured request | Medium | Opt-in Slack-derived HMAC signing on the A2A channel via `A2aChannelConfig::signing_secret` (`crates/core/src/app.rs`). When enabled, requests must carry `X-Everruns-A2A-Timestamp` + `X-Everruns-A2A-Signature` headers; the server verifies HMAC-SHA256 over `v0:{timestamp}:{channel_scope}:{raw_body}` (where `channel_scope` is the literal `{app_id}:{channel_id}`) using constant-time compare in `crates/server/src/api/a2a_signing.rs`. Including the channel scope inside the signed basestring also prevents cross-channel replay when operators share the same `signing_secret` across multiple A2A channels — without it, a captured request for channel A could be forwarded to channel B because the per-channel-keyed replay store would not catch the cross-channel reuse. A symmetric 5-minute timestamp window plus signature-keyed dedup (scope `app_id:channel_id`) mean a captured request can only be replayed once and only inside that window. Two backends mirror the rate limiter — in-memory HashMap with on-insert TTL pruning for single-instance/dev, Valkey `SET ... NX EX` for distributed. Check runs after primary authentication (API key or endpoint-auth) so unauthenticated callers cannot probe channel existence or grow the in-memory store. Plaintext secret is encrypted at rest via the existing `channel_config` envelope encryption, redacted on read with `signing_secret_configured: bool`, and preserved across PATCH. Channels without `signing_secret` keep the existing auth-only behavior, so existing deployments keep working. Same fail-open behavior on Valkey outage as TM-DOS-010 / TM-A2A-013. Defense-in-depth still applies: HTTPS (TM-AUTH-005) and rotation remain available | MITIGATED |
| TM-A2A-011 | `message/stream` SSE leaks events from unrelated sessions or holds resources after auth fails | Medium | The streaming branch reuses the same auth + channel + method gate as `message/send` and only subscribes to `EventDelivery` after the per-call session is resolved. The translator filters by `event.session_id == session_id` before emitting any frame, only translates an allowlist of session events (`output.message.completed`, `turn.completed`, `turn.failed`, `turn.cancelled`), and closes the stream after the first terminal status frame. A dropped subscription emits a synthetic `failed` final frame so the client does not hang | MITIGATED |
| TM-A2A-012 | `tasks/get` / `tasks/cancel` cross-channel reads or destructive actions | Medium | Both handlers reuse the same auth + channel + method gate as `message/send`, look up the underlying session org-scoped via `get_session(auth.org_id, ...)`, and additionally verify the session belongs to the authenticated app + channel via routing tags (`app:<public_id>` and `app_channel:<public_id>`) before returning or modifying anything. An API key from one A2A channel cannot read or cancel tasks for sessions created by another channel even if both share the same org. Sessions that fail the binding check return `-32001 Task not found` rather than leaking existence. State derivation only consults turn lifecycle events; raw prompts, tool args, and LLM outputs are never echoed back. Cancelling an already-terminal task is idempotent — `cancel_a2a_session_turn` re-derives state after `cancel_run` and skips the synthetic `turn.cancelled` emission if the workflow has reached a terminal state, so derived state cannot race-flip a `completed` task to `canceled` | MITIGATED |
| TM-A2A-013 | DoS via runaway A2A client | Medium | App owners can configure a per-app, per-IP request cap via `A2aChannelConfig::rate_limit_per_minute` (`crates/core/src/app.rs`), enforced in `crates/server/src/api/app_a2a.rs::authenticate_request` after API key verification so an unauthenticated caller cannot grow the limiter cache or learn channel existence from rate-limit signals. Backed by the shared `ChannelRateLimiter` primitive (`crates/server/src/api/channel_rate_limit.rs`) — in-memory governor for single-instance/dev, Valkey sliding-window when `VALKEY_URL` is set; namespaces (`agui` / `a2a`) keep keys disjoint. A2A scope is `app_id:channel_id` (not just `app_id`) so multiple A2A channels on the same app keep independent buckets — sharing an `app_id`-only key would let an attacker alternate between channels with different limits to flush the cached limiter (replace-on-limit-change) and bypass the stricter cap. Server caps the field at `1_000_000` so a typo cannot silently disable the limit. `0` / `None` disables the per-channel cap and falls back to the global API limit. Same fail-open behavior on Valkey outage as TM-DOS-010 | MITIGATED |
| TM-A2A-014 | Agent Card advertises stale or wrong auth scheme | Medium | Agent Card security metadata is derived from the same effective `A2aChannelConfig.auth` used by `authenticate_request`. Legacy channels continue to advertise bearer API key; OIDC/Google emit `openIdConnect`, HTTP Basic emits `http/basic`, OAuth2 introspection emits generic HTTP bearer rather than fabricating OAuth token-flow metadata, and mTLS emits `mutualTLS`. The card remains published only for live apps with enabled A2A channels and never includes secrets. | MITIGATED |

### Mitigation Details

**TM-A2A-001 — API Key Generation:**
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
The plaintext is returned in the `add_a2a_app_channel` and `regenerate_a2a_app_channel_key` command responses and never read back from storage — `channel_config` only holds the SHA-256 hash and the display prefix. Agent Card responses derive from the same row but explicitly select non-secret fields.

**TM-A2A-005 — Method Gate:**
The handler dispatches strictly on `method == "message/send"`. Any other JSON-RPC method short-circuits with a structured `-32601` response *before* any session work. This keeps the surface narrow until streaming and task lifecycle features are implemented.

**TM-A2A-007 — Tag-spoof Hardening:**
A2A reuses `find_app_session_by_tags_and_owner` (already mitigates the webhook variant TM-AUTHZ-006). Sessions matched for `shared_session` mode require the requesting app's `org_id` *and* `owner_principal_id` to line up, so a user-created session sharing the same surface tags is rejected.

## 23. FCP Channel (TM-FCP)

App-scoped Free Communication Protocol ingress. Text-first HTTP endpoint with a deliberately minimal auth stack (anonymous + optional shared bearer token) and a dedicated `ChannelRateLimiter` namespace. Mitigations live in `crates/server/src/api/fcp.rs` and `crates/server/src/domains/apps/commands.rs`. See `specs/fcp-channel.md`.

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
| TM-FCP-011 | Internal vocabulary or provider details leaked through error bodies | Medium | All `turn.failed` causes pass through `PublicError::from_internal_code` (`crates/server/src/api/public.rs`). The mapping returns one of four sanitized public messages (`InternalError`, `RateLimited`, `ServiceUnavailable`, `RequestTooLarge`) — provider names, stack traces, and internal codes never reach the wire. Tested in `crates/server/src/api/fcp.rs::tests::turn_error_response_body_is_sanitized` | MITIGATED |
| TM-FCP-012 | Handshake (GET) used to probe operator existence | Low | The handshake is intentionally open per FCP SPEC, but `resolve_context` returns the same generic 404 body for unknown / draft / wrong-channel apps. The per-app rate limiter (when configured) also applies to GETs so a single client cannot hammer the handshake either | MITIGATED |

### Mitigation Details

**TM-FCP-003 — Token redaction across read paths:**
```rust
// crates/server/src/domains/apps/commands.rs
ChannelType::Fcp => {
    if map.remove("token").is_some() {
        map.insert("token_configured".to_string(), Value::Bool(true));
    }
}
```
And on update, the preserved-secrets merger reinjects the existing encrypted token if the caller did not provide a new one, so PATCHing other fields does not clear the configured token by accident.

**TM-FCP-008 — Session ownership invariant:**
`SessionService::create_from_app` sets `session.owner_principal_id = app.owner_principal_id` and tags every session with `fcp:app:<app_public_id>`. The reuse path (`find_app_session_by_tags`) requires the `org_id`, `app.internal_id`, AND the routing tag to all match — so even an attacker who guesses a victim app's id and forges a cookie cannot adopt a session owned by a different org or app.

## 24. User Hooks (TM-HOOK)

User-authored shell commands run at lifecycle and tool events via the
`user_hooks` capability and any capability that returns `UserHookSpec`
entries from `user_hooks()`. See `specs/user-hooks.md`.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-HOOK-001 | Hook-as-injection-amplifier: model-controlled file at the hook command path lets prompt-injected agent influence hook behavior | High | Hooks execute through `bashkit_shell` against the session VFS, identical FS isolation as the `bash` tool. Operators who reference scripts from agent-writable paths (`/workspace`) opt in to that risk; the recommended pattern is to inline the command or read scripts from read-only capability mounts | MITIGATED |
| TM-HOOK-002 | Hook-as-exfil-channel: a hook command makes outbound network calls to leak session state | High | This build of bashkit is compiled **without** the `http_client` feature (see `bashkit_shell.rs::642` and TM-BASH-003), so `curl`/`after_http`/etc. builtins do not exist in the hook's vocabulary at all. The interpreter has no built-in path to open a socket. If a future build flips `http_client` on, this entry must be re-evaluated and an outbound allowlist enforced at the hook layer. | MITIGATED |
| TM-HOOK-003 | Stdout poisoning: a long-running or malicious hook fills stdout with bogus JSON or floods to deny tool execution | Medium | Per-hook timeout (default 5 s, max 30 s) + 64 KiB combined stdout/stderr cap (reuses `OutputHardLimitHook` ceiling) + `on_error` policy (`block`/`allow`/`warn`). Overrun is treated as an executor error, not a decision | MITIGATED |
| TM-HOOK-004 | Privilege escalation via capability contribution: a built-in capability ships hooks that exfiltrate or block | High | The built-in `user_hooks` capability is permanently `High` and admin-gated on assignment via `check_high_risk_caps`. Capability-contributed hooks are surfaced in audit logs with their `{capability_id}:{name}` `HookId` so operators can locate and mute them via the `disabled_contributions` list on a sibling `user_hooks` config. Declarative-capability-contributed hook bundles (with the matching auto-elevation rule) are **not yet implemented** — see `specs/user-hooks.md` for the deferred path | MITIGATED |
| TM-HOOK-005 | Hook chain DoS via fan-out across many configured hooks | Medium | Per-hook timeout caps wall-clock; hook execution is serial within a single event firing; combined chain wall-clock for `pre_tool_use` is bounded by `Σ timeout_ms` which is itself bounded by `(MAX_HOOK_TIMEOUT_MS × N hooks)`. Operators set the contributing capability list, capping `N` in practice | MITIGATED |
| TM-HOOK-006 | Future risk: declarative-capability hook bundles bypass the admin gate | High | Declarative `user_hooks` are deferred (no field on `DeclarativeCapabilityDefinition` today). The path is reserved: when added, the declarative-capability write API must compute effective risk including `user_hooks` and force `RiskLevel::High` when the array is non-empty. Threat tracked here so the contract lands with the feature | **OPEN** (deferred) |

### Mitigation Details

**TM-HOOK-001 — Path trust model:** Hook commands are interpreted as
bash command lines. When the command references a script from the
session VFS (e.g. `bash /workspace/scripts/fmt.sh`), the operator is
trusting the contents of that path. Recommended patterns:

1. Inline the command directly in `executor.command`.
2. Mount scripts read-only via a capability `mounts()` declaration so
   the agent cannot rewrite them mid-session.
3. Reference scripts from a known-safe path that the agent has no
   tools to write to (e.g. `/.agents/hooks/...` under a capability
   read-only mount).

**TM-HOOK-002 — Egress inheritance:** `BashHookExecutor` does not
construct a separate `NetworkAccessList`; the session sandbox supplies
the same policy `bashkit_shell` honors. There is no way to "opt out" a
hook command from session egress controls without explicit operator
action on the agent.

**TM-HOOK-004 — Capability-contributed hooks:** Today the only
in-tree contributor surface is the built-in `user_hooks` capability,
which carries `RiskLevel::High` and is gated on admin assignment via
`check_high_risk_caps`. Capability authors that override
`Capability::user_hooks` / `user_hooks_with_config` to ship hook bundles
also ride the trust gate of having their capability assigned to an agent.
The runtime collection path (`finalize_hook_specs`) stamps every
non-`user_hooks` contribution into the `{capability_id}:` `HookId`
namespace — capability authors cannot forge the `user:` namespace — and
drops any contribution whose id appears in a sibling `user_hooks`
capability's `disabled_contributions` list, so an operator can always
mute a bundled hook without removing the contributing capability.

**TM-HOOK-006 — Deferred contract:** When declarative
`user_hooks` lands, the capability-write API must compute effective
risk *including* `user_hooks` and force `RiskLevel::High` when the
array is non-empty. The elevation must be unconditional and not
downgradeable by the author. The Linear ticket linked from the
`specs/user-hooks.md` follow-up list tracks this work.

## 25. CI / Build Pipeline (TM-CI)

GitHub Actions workflows in `.github/workflows/` are part of the trust boundary because they execute on push to `main` and on fork pull requests once the first-time-contributor approval is granted. After that one-time approval, every subsequent PR from the same fork runs CI automatically; the only barrier between attacker-controlled code and the repo's secrets is the per-workflow secret scoping.

Key GitHub guarantee: for `pull_request` triggers, the workflow YAML is read from the BASE branch, so a fork PR cannot directly add an exfiltration step to a workflow. The risk is indirect — workflow YAML in the base branch may execute PR-controlled code (cargo build/test runs `build.rs`, proc-macros, and test bodies; PR-built server/worker/CLI binaries execute) with secrets injected into the process env. Any secret available to that code is exfiltratable.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-CI-001 | Workflow-level `DOPPLER_TOKEN` exfil via PR-controlled cargo build/test | Critical | `DOPPLER_TOKEN` is no longer declared at workflow `env:` in `ci.yml`, `brave-search-integration.yml`, `cursor-integration.yml`, or `sprites-integration.yml`. It is injected only into jobs/steps gated on `github.event_name == 'push'` (live-test jobs in ci.yml + per-integration live-test jobs). The Slack step in `integration-test` was changed from `if: env.DOPPLER_TOKEN != ''` to `if: github.event_name == 'push'` with step-scoped env | MITIGATED |
| TM-CI-002 | LLM provider keys (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`) exfil from `integration-test` job env via PR-controlled cargo test | High | Three keys removed from `integration-test` job env. The single step that needs them (`Run LLM integration tests`) gates on `github.event_name == 'push'` and sets them via step env | MITIGATED |
| TM-CI-003 | LLM provider keys exfil from PR-built `everruns-server`/`everruns-worker` binaries in `workflow-test` | High | `workflow-test` job condition now requires `github.event_name == 'push'`. Compile-time PR coverage of the binaries is preserved via `build-binaries` | MITIGATED |
| TM-CI-004 | LLM provider keys exfil via `DEFAULT_*_API_KEY` from PR-built CLI in `cli-e2e-test` | High | `cli-e2e-test` job condition now requires `github.event_name == 'push'`. PR-side CLI coverage continues via `unit-test`'s CLI integration tests, which run without keys | MITIGATED |
| TM-CI-005 | Brave Search live tests exposing Doppler vault on fork PRs | High | `brave-search-integration.yml` split into a PR-safe `unit-test` job (no secrets) and a push-only `live-test` job. Weekly `integration-live-sweep.yml` backstops shared-crate regression coverage | MITIGATED |
| TM-CI-006 | First-time-contributor approval grants persistent CI access | Medium | GitHub setting "Require approval for all outside collaborators" recommended at the org/repo level. Not enforced in this repo's workflows; orthogonal to the per-secret scoping above | **OPERATIONAL** |
| TM-CI-007 | `GITHUB_TOKEN` exfil on PR | Low | GitHub scopes the fork-PR `GITHUB_TOKEN` to read-only by default. `docker-publish.yml` uses it only on `push`/tag jobs; PR-validation jobs do not log in to GHCR | MITIGATED |

### Mitigation Details

**TM-CI-001 / TM-CI-002 / TM-CI-003 / TM-CI-004 — `pull_request` vs `push` gating:**
The four workflows touched in this category previously declared secrets at workflow `env:` or at job `env:` on jobs that ran on `pull_request`. After the fix, every step that has any secret in its env satisfies one of:
- The enclosing job condition includes `github.event_name == 'push'` (or `workflow_dispatch`), or
- The step itself has an `if: github.event_name == 'push'` guard, and the secret is set at step `env:` only.

This ensures GitHub never instantiates the secret value into a runner that is executing fork-PR-controlled code (build.rs, proc-macros, test bodies, or PR-built binaries).

**TM-CI-006 — Outside-collaborator approval gate:**
Even with per-secret scoping, an attacker who controls a previously-approved fork can submit malicious PRs that the workflow base-branch YAML still runs. The remaining defense layer is the GitHub org setting "Require approval for all outside collaborators" under Actions → General → Fork pull request workflows. This is a repo/org-level operational control, not a workflow change, and is therefore tracked here for visibility.

## 26. Plugins (TM-PLUGIN)

Installed plugins (`specs/plugins.md`) compile third-party remote content into agent context: marketplace catalogs and plugin directories are fetched from external sources and become system prompt text, skills, and scoped MCP config. This is a supply-chain and prompt-injection surface layered on the declarative-capability model; everything compiled passes the same declarative validation (size/count limits, text-only files, traversal rejection, scoped-MCP URL checks).

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-PLUGIN-001 | Prompt injection via marketplace plugin content (agents/skills/commands compiled into system prompt and skill mounts) | High | Same class as TM-AGENT-001/002 — no complete defense. Adding marketplaces and installing plugins are admin-gated org actions; content is pinned at install time and reviewable; install warnings surface dropped components | **ACCEPTED** |
| TM-PLUGIN-002 | Supply-chain mutation of an installed plugin (upstream force-push or tag move) | High | GitHub installs pin a commit SHA at install time; the running capability is the persisted compiled definition, never re-fetched implicitly; updates are explicit, re-fetched at the marketplace's current synced SHA, and recompiled through full validation | MITIGATED |
| TM-PLUGIN-003 | SSRF via `url` marketplace source or plugin-declared MCP servers | High | `url` sources are HTTPS-only and SSRF-validated before fetch; plugin `.mcp.json` passes the same SSRF-safe scoped-MCP validation as agent/harness `mcpServers` (TM-MCP) | MITIGATED |
| TM-PLUGIN-004 | Resource exhaustion via oversized catalog or plugin archive (zip bomb) | Medium | Size cap on fetched `marketplace.json`; tarball extraction enforces per-file (64 KB), total (4 MB), and file-count (256) caps and rejects symlink/hardlink entries | MITIGATED |
| TM-PLUGIN-005 | Code-execution smuggling via plugin components | High | v1 compiles data-only contributions; `hooks`, `lspServers`, `monitors` and other executable components are dropped with install warnings and never executed server-side; MCP tools execute remotely under existing TM-MCP controls | MITIGATED |
| TM-PLUGIN-006 | Server filesystem read via `local_path` marketplace source | High | `local_path` is rejected unless the deployment is dev-grade (`DeploymentGrade::from_env().is_dev()`); production deployments only accept `github`/`url` sources | MITIGATED |
| TM-PLUGIN-007 | Typosquatting / spoofed plugin names in an org's marketplaces | Medium | Marketplace registration is admin-gated; plugin and marketplace names are unique per org; no global plugin namespace exists in v1, so impersonation requires an admin to register the hostile marketplace | **ACCEPTED** |

### Mitigation Details

**TM-PLUGIN-002 — Pinned compile-at-install model:**
The capability that agents execute is the compiled `definition` JSONB persisted in `plugin_installs` at install/update time. Upstream changes to the source repository have no effect on running agents until an admin explicitly updates, at which point the content is re-fetched at the marketplace's current synced SHA and re-validated end to end. This is the same trust shape as a lockfile: sync moves the candidate version; update moves the installed one.

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
| TM-AUTH-015 | JWT secret insecure default | High | Fail startup if AUTH_JWT_SECRET unset in production |
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
| TM-AUTH-011 | Auth bypass in none mode | By design for local development |
| ~~TM-API-008~~ | ~~WebFetch SSRF~~ | Reclassified to **MITIGATED** — fetchkit v0.1.2 DnsPolicy blocks private IPs |
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
| A2A replay of captured request | TM-A2A-010 | Opt-in scope-bound HMAC signing (`A2aChannelConfig::signing_secret`) — basestring `v0:{ts}:{app_id}:{channel_id}:{body}` so signatures are non-reusable across channels; 5-minute timestamp window plus signature-keyed dedup; in-memory or Valkey backend |
| CI secret scoping | TM-CI-001..005 | No `secrets.*` at workflow `env:`; secrets are injected only into jobs/steps gated on `github.event_name == 'push'` (or `workflow_dispatch`) so fork-PR-controlled cargo/binary code cannot read them from process env |

## References

- `specs/authentication.md` — Authentication modes, JWT, personal access tokens, OAuth
- `specs/encryption.md` — Envelope encryption design
- `specs/multitenancy.md` — Org-based isolation model
- `specs/workspace.md` — Session file storage and path validation
- `specs/session-sqldb.md` — SQLite sandbox and VFS design
- `specs/tool-execution.md` — Tool types and execution flow
- `specs/mcp-servers.md` — MCP server integration
- `specs/llm-drivers.md` — LLM provider abstraction
- `specs/durable-execution-engine.md` — Workflow engine and worker communication
- `specs/scheduled-tasks.md` — Cron-based task scheduling
- `specs/observability.md` — OpenTelemetry and Braintrust observability providers
- `specs/apis.md` — HTTP API endpoints and error handling
- `specs/capabilities.md` — Agent capabilities system
- `specs/bashkit-requirements.md` — Bashkit integration requirements
- `integrations/daytona/SPEC.md` — Daytona cloud sandbox integration
- `integrations/deno/SPEC.md` — Deno sandbox integration
- `integrations/e2b/SPEC.md` — E2B cloud sandbox integration
- `specs/client-side-tools.md` — Client-side tools for API/SDK consumers
- `specs/apps.md` — Apps system (agent deployment to channels)
- `crates/server/specs/slack-integration.md` — Slack bot integration
- `integrations/brave-search/SPEC.md` — Brave Search web search integration
- `specs/infinity-context.md` — Unlimited conversation length via context management
- [fetchkit v0.1.2 source](https://crates.io/crates/fetchkit) — SSRF protection (resolve-then-check, DNS pinning, DnsPolicy), URL prefix blocking, fetch options, fetcher registry
