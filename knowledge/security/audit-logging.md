---
type: Specification
title: "Audit Logging"
description: "Audit logging."
tags:
  - everruns
  - security
---
# Audit Logging

Structured audit logging for security-relevant and operational events.

## Status

Implements [EVE-226](https://linear.app/everruns/issue/EVE-226).

## Overview

Two audit domains cover complementary surfaces:

| Domain | What it tracks | Who writes |
|--------|---------------|------------|
| **Management** | Org CRUD, member invite/remove, role change, API key create/revoke, settings changes | Service layer via `#[audit]` macro |
| **Agent** | Agent runs, app-channel invocations, tool calls, LLM interactions | Execution engine and app-channel ingress |

## Core Types

See `crates/core/src/audit.rs` for full type definitions (`AuditDomain`, `AuditAction`, `AuditTarget`, `AuditEvent`, `AuditLogger` trait) and domain-specific builders.

## App-Channel Invocations

Webhook, schedule, and A2A app-channel invocations emit
`agent.app_invocation.started` after the session is resolved and the rendered
user message is dispatched. The target is the app channel; metadata records the
source (`app_webhook`, `app_schedule`, or `app_a2a`), app id, channel id/type,
session id, whether a new session was created, the app owner principal id, and
the agent identity id when the invocation runs through an agent identity.

## AOP: `#[audit]` Proc Macro

Cross-cutting audit logging via attribute macro on service methods. Emits an audit event after successful execution without polluting business logic.

```rust
#[audit(ManagementAction::MemberInvited, target_type = "member")]
pub async fn invite_member(&self, caller: &Caller, req: InviteRequest) -> Result<Member> {
    // business logic only — audit event emitted automatically on success
}
```

The macro:
1. Finds the `&Caller` parameter by type
2. Finds `&self` to access `self.audit_logger()` (must return a concrete cloneable `AuditLogger` type)
3. Wraps the function body: executes original, on `Ok` emits audit event via fire-and-forget `tokio::spawn`
4. When `target_type` is specified, the target ID is extracted from the `Ok` result via `HasAuditTargetId` trait
5. When `target_type` is omitted, no target is recorded

Authorization is enforced at the command layer (`Command::run` in `crates/server/src/domains/common.rs`), not at the service method, so `#[audit]` is the only attribute macro used on service methods today.

## Storage

### Migration

See `crates/server/migrations/` for the audit_logs schema (migration 010 adds domain, action, target columns and index).

### PostgreSQL Implementation

`AuditLogger` implemented on `StorageBackend`, delegates to repo layer. Fire-and-forget via `tokio::spawn` (TM-OBS-007: never blocks caller).

### Retention

Configurable auto-purge via `AUDIT_LOG_RETENTION_DAYS` env var (default: 90 days). Existing `delete_audit_logs_before` method handles cleanup.

## API

### `GET /v1/orgs/{org}/audit-logs`

Paginated, filterable. Requires `OrgAuditLogsView` permission (Owner/Admin only).

Query parameters:
- `limit`, max entries (default 50, max 200)
- `before`, cursor timestamp
- `domain`, filter by domain (`management` or `agent`)
- `action`, filter by action string
- `actor_id`, filter by actor UUID
- `event_type`, legacy filter (prefix match on `event_type`)

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
- **IP tracking**: Client IP uses the auth rate limiter's trusted-proxy
  extraction contract before proxy headers are recorded for forensics
- **JSONB details**: Freeform but never contains secrets (PII limited to IDs)

## Threat Model References

- TM-OBS-007: Audit log write failures must not block operations
- TM-TENANT: Org-scoped queries prevent cross-tenant data leaks
- TM-AUTHZ: Policy enforcement on read endpoint
