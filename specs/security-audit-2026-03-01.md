# Security Audit Report — 2026-03-01

## Scope

Full security analysis of everruns covering:
- **Backend**: Rust API server, authentication, authorization, cryptography, gRPC
- **UI**: React/Next.js frontend, XSS, CSRF, token handling, dependencies
- **Database**: SQL injection, tenant isolation, encryption at rest, SQLite sandbox

## Executive Summary

The everruns codebase demonstrates strong security fundamentals: parameterized SQL queries, envelope encryption for secrets, Argon2id password hashing, and solid sandboxing for session SQLite and bash execution. However, the audit identified **12 security findings** — 3 critical/high priority issues require immediate attention.

**Key concern**: The threat model (`specs/threat-model.md`) had 2 entries marked MITIGATED that are actually unimplemented (TM-AUTH-007, TM-DURABLE-002). These discrepancies have been corrected.

## Findings Summary

| # | Severity | Finding | Threat Model | Linear |
|---|----------|---------|-------------|--------|
| 1 | **Critical** | OAuth CSRF state parameter not validated | TM-AUTH-007 (was MITIGATED → now OPEN) | EVE-28 |
| 2 | **High** | Durable API endpoints lack authentication | TM-DURABLE-010 (NEW) | EVE-29 |
| 3 | **High** | User listing endpoint lacks org isolation | TM-TENANT-008 (NEW) | EVE-30 |
| 4 | **High** | No rate limiting on auth endpoints | TM-AUTH-001 (OPEN) | EVE-31 |
| 5 | **High** | JWT secret falls back to insecure default | TM-AUTH-015 (NEW) | EVE-35 |
| 6 | **High** | gRPC auth optional and lacks org scoping | TM-DURABLE-002 (was MITIGATED → now OPEN) | EVE-37 |
| 7 | **Medium** | Account enumeration via registration | TM-AUTH-014 (NEW) | EVE-32 |
| 8 | **Medium** | Missing security headers (CSP, X-Frame-Options) | TM-WEB-004/005 (OPEN) | EVE-33 |
| 9 | **Medium** | No audit logging for auth events | TM-OBS-007 (NEW) | EVE-34 |
| 10 | **Medium** | ReDoS risk in session file grep | TM-DOS-008 (NEW) | EVE-36 |
| 11 | **Medium** | Limited encryption scope | TM-CRYPTO-007 (OPEN) | EVE-38 |
| 12 | **Medium** | Database connection TLS not enforced | NEW | EVE-39 |

## Detailed Findings

### Finding 1: OAuth CSRF State Parameter Not Validated [CRITICAL]

**Location**: `crates/server/src/auth/routes.rs:508-525`
**Threat Model**: TM-AUTH-007 (incorrectly was MITIGATED)

The `generate_oauth_state()` function creates a random state string, and it's included in the OAuth redirect URL. However, the state is never stored (in cookie or session) and never validated in `oauth_callback()`. The code has explicit TODO comments acknowledging this gap:
- Line 508: "In a production system, we'd store the state in a session/cookie for verification"
- Line 524: "TODO: Validate state from session for CSRF protection"

**Impact**: Attackers can perform OAuth CSRF to link their OAuth account to a victim's session.

### Finding 2: Durable API Endpoints Lack Authentication [HIGH]

**Location**: `crates/server/src/api/durable.rs:148-210`
**Threat Model**: TM-DURABLE-010 (NEW)

All `/v1/durable/*` endpoints (workflows, workers, circuit breakers, DLQ, SSE) use only `State(state): State<AppState>` — no `AuthUser` or `ResolvedOrg` extractor. They are accessible without authentication. Other API endpoints (agents, sessions, files) correctly use `ResolvedOrg`.

**Impact**: Unauthenticated users can list/cancel workflows, drain workers (DoS), view cross-org workflow data.

### Finding 3: User Listing Endpoint Lacks Org Isolation [HIGH]

**Location**: `crates/server/src/api/users.rs:104-135`
**Threat Model**: TM-TENANT-008 (NEW)

`GET /v1/users` uses `_auth: AuthUser` (auth only) without `ResolvedOrg`. The underlying query at `repositories.rs:237` has no `org_id` WHERE clause, returning ALL users across the entire system.

**Impact**: Any authenticated user can enumerate all users (emails, names) across all organizations.

### Finding 4: No Rate Limiting on Auth Endpoints [HIGH]

**Location**: `crates/server/src/auth/routes.rs` (login, register, refresh)
**Threat Model**: TM-AUTH-001 (already OPEN)

No rate limiting on `/v1/auth/login`, `/v1/auth/register`, `/v1/auth/refresh`. LLM-provider rate limiting exists but not for authentication.

### Finding 5: JWT Secret Insecure Default [HIGH]

**Location**: `crates/server/src/auth/config.rs:176-177`
**Threat Model**: TM-AUTH-015 (NEW)

When `AUTH_JWT_SECRET` is not set, the server falls back to `"insecure-dev-secret-change-me"`. No startup validation prevents this in production.

### Finding 6: gRPC Auth Optional and Lacks Org Scoping [HIGH]

**Location**: `crates/server/src/grpc_service.rs:172-196`
**Threat Model**: TM-DURABLE-002 (was MITIGATED, now OPEN)

`GRPC_AUTH_TOKEN` is read via `env::var().ok()` — when unset, auth is disabled. Additionally, gRPC handlers have no organization-level scoping.

### Finding 7: Account Enumeration [MEDIUM]

**Location**: `crates/server/src/auth/routes.rs:280-288`
**Threat Model**: TM-AUTH-014 (NEW)

Registration returns distinct "Email already registered" error, enabling email harvesting.

### Finding 8: Missing Security Headers [MEDIUM]

**Location**: `crates/server/src/app_builder.rs`
**Threat Model**: TM-WEB-004/005 (already OPEN)

No X-Frame-Options, CSP, X-Content-Type-Options, or Referrer-Policy headers.

### Finding 9: No Security Audit Logging [MEDIUM]

**Location**: `crates/server/src/auth/` (all auth files)
**Threat Model**: TM-OBS-007 (NEW)

No structured audit logs for login attempts, API key operations, permission changes, or OAuth linking.

### Finding 10: ReDoS Risk in File Grep [MEDIUM]

**Location**: `crates/server/src/api/session_files.rs:88-96`
**Threat Model**: TM-DOS-008 (NEW)

User-supplied regex patterns accepted without complexity validation. Note: if using Rust's `regex` crate, catastrophic backtracking is prevented by design. Verify implementation.

### Finding 11: Limited Encryption Scope [MEDIUM]

**Threat Model**: TM-CRYPTO-007 (already OPEN)

System prompts, chat history, and event data stored as plaintext. Only API keys and connection tokens encrypted.

### Finding 12: Database TLS Not Enforced [MEDIUM]

**Location**: `.env.example`, `crates/server/src/storage/repositories.rs`
**Threat Model**: NEW

`DATABASE_URL` in `.env.example` uses plain connection without `sslmode=require`. No code-level validation.

## Positive Findings

The audit confirmed strong security in several areas:

| Area | Assessment |
|------|-----------|
| SQL injection prevention | **Secure** — All queries use sqlx parameterized statements. No raw SQL formatting. |
| Encryption at rest | **Secure** — AES-256-GCM envelope encryption with key rotation for API keys and secrets. |
| Password hashing | **Secure** — Argon2id with random salt per password. |
| Cookie security | **Secure** — HttpOnly, Secure, SameSite=Lax/Strict. Access token is HttpOnly. |
| SSRF protection | **Secure** — fetchkit v0.1.2 DnsPolicy blocks private IPs with DNS pinning. |
| SQLite sandboxing | **Secure** — Authorizer blocks ATTACH/DETACH, load_extension, write PRAGMAs. VFS isolation. Resource limits. |
| Bash sandboxing | **Secure** — Bashkit WASM-like isolation, no filesystem/network access, resource limits. |
| XSS prevention (UI) | **Secure** — No dangerouslySetInnerHTML. Safe markdown rendering via Streamdown. |
| Token management (UI) | **Secure** — No localStorage for tokens. HttpOnly cookies. Proper refresh logic. |
| API key storage | **Secure** — SHA-256 hashed, shown once at creation, prefix only in listings. |
| Org isolation (core) | **Secure** — All core entity queries include `WHERE org_id = $org_id`. |
| Error sanitization | **Secure** — Generic 500 errors returned. Details logged server-side only. |
| Dependency versions | **Current** — All major Rust and JS dependencies at recent stable versions. |

## Threat Model Updates

The following changes were made to `specs/threat-model.md`:

### Status Corrections
- **TM-AUTH-007**: MITIGATED → **OPEN** (state validation not implemented)
- **TM-DURABLE-002**: MITIGATED → **OPEN** (auth optional, no org scoping)

### New Entries Added
- **TM-AUTH-014**: Account enumeration via registration (OPEN)
- **TM-AUTH-015**: JWT secret insecure default (OPEN)
- **TM-TENANT-008**: User listing cross-org (OPEN)
- **TM-DURABLE-010**: Durable API endpoints unauthenticated (OPEN)
- **TM-DOS-008**: ReDoS via file grep endpoint (OPEN)
- **TM-OBS-007**: No security audit logging (OPEN)

## Recommended Priority

**Immediate (P1)**:
1. EVE-28: Fix OAuth state validation (TM-AUTH-007)
2. EVE-29: Add auth to durable API endpoints (TM-DURABLE-010)
3. EVE-30: Add org isolation to user listing (TM-TENANT-008)

**High (P2)**:
4. EVE-35: Fail startup on missing JWT secret (TM-AUTH-015)
5. EVE-37: Require gRPC auth token in production (TM-DURABLE-002)
6. EVE-31: Implement auth rate limiting (TM-AUTH-001)

**Medium (P3)**:
7. EVE-33: Add security headers (TM-WEB-004/005)
8. EVE-32: Fix account enumeration (TM-AUTH-014)
9. EVE-34: Add audit logging (TM-OBS-007)
10. EVE-36: Add regex validation (TM-DOS-008)
11. EVE-38: Expand encryption scope (TM-CRYPTO-007)
12. EVE-39: Enforce database TLS (NEW)
