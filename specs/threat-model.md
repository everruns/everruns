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
| TM-BASH | Bash Sandbox | Bashkit sandbox escape, resource exhaustion, VFS boundary |
| TM-DOS | Denial of Service | Resource exhaustion, large payloads |
| TM-CLIENT | Client-Side Tools | Tool ID spoofing, timeout abuse |
| TM-SLACK | Slack Integration | Webhook forgery, signing secret leak, bot loops |

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
| TM-AUTH-004 | Weak password | Medium | Minimum 8 characters enforced; Argon2id hashing | MITIGATED |
| TM-AUTH-005 | API key exposure in transit | High | HTTPS required in production; keys prefixed `evr_` for scanning | MITIGATED |
| TM-AUTH-006 | API key brute force | Medium | Keys stored as SHA-256 hashes; 128-bit entropy makes brute force infeasible | MITIGATED |
| TM-AUTH-007 | OAuth state fixation | High | State parameter generated but NOT validated in callback (`routes.rs:508-525` has TODO) | **OPEN** |
| TM-AUTH-008 | Session fixation via cookie | Medium | New tokens issued on login; HTTP-only, SameSite=Lax cookies | MITIGATED |
| TM-AUTH-009 | Refresh token theft | High | Stored hashed in DB; HTTP-only cookie; revocable | MITIGATED |
| TM-AUTH-010 | Admin password in env var | Low | Limited to admin mode; documented risk; shell history exposure possible | **ACCEPTED** |
| TM-AUTH-011 | Auth bypass in `none` mode | Info | By design for local development; anonymous user gets admin role | **BY DESIGN** |
| TM-AUTH-012 | OAuth account linking collision | Medium | Accounts linked by email; if attacker controls email at provider, they gain access | **CALLER RISK** |
| TM-AUTH-013 | Expired API key still in use | Medium | Expiration checked on every request via DB lookup; `last_used_at` tracked | MITIGATED |
| TM-AUTH-014 | Account enumeration via registration | Medium | Returns generic "Registration failed" for existing emails; password hash computed first for timing consistency | MITIGATED |
| TM-AUTH-015 | JWT secret insecure default | High | Falls back to hardcoded `insecure-dev-secret-change-me` if `AUTH_JWT_SECRET` unset; no startup check in production | **OPEN** |

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

**TM-AUTH-006 — API Key Storage:**
```
User sees: evr_<full-random-key>    (shown once at creation)
DB stores: SHA-256(evr_<full-key>)  (irreversible)
Display:   evr_<first-8-chars>...   (prefix for identification)
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

## 3. Tenant Isolation (TM-TENANT)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-TENANT-001 | Cross-org resource access | Critical | All DB queries include `WHERE org_id = $org_id`; enforced at repository layer | MITIGATED |
| TM-TENANT-002 | Org enumeration via error codes | Medium | 404 returned for cross-org access (not 403); prevents existence discovery | MITIGATED |
| TM-TENANT-003 | Org cookie manipulation | High | Cookie value is `public_id`; server validates user membership against DB | MITIGATED |
| TM-TENANT-004 | API key scope escalation | High | API keys 1:1 with org; key lookup returns org directly; org validated in auth middleware | MITIGATED |
| TM-TENANT-005 | Internal org_id exposure | Medium | `org_id` (BIGINT) never in APIs, URLs, logs, or error messages; only `public_id` exposed | MITIGATED |
| TM-TENANT-006 | Session inherits wrong org | Medium | Sessions scoped via agent FK; agent scoped to org; query joins enforce chain | MITIGATED |
| TM-TENANT-007 | Durable tasks cross-org | Medium | gRPC `GetTurnContext` validates org_id in request matches record in DB | MITIGATED |
| TM-TENANT-008 | User listing cross-org | High | `GET /v1/users` returns all system users without org filtering; uses `AuthUser` not `ResolvedOrg` | **OPEN** |

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
| TM-FS-008 | Large file storage abuse | Medium | No per-session storage quota enforced at application level | **OPEN** |

### Mitigation Details

**TM-FS-001 — Defense in Depth:**
Path validated at three layers:
1. **Application:** Path parsing rejects traversal patterns
2. **Database constraint:** `session_files_path_check` CHECK constraint
3. **Unique constraint:** `(session_id, path)` prevents collision

**TM-FS-008 — Storage Quota (OPEN):**
No per-session or per-org storage limit on session files. An agent could create many large files.
- **Recommendation:** Enforce per-session file count and total size limits.
- **Priority:** Medium

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
| TM-TOOL-009 | No per-agent tool rate limiting | Medium | All tools execute without rate limits | **OPEN** |
| TM-TOOL-010 | Skill SKILL.md prompt injection | Medium | Skill instructions returned as `tool_result` role (not system prompt); `<skill>` XML wrapper provides clear boundary | MITIGATED |
| TM-TOOL-011 | Skill archive path traversal | High | ZIP extraction validates all paths; rejects `../`, absolute paths, symlinks; max 100 files, 1 MB each, 10 MB total | MITIGATED |
| TM-TOOL-012 | Skill archive zip bomb | High | Decompressed size capped at 10 MB; file count capped at 100; individual file size capped at 1 MB | MITIGATED |
| TM-TOOL-013 | Skill name collision across orgs | Medium | Skill names are unique per organization; capability IDs include UUID for global uniqueness | MITIGATED |
| TM-TOOL-014 | Disabled skill still activatable | Medium | `CapabilityService.list_all()` filters out disabled skills; disabled skills not included in `<available_skills>` | MITIGATED |

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

**TM-TOOL-011/012 — Skill Archive Validation:**
ZIP archive extraction in `SkillService::create_from_archive()` enforces:
1. No path traversal: paths checked for `../`, absolute paths, and symlinks
2. File count limit: max 100 files per archive
3. Per-file size limit: 1 MB per individual file
4. Total decompressed size limit: 10 MB
5. Files extracted into `skill_files` table as individual rows (no runtime ZIP extraction)

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

### Mitigation Details

**TM-LLM-001 — Key Retrieval Flow:**
```
Worker needs LLM key
    → gRPC GetTurnContext (no key material in worker config)
    → Control plane fetches llm_providers row
    → EncryptionService.decrypt(api_key_encrypted)
    → Key returned in gRPC response (in-memory only)
    → Worker creates LlmDriver with key
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
| TM-DURABLE-010 | Durable API endpoints unauthenticated | High | All `/v1/durable/*` endpoints require `AuthUser` extractor | MITIGATED |

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
All `/v1/durable/*` HTTP endpoints require `AuthUser` extractor. Unauthenticated requests are rejected.

## 10. Scheduled Tasks (TM-SCHED)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-SCHED-001 | Malicious schedule creation | Medium | Only authenticated users with appropriate permissions can create schedules | MITIGATED |
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
- Event types follow `domain.action.outcome` convention: `auth.login.success`, `auth.login.failure`, `auth.register.success`, `auth.token_refresh.success`, `auth.api_key.created`, `auth.api_key.deleted`, `auth.oauth.success`, `auth.oauth.failure`.
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

### Mitigation Details

**TM-WEB-004 / TM-WEB-005 — Security Headers (MITIGATED):**
Applied via `SetResponseHeaderLayer` (`if_not_present`) in `app_builder.rs`:
- `X-Frame-Options: DENY` — prevents clickjacking
- `X-Content-Type-Options: nosniff` — prevents MIME sniffing
- `Referrer-Policy: strict-origin-when-cross-origin` — limits referrer leakage
- `Permissions-Policy: camera=(), microphone=(), geolocation=()` — disables unused APIs
- `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'`

## 13. AI Agent Behavior (TM-AGENT)

The agent loop is a core trust boundary: an LLM decides which tools to call with what arguments. The system prompt, user messages, tool results, and MCP tool descriptions all influence LLM behavior. Agents are semi-trusted within organizational scope — the agent creator (org member) is trusted, but the LLM's runtime decisions are not fully controllable.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-AGENT-001 | Direct prompt injection via user message | High | Role separation (user vs system); LLM providers apply safety training; no complete defense | **ACCEPTED** |
| TM-AGENT-002 | Indirect prompt injection via tool results | High | Tool results use `tool_result` role, not `system`; LLM may still follow adversarial instructions in results | **ACCEPTED** |
| TM-AGENT-003 | Indirect prompt injection via MCP tool descriptions | Medium | MCP tool names/descriptions fed to LLM as tool schema; adversarial descriptions could influence behavior | **ACCEPTED** |
| TM-AGENT-004 | Agent jailbreak via system prompt | Medium | System prompt set by org member at agent creation; no sanitization of prompt content | **BY DESIGN** |
| TM-AGENT-005 | Capability escalation via agent creation | High | RiskLevel enum on Capability trait; high-risk capabilities (docker, daytona) require Admin role to assign via API | MITIGATED |
| TM-AGENT-006 | Cost runaway — unbounded LLM calls | High | Max iterations per turn (default 100); configurable per agent | MITIGATED |
| TM-AGENT-007 | Cost runaway — many tools per iteration | Medium | No per-iteration tool call limit; agent can invoke many tools in a single LLM response | **OPEN** |
| TM-AGENT-008 | Context window poisoning | Medium | Auto-compaction via `llm_driver.compact()` on `RequestTooLarge`; older messages compressed | MITIGATED |
| TM-AGENT-009 | Agent self-modification | Medium | Agents with `platform_management` capability can modify agents/sessions via tools; capability must be explicitly assigned; org-scoped | **OPEN** |
| TM-AGENT-010 | Agent spawning agent chains | Medium | Agents with `platform_management` capability can create agents/sessions; capability must be explicitly assigned; no recursive depth limit | **OPEN** |
| TM-AGENT-011 | Sensitive data in system prompt | Medium | PII must not be placed in system prompts; no encryption at rest for prompts | **OPEN** |
| TM-AGENT-012 | Tool result size amplification | Medium | No size limit on tool results fed back to LLM; large results consume context and cost | **OPEN** |
| TM-AGENT-013 | Exfiltration via web_fetch | Medium | Agent with web_fetch capability can send session data to arbitrary URLs | **ACCEPTED** |
| TM-AGENT-014 | Confused deputy — tool call with wrong session | Low | Tool context includes session_id; tools scoped to active session only | MITIGATED |
| TM-AGENT-015 | Dangling tool calls cause LLM confusion | Low | Patched with synthetic "cancelled" results before LLM call; prevents API errors | MITIGATED |
| TM-AGENT-016 | Plaintext secrets in chat history | Medium | When agent asks user for API key in chat, plaintext value stored in events table as message content; session secrets encrypt separately but chat retains plaintext | **OPEN** |
| TM-AGENT-017 | Agent-initiated entity management | High | Agents with `platform_management` can create/update/delete harnesses, agents, sessions org-wide; no fine-grained RBAC within org; capability must be explicitly assigned | **OPEN** |

### Mitigation Details

**TM-AGENT-001 / TM-AGENT-002 — Prompt Injection (ACCEPTED):**
Prompt injection is an inherent limitation of current LLM architecture. Defense-in-depth:
1. **Role separation:** System, user, assistant, tool_result messages are distinct roles
2. **Iteration limits:** Max turns prevents infinite manipulation loops
3. **Tool registry:** LLM can only call registered tools (no arbitrary code execution)
4. **Session isolation:** Even if manipulated, agent is confined to its session
5. **No auto-escalation:** Agent cannot grant itself new capabilities

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
Each capability declares a `RiskLevel` (Low, Medium, High) via the `Capability` trait. High-risk capabilities (`docker_container`, `daytona`) require `OrgRole::Admin` to assign. The check runs in create/update/upsert/import agent API handlers, returning 403 if a non-admin user attempts to assign a high-risk capability. The `risk_level` field is exposed in the capabilities list API for UI display.

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
- **Current mitigations:** (1) Capability must be explicitly assigned by an org member. (2) All operations are org-scoped — cross-org access blocked by tenant isolation (TM-TENANT-001). (3) `DirectPlatformStore` uses existing storage layer with org_id filtering. (4) `WorkerAdapters::platform_store(org_id)` receives the session's actual org_id from activity context, preventing cross-org access via hardcoded defaults.
- **Recommendation:** Add audit logging for all platform management tool calls. Consider RBAC (e.g., "can only manage own sessions") and approval workflows for dangerous operations (creating agents with `virtual_bash`). Add recursion depth limits for agent-spawned session chains.
- **Code:** `// THREAT[TM-AGENT-017]` at `PlatformManagementCapability` registration and `DirectPlatformStore` implementation.
- **Priority:** High

## 14. Bash Sandbox (TM-BASH)

Everruns uses [bashkit](https://github.com/everruns/bashkit) (v0.1.2) as a sandboxed bash interpreter for the `virtual_bash` capability. Bashkit provides WASM-like isolation: no real filesystem, no network, no system calls. The session file store is bridged via the `SessionFileSystemAdapter`.

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
| TM-BASH-016 | Write amplification via bash | Medium | No per-session storage quota on files written by bash (see TM-FS-008) | **OPEN** (see TM-FS-008) |

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

## 15. Denial of Service (TM-DOS)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-DOS-001 | Large API request body | High | Input size limits on all fields; multipart upload capped at 101 MB | MITIGATED |
| TM-DOS-002 | Agent loop infinite iteration | High | Max 10 iterations per turn; configurable | MITIGATED |
| TM-DOS-003 | SSE connection exhaustion | Medium | Global (10k), per-org (1k), per-session (5) RAII connection limits via `SseConnectionTracker`; HTTP/2 flow control windows tuned (2 MB/stream, 16 MB/connection) with adaptive sizing; connection cycling with ±20% jitter prevents thundering herd; HTTP/2 PING keepalive detects dead connections | MITIGATED |
| TM-DOS-004 | Database connection pool exhaustion | Medium | sqlx connection pool with max_connections; timeouts on acquisition | MITIGATED |
| TM-DOS-005 | Session file storage abuse | Medium | No per-session storage quota; large files stored as PostgreSQL BYTEA | **OPEN** (see TM-FS-008) |
| TM-DOS-006 | Durable task queue flooding | Medium | Per-workflow pending task limit (see TM-DURABLE-004) | MITIGATED |
| TM-DOS-007 | Nested JSON depth in API input | Medium | Input validation rejects deeply nested structures | MITIGATED |
| TM-DOS-008 | ReDoS via file grep endpoint | Medium | `POST /v1/sessions/:id/fs/_/grep` accepts user regex with no complexity limits | **OPEN** |
| TM-DOS-009 | Valkey unauthenticated access | Medium | Valkey listens on localhost:6379 by default; no AUTH configured in local/example compose | **CALLER RISK** |
| TM-DOS-010 | Rate limit bypass via Valkey failure | Low | Fail-open design: if Valkey is down, requests are allowed without rate limiting | **ACCEPTED** |

### Mitigation Details

**TM-DOS-009 — Valkey Network Exposure (CALLER RISK):**
Valkey (Redis-compatible) is used for distributed rate limiting. In local/dev compose, it runs without authentication on port 6379.
- **Production:** Deploy Valkey on a private network, not exposed to the internet. Use `rediss://` (TLS) URLs and AUTH passwords for cloud-managed instances (e.g., AWS ElastiCache, GCP Memorystore).
- **Blast radius if compromised:** Attacker can flush rate limit counters (bypassing rate limits) or inject fake counters (DoS via false rate-limit-exceeded). No sensitive data stored in Valkey.

**TM-DOS-010 — Fail-Open Rate Limiting (ACCEPTED):**
By design, Valkey errors cause rate limiting to fail open (allow requests). This prioritizes availability over strictness. The blast radius is limited to auth endpoints (login/register/refresh) and only matters if Valkey is persistently down.

## 16. Daytona Cloud Sandbox (TM-DAYTONA)

Daytona sandboxes are remote Linux environments managed via REST API. The agent can create, exec commands, and manage files in these sandboxes. The `daytona_git_credentials` tool writes a GitHub token to disk inside the sandbox to enable git push/pull/fetch operations.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-DAYTONA-001 | Git token persisted on sandbox disk | Medium | Token written to `/tmp/.git-credentials`; lost on sandbox stop/delete; same trust boundary as `daytona_exec` (anyone who can exec can already read the file) | **ACCEPTED** |
| TM-DAYTONA-002 | Git token expiry — stale credentials | Low | GitHub App installation tokens expire in ~1 hour; tool hint tells agent to call `daytona_git_credentials` again to refresh | MITIGATED |
| TM-DAYTONA-003 | Git token scope — over-privileged access | Medium | Token scoped by GitHub App installation permissions; user controls repo access via GitHub App settings | **CALLER RISK** |
| TM-DAYTONA-004 | Daytona API key compromise | High | Stored in user connections (Settings > Connections); encrypted at rest via envelope encryption (AES-256-GCM) | MITIGATED |
| TM-DAYTONA-005 | Cross-session sandbox access | Critical | Sandbox IDs stored in session-scoped secrets (`daytona_sandbox:{id}`); session isolation enforced by storage store | MITIGATED |
| TM-DAYTONA-006 | Sandbox not deleted — resource leak | Low | Auto-stop after 5 min inactivity; system prompt instructs agent to delete when done | MITIGATED |
| TM-DAYTONA-007 | Git credential helper persists after sandbox reuse | Low | Credential file in `/tmp` cleared on stop; sandbox stop resets environment | MITIGATED |

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

Session A cannot access sb_xyz (different session_id in storage query)
```

## 17. Client-Side Tools (TM-CLIENT)

Client-side tools pause server execution and wait for client to submit results via API. Attack surface includes tool call ID spoofing and timeout abuse.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-CLIENT-001 | Tool call ID spoofing | Medium | Submitted `tool_call_id` values must exactly match pending requests; mismatches rejected | MITIGATED |
| TM-CLIENT-002 | Tool result size explosion | Medium | Per-result size capped at 100 KB | MITIGATED |
| TM-CLIENT-003 | Client timeout abuse | Low | Default 5 min timeout; session transitions to failed state on expiry | MITIGATED |

## 18. Brave Search (TM-LLM)

Search results from Brave Search are returned as tool results. Adversarial content in search results could influence LLM behavior.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-LLM-008 | Search result prompt injection | Medium | Results returned as `tool_result` role; inherent LLM limitation (same as TM-TOOL-005) | **ACCEPTED** |
| TM-LLM-009 | Search query privacy | Low | Queries sent to Brave Search (third party); caller responsibility to assess data classification | **CALLER RISK** |

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
| TM-TENANT-008 | User listing cross-org | High | Add org filtering to GET /v1/users |
| TM-DOS-008 | ReDoS via file grep endpoint | Medium | Add regex complexity limits and timeout |
| TM-AGENT-007 | No per-iteration tool call limit | Medium | Cap tool calls per LLM response |
| TM-AGENT-012 | Tool result size amplification | Medium | Cap tool result size fed back to LLM |
| TM-FS-008 | No session storage quota | Medium | Enforce per-session file size limits |
| TM-TOOL-008 | Tool approval not enforced | Low | Implement HITL approval for requires_approval policy |
| TM-TOOL-009 | No tool rate limiting | Medium | Per-agent tool execution rate limits |
| TM-DOS-003 | SSE connection exhaustion | Medium | Global (10k), per-org (1k), per-session (5) limits enforced |
| TM-AGENT-016 | Plaintext secrets in chat history | Medium | Prefer Settings UI; phase out in-chat secret collection |
| TM-AGENT-017 | Agent-initiated entity management | High | Add RBAC for platform management; audit logging; recursion depth limits |

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
| TM-DURABLE-006 | DLQ growth | Tasks preserved for debugging; manual cleanup |
| TM-DAYTONA-001 | Git token on sandbox disk | Same trust boundary as exec; `/tmp` cleared on stop; short-lived token |

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
| Review agent capabilities | TM-AGENT-005, TM-AGENT-013 | High-risk capabilities require Admin role; audit capability assignments for org admin accounts |
| System prompt review | TM-AGENT-004 | Review agent system prompts for jailbreak patterns before deployment |
| Block cloud metadata | TM-API-009 | Defense-in-depth: enable IMDSv2 (AWS), metadata concealment (GCP), or equivalent; fetchkit v0.1.2 blocks 169.254.0.0/16 at application level |
| Worker network isolation | TM-API-008, TM-API-010, TM-API-011 | Defense-in-depth: restrict worker container egress; fetchkit v0.1.2 blocks private IPs at application level |
| Review GitHub App permissions | TM-DAYTONA-003 | Audit which repositories the GitHub App installation can access; Everruns does not enforce per-repo restrictions |

## Security Controls Matrix

| Control | Category | Implementation |
|---------|----------|----------------|
| Authentication | TM-AUTH | JWT (15 min), API keys (SHA-256), OAuth, Argon2id passwords |
| Authorization | TM-TENANT | Org-scoped queries, ResolvedOrg extractor, 404 on cross-org |
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
| Slack webhook forgery | TM-SLACK-001 | HMAC-SHA256 signing secret verification, 5-min replay window |
| Slack bot loop | TM-SLACK-002 | Skip events with `bot_id` or `subtype` to prevent infinite loops |
| Slack signing secret exposure | TM-SLACK-003 | Stored in `channel_config` (org-scoped access), not logged |

## References

- `specs/authentication.md` — Authentication modes, JWT, API keys, OAuth
- `specs/encryption.md` — Envelope encryption design
- `specs/multitenancy.md` — Org-based isolation model
- `specs/session-filesystem.md` — Session file storage and path validation
- `specs/session-sqldb.md` — SQLite sandbox and VFS design
- `specs/tool-execution.md` — Tool types and execution flow
- `specs/mcp-servers.md` — MCP server integration
- `specs/llm-drivers.md` — LLM provider abstraction
- `specs/durable-execution-engine.md` — Workflow engine and worker communication
- `specs/scheduled-tasks.md` — Cron-based task scheduling
- `specs/otel-observability.md` — OpenTelemetry tracing
- `specs/braintrust-integration.md` — Braintrust event forwarding
- `specs/apis.md` — HTTP API endpoints and error handling
- `specs/capabilities.md` — Agent capabilities system
- `specs/bashkit-requirements.md` — Bashkit integration requirements
- `specs/daytona.md` — Daytona cloud sandbox integration
- `specs/client-side-tools.md` — Client-side tools for API/SDK consumers
- `specs/apps.md` — Apps system (agent deployment to channels)
- `specs/slack-integration.md` — Slack bot integration
- `specs/brave-search.md` — Brave Search web search integration
- `specs/infinity-context.md` — Unlimited conversation length via context management
- [fetchkit v0.1.2 source](https://crates.io/crates/fetchkit) — SSRF protection (resolve-then-check, DNS pinning, DnsPolicy), URL prefix blocking, fetch options, fetcher registry
