# Audit Logging

Structured audit logging for security-relevant and operational events.

## Status

Implements [EVE-226](https://linear.app/everruns/issue/EVE-226).

## Overview

Two audit domains cover complementary surfaces:

| Domain | What it tracks | Who writes |
|--------|---------------|------------|
| **Management** | Org CRUD, member invite/remove, role change, API key create/revoke, settings changes | Service layer via `#[audit]` macro |
| **Agent** | Agent runs, tool calls, LLM interactions | Execution engine (future) |

## Core Types

See `crates/core/src/audit.rs` for full definitions.

- `AuditDomain` — enum: `Management`, `Agent`
- `AuditAction` — typed action per domain (e.g. `MemberInvited`, `AgentRunStarted`)
- `AuditTarget` — `{ target_type, target_id }` identifying the affected resource
- `AuditEvent` — full event: domain, action, actor, org, target, details (JSON), timestamp, IP
- `AuditLogger` trait — `async fn log_event(&self, event: AuditEvent) -> Result<()>`

### Domain-Specific Builders

Type-safe builders prevent mixing domains:

```rust
AuditEvent::management(action, caller)
    .target("member", user_id)
    .detail("role", "admin")
    .build()

AuditEvent::agent(action, caller)
    .target("session", session_id)
    .build()
```

## AOP: `#[audit]` Proc Macro

Cross-cutting audit logging via attribute macro on service methods. Emits an audit event after successful execution without polluting business logic.

```rust
#[audit(Management, MemberInvited, target = "member")]
pub async fn invite_member(&self, caller: &Caller, req: InviteRequest) -> Result<Member> {
    // business logic only — audit event emitted automatically on success
}
```

The macro:
1. Finds `&Caller` parameter (same as `#[policy]`)
2. Finds `&self` to access `self.audit_logger`
3. Wraps the function body: executes original, on `Ok` emits audit event via fire-and-forget `tokio::spawn`
4. The target ID is extracted from the `Ok` result if it implements `AuditTarget` trait, or from a named field

Composes with `#[policy]`: policy checks run first (injected at top), audit fires after success.

```rust
#[policy(MEMBER_MANAGE)]
#[audit(Management, MemberInvited, target = "member")]
pub async fn invite(&self, caller: &Caller, req: Req) -> Result<Member> { ... }
```

## Storage

### Migration

Adds columns to existing `audit_logs` table (migration 010):

```sql
ALTER TABLE audit_logs
    ADD COLUMN domain TEXT NOT NULL DEFAULT 'management',
    ADD COLUMN action TEXT NOT NULL DEFAULT '',
    ADD COLUMN target_type TEXT,
    ADD COLUMN target_id TEXT;

CREATE INDEX idx_audit_logs_org_domain_created
    ON audit_logs (org_id, domain, created_at DESC);
```

### PostgreSQL Implementation

`AuditLogger` implemented on `StorageBackend` — delegates to repo layer. Fire-and-forget via `tokio::spawn` (TM-OBS-007: never blocks caller).

### Retention

Configurable auto-purge via `AUDIT_LOG_RETENTION_DAYS` env var (default: 90 days). Existing `delete_audit_logs_before` method handles cleanup.

## API

### `GET /v1/orgs/{org}/audit-logs`

Paginated, filterable. Requires `OrgAuditLogsView` permission (Owner/Admin only).

Query parameters:
- `limit` — max entries (default 50, max 200)
- `before` — cursor timestamp
- `domain` — filter by domain (`management` or `agent`)
- `action` — filter by action string
- `actor_id` — filter by actor UUID
- `event_type` — legacy filter (prefix match on `event_type`)

Response includes `domain`, `action`, `target_type`, `target_id` fields.

## Permissions

| Permission | Granted to | Purpose |
|-----------|-----------|---------|
| `OrgAuditLogsView` | Owner, Admin | Read audit logs via API |

Policy: `AUDIT_LOG_VIEW` requires `OrgAuditLogsView`.

## Security Decisions

- **Fire-and-forget**: Audit failures never block operations (TM-OBS-007)
- **Tenant isolation**: All queries include `org_id` filter
- **Admin-only access**: Policy-gated, not just role check
- **No mutation API**: Audit logs are append-only; no update/delete endpoints
- **IP tracking**: Client IP captured from proxy headers for forensics
- **JSONB details**: Freeform but never contains secrets (PII limited to IDs)

## Threat Model References

- TM-OBS-007: Audit log write failures must not block operations
- TM-TENANT: Org-scoped queries prevent cross-tenant data leaks
- TM-AUTHZ: Policy enforcement on read endpoint
