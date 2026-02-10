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

**Trust boundary 2 — Control Plane → Workers:** Workers are stateless executors with no database credentials. Communication via gRPC. Currently relies on network isolation (no mutual auth).

**Trust boundary 3 — Workers → External Services:** LLM providers and MCP servers are external. API keys transmitted over HTTPS. MCP responses parsed defensively.

**Trust boundary 4 — LLM → Agent Tools:** The LLM decides which tools to call and with what arguments. The agent loop executes LLM-chosen tool calls within sandboxed capabilities. The LLM is semi-trusted: it operates within registered tools and iteration limits, but its outputs (tool arguments, text) are not validated for intent.

## 1. Authentication (TM-AUTH)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-AUTH-001 | Brute force login | High | No rate limiting on `/v1/auth/login` | **OPEN** |
| TM-AUTH-002 | JWT secret compromise | Critical | Stored in env var `AUTH_JWT_SECRET`; min 32 bytes recommended; never logged | MITIGATED |
| TM-AUTH-003 | Token replay after logout | Medium | Refresh tokens stored in DB, revocable via DELETE; access tokens short-lived (15 min) | MITIGATED |
| TM-AUTH-004 | Weak password | Medium | Minimum 8 characters enforced; Argon2id hashing | MITIGATED |
| TM-AUTH-005 | API key exposure in transit | High | HTTPS required in production; keys prefixed `evr_` for scanning | MITIGATED |
| TM-AUTH-006 | API key brute force | Medium | Keys stored as SHA-256 hashes; 128-bit entropy makes brute force infeasible | MITIGATED |
| TM-AUTH-007 | OAuth state fixation | High | State parameter validated in callback flow | MITIGATED |
| TM-AUTH-008 | Session fixation via cookie | Medium | New tokens issued on login; HTTP-only, SameSite=Lax cookies | MITIGATED |
| TM-AUTH-009 | Refresh token theft | High | Stored hashed in DB; HTTP-only cookie; revocable | MITIGATED |
| TM-AUTH-010 | Admin password in env var | Low | Limited to admin mode; documented risk; shell history exposure possible | **ACCEPTED** |
| TM-AUTH-011 | Auth bypass in `none` mode | Info | By design for local development; anonymous user gets admin role | **BY DESIGN** |
| TM-AUTH-012 | OAuth account linking collision | Medium | Accounts linked by email; if attacker controls email at provider, they gain access | **CALLER RISK** |
| TM-AUTH-013 | Expired API key still in use | Medium | Expiration checked on every request via DB lookup; `last_used_at` tracked | MITIGATED |

### Mitigation Details

**TM-AUTH-001 — Rate Limiting (OPEN):**
No rate limiting is currently implemented on authentication endpoints. An attacker can attempt unlimited logins.
- **Recommendation:** Implement per-IP rate limiting: 5 attempts/min on `/v1/auth/login`, exponential backoff on failures.
- **Priority:** High

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
| TM-CRYPTO-007 | Limited encryption scope | Medium | Only LLM API keys encrypted; other sensitive fields (system prompts, session data) stored plaintext | **OPEN** |

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
| TM-API-008 | WebFetch SSRF | Medium | Agent-controlled, not user-controlled URLs; agents are semi-trusted within session scope | **ACCEPTED** |

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

## 5. Session Filesystem (TM-FS)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-FS-001 | Path traversal | High | DB constraint rejects `..`; paths must match `^/([^/\0]+(/[^/\0]+)*)?$` | MITIGATED |
| TM-FS-002 | Cross-session file access | Critical | Files scoped by `session_id` FK; session scoped by org via agent join | MITIGATED |
| TM-FS-003 | Null byte injection | High | Regex constraint rejects `\0` bytes | MITIGATED |
| TM-FS-004 | Double-slash bypass | Medium | Regex constraint rejects `//` | MITIGATED |
| TM-FS-005 | Readonly file modification | Medium | `is_readonly` flag enforced; readonly files cannot have content updated | MITIGATED |
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
| TM-DURABLE-002 | gRPC unauthenticated access | High | Internal port 9001; relies on network isolation; no mutual auth | **OPEN** |
| TM-DURABLE-003 | Event injection | Medium | Events created via gRPC only; validated for session membership | MITIGATED |
| TM-DURABLE-004 | Queue flooding | Medium | No rate limiting on task creation; backpressure via worker watermarks | **OPEN** |
| TM-DURABLE-005 | Heartbeat timeout manipulation | Low | 30s timeout is reasonable for LLM operations; reclaimed tasks re-queued | MITIGATED |
| TM-DURABLE-006 | Dead letter queue growth | Low | Failed tasks preserved in DLQ; no automatic cleanup | **ACCEPTED** |
| TM-DURABLE-007 | Task state manipulation | Medium | Tasks immutable after creation; only status transitions allowed via state machine | MITIGATED |
| TM-DURABLE-008 | Worker impersonation | High | No worker authentication; any process on the network can claim tasks | **OPEN** (same as TM-DURABLE-002) |
| TM-DURABLE-009 | Replay attack on workflow events | Low | Event store is append-only; events processed in sequence order | MITIGATED |

### Mitigation Details

**TM-DURABLE-001 — Task Ownership:**
```
Worker A claims task → heartbeat timeout → task reclaimed by Worker B
Worker A finishes late → CompleteDurableTask → TaskNotOwned error
Worker B continues execution → task completes correctly
```
Prevents duplicate activity execution when workers lose connectivity.

**TM-DURABLE-002 — gRPC Security (OPEN):**
Workers connect to control plane gRPC (port 9001) without authentication. Any process with network access to port 9001 can:
- Claim tasks (`ClaimDurableTasks`)
- Emit events (`EmitEventStream`)
- Complete/fail tasks (`CompleteDurableTask`/`FailDurableTask`)

Current mitigation: Network isolation (private network, no public exposure).
- **Recommendation:** Implement mTLS or bearer token authentication for worker↔control-plane gRPC.
- **Priority:** High for production deployments

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

### Mitigation Details

**TM-OBS-001 — Braintrust Data Flow:**
```
Agent turn → events emitted → BraintrustEventListener (async)
    → Convert to OpenAI format
    → POST /v1/project_logs/{project_id}/insert
    → Fire-and-forget (no retry)
```
Full conversation data (user messages, LLM responses, tool results) is transmitted. Organizations must evaluate whether Braintrust integration is appropriate given their data classification requirements.

## 12. Web Security (TM-WEB)

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-WEB-001 | XSS via stored content | Medium | React UI auto-escapes; file preview uses Shiki (no raw HTML injection) | MITIGATED |
| TM-WEB-002 | CSRF on state-changing requests | Medium | SameSite=Lax cookies; JSON content type required; no GET side effects | MITIGATED |
| TM-WEB-003 | Cookie theft via XSS | High | Refresh token cookie: HTTP-only; access token cookie: HTTP-only | MITIGATED |
| TM-WEB-004 | Clickjacking | Medium | No X-Frame-Options or CSP frame-ancestors header set | **OPEN** |
| TM-WEB-005 | Missing security headers | Low | No Content-Security-Policy, X-Content-Type-Options, Referrer-Policy headers | **OPEN** |
| TM-WEB-006 | Open redirect in OAuth flow | Medium | OAuth callbacks validated against configured redirect URIs | MITIGATED |
| TM-WEB-007 | CORS wildcard exposure | Medium | `CORS_ALLOWED_ORIGINS` not set by default; must be explicitly configured | MITIGATED |

### Mitigation Details

**TM-WEB-004 / TM-WEB-005 — Security Headers (OPEN):**
- **Recommendation:** Add the following response headers:
  ```
  X-Frame-Options: DENY
  X-Content-Type-Options: nosniff
  Referrer-Policy: strict-origin-when-cross-origin
  Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'
  ```
- **Priority:** Medium

## 13. AI Agent Behavior (TM-AGENT)

The agent loop is a core trust boundary: an LLM decides which tools to call with what arguments. The system prompt, user messages, tool results, and MCP tool descriptions all influence LLM behavior. Agents are semi-trusted within organizational scope — the agent creator (org member) is trusted, but the LLM's runtime decisions are not fully controllable.

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-AGENT-001 | Direct prompt injection via user message | High | Role separation (user vs system); LLM providers apply safety training; no complete defense | **ACCEPTED** |
| TM-AGENT-002 | Indirect prompt injection via tool results | High | Tool results use `tool_result` role, not `system`; LLM may still follow adversarial instructions in results | **ACCEPTED** |
| TM-AGENT-003 | Indirect prompt injection via MCP tool descriptions | Medium | MCP tool names/descriptions fed to LLM as tool schema; adversarial descriptions could influence behavior | **ACCEPTED** |
| TM-AGENT-004 | Agent jailbreak via system prompt | Medium | System prompt set by org member at agent creation; no sanitization of prompt content | **BY DESIGN** |
| TM-AGENT-005 | Capability escalation via agent creation | High | Agent creator chooses capabilities; no approval workflow for dangerous capabilities (virtual_bash, docker) | **OPEN** |
| TM-AGENT-006 | Cost runaway — unbounded LLM calls | High | Max iterations per turn (default 100); configurable per agent | MITIGATED |
| TM-AGENT-007 | Cost runaway — many tools per iteration | Medium | No per-iteration tool call limit; agent can invoke many tools in a single LLM response | **OPEN** |
| TM-AGENT-008 | Context window poisoning | Medium | Auto-compaction via `llm_driver.compact()` on `RequestTooLarge`; older messages compressed | MITIGATED |
| TM-AGENT-009 | Agent self-modification | Low | No tools for agent/session CRUD; system prompt immutable within a session | MITIGATED |
| TM-AGENT-010 | Agent spawning agent chains | Low | No agent-creation tools; agents cannot spawn child agents or sessions | MITIGATED |
| TM-AGENT-011 | Sensitive data in system prompt | Medium | System prompts stored plaintext in DB; not encrypted at rest | **OPEN** (see TM-CRYPTO-007) |
| TM-AGENT-012 | Tool result size amplification | Medium | No size limit on tool results fed back to LLM; large results consume context and cost | **OPEN** |
| TM-AGENT-013 | Exfiltration via web_fetch | Medium | Agent with web_fetch capability can send session data to arbitrary URLs | **ACCEPTED** |
| TM-AGENT-014 | Confused deputy — tool call with wrong session | Low | Tool context includes session_id; tools scoped to active session only | MITIGATED |
| TM-AGENT-015 | Dangling tool calls cause LLM confusion | Low | Patched with synthetic "cancelled" results before LLM call; prevents API errors | MITIGATED |

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

**TM-AGENT-005 — Capability Escalation (OPEN):**
Capabilities are all-or-nothing. An org member can create an agent with `virtual_bash` + `web_fetch`, giving it the ability to execute bash commands and exfiltrate results via HTTP. No approval workflow exists for dangerous capability combinations.

- **Recommendation:** Implement HITL approval for high-risk capabilities (`virtual_bash`, `docker_container`). Consider capability scoping (e.g., read-only filesystem).
- **Priority:** Medium

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
| TM-DOS-003 | SSE connection exhaustion | Medium | SSE connections tied to session; no global connection limit | **OPEN** |
| TM-DOS-004 | Database connection pool exhaustion | Medium | sqlx connection pool with max_connections; timeouts on acquisition | MITIGATED |
| TM-DOS-005 | Session file storage abuse | Medium | No per-session storage quota; large files stored as PostgreSQL BYTEA | **OPEN** (see TM-FS-008) |
| TM-DOS-006 | Durable task queue flooding | Medium | No rate limiting on task creation; worker backpressure is reactive | **OPEN** (see TM-DURABLE-004) |
| TM-DOS-007 | Nested JSON depth in API input | Medium | Input validation rejects deeply nested structures | MITIGATED |

## Vulnerability Summary

### Open Threats (Require Action)

| ID | Threat | Severity | Recommendation |
|----|--------|----------|----------------|
| TM-AUTH-001 | No rate limiting on login | High | Per-IP rate limiting on auth endpoints |
| TM-DURABLE-002 | gRPC unauthenticated | High | mTLS or bearer token auth for workers |
| TM-AGENT-005 | No approval for dangerous capabilities | High | HITL approval for virtual_bash, docker |
| TM-AGENT-007 | No per-iteration tool call limit | Medium | Cap tool calls per LLM response |
| TM-AGENT-012 | Tool result size amplification | Medium | Cap tool result size fed back to LLM |
| TM-CRYPTO-007 | Limited encryption scope | Medium | Encrypt system prompts and other sensitive fields |
| TM-WEB-004 | Missing clickjacking protection | Medium | Add X-Frame-Options: DENY |
| TM-WEB-005 | Missing security headers | Low | Add CSP, X-Content-Type-Options, Referrer-Policy |
| TM-FS-008 | No session storage quota | Medium | Enforce per-session file size limits |
| TM-TOOL-008 | Tool approval not enforced | Low | Implement HITL approval for requires_approval policy |
| TM-TOOL-009 | No tool rate limiting | Medium | Per-agent tool execution rate limits |
| TM-DOS-003 | SSE connection exhaustion | Medium | Global SSE connection limit with per-user cap |

### Accepted Risks

| ID | Threat | Rationale |
|----|--------|-----------|
| TM-AUTH-010 | Admin password in env var | Limited to development mode; documented |
| TM-AUTH-011 | Auth bypass in none mode | By design for local development |
| TM-API-008 | WebFetch SSRF | Agent-controlled, not user-controlled URLs |
| TM-FS-006 | File content unencrypted at rest | Relies on infrastructure encryption |
| TM-FS-007 | No file access audit log | Privacy tradeoff; not compliance-required |
| TM-LLM-007 | Indirect prompt injection | Inherent LLM limitation; mitigated by role separation |
| TM-AGENT-001 | Direct prompt injection | Inherent LLM limitation; role separation + iteration limits |
| TM-AGENT-002 | Indirect prompt injection via tool results | Tool results role-separated; no complete defense exists |
| TM-AGENT-003 | MCP tool description poisoning | MCP servers org-configured; descriptions used as schemas only |
| TM-AGENT-013 | Data exfiltration via web_fetch | Opt-in capability; org members trusted; intended functionality |
| TM-DURABLE-006 | DLQ growth | Tasks preserved for debugging; manual cleanup |

### Caller Responsibilities

| Responsibility | Related Threats | Description |
|---------------|-----------------|-------------|
| Enable TLS/HTTPS | TM-AUTH-005, TM-LLM-006 | All production traffic must use HTTPS |
| Secure env vars | TM-AUTH-002, TM-CRYPTO-001 | Never commit secrets to source control |
| Configure CORS | TM-API-007, TM-WEB-007 | Set explicit allowed origins in production |
| Network isolation | TM-DURABLE-002 | Keep gRPC port 9001 on private network |
| Evaluate Braintrust | TM-OBS-001 | Assess data classification before enabling |
| Secure OTLP endpoint | TM-OBS-003 | Use trusted internal infrastructure only |
| OAuth provider trust | TM-AUTH-012 | Verify email ownership at OAuth providers |
| Review agent capabilities | TM-AGENT-005, TM-AGENT-013 | Audit capability assignments; avoid virtual_bash + web_fetch on untrusted agents |
| System prompt review | TM-AGENT-004 | Review agent system prompts for jailbreak patterns before deployment |

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
| Tool validation | TM-TOOL | Registry-based validation, defensive MCP parsing |
| Resource limits | TM-DOS, TM-BASH | Input sizes, iteration limits, query timeouts, bash limits |
| Task ownership | TM-DURABLE | Verified on completion, heartbeat-based reclaim |

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
