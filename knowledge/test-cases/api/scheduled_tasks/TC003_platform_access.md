---
type: Test Case
title: "TC003: Durable Schedules - Platform Access"
description: "Verify durable schedules require explicit platform-user access rather than organization ownership, and read-only credentials cannot mutate them."
tags:
  - everruns
  - test-case
  - api
  - scheduled-tasks
---
# TC003: Durable Schedules - Platform Access

## Description

Verify durable schedules require explicit platform-user access rather than organization ownership, and read-only credentials cannot mutate them.

## Preconditions

- Authenticated stack with a platform-user fixture and a non-platform org-owner fixture.
- One disposable paused durable schedule; avoid triggering production/system schedules.
- Full-access and read-only PATs for the platform user. No browser cookies in PAT checks.

## Test Data

| Caller | Read list/detail/executions | Create/update/trigger/delete |
| --- | --- | --- |
| Unauthenticated | 401 | 401 |
| Non-platform org owner | 403 | 403 |
| Platform user, full credential | Allowed | Allowed |
| Platform user, read-only PAT | Allowed | 403 `insufficient_scope` |

## Steps

1. Execute each row through `/api/v1/durable/schedules` and the disposable schedule's detail, executions, and mutation endpoints.
2. Inspect `/api/v1/durable/config` for the non-platform and platform fixtures; compare navigation availability to policy results.
3. Repeat representative read and mutation operations through the matching command entry points.
4. Select a different org for the platform user and re-read the schedule. Durable schedules are global platform resources, not tenant session schedules.
5. Verify denied mutations left the paused schedule and execution count unchanged.

## Expected Result

- Every row matches its access outcome across HTTP and command dispatch.
- Owning a tenant org does not grant platform access; changing `X-Org-Id` cannot elevate a caller.
- Read-only scope remains enforced even for platform users.
- Platform schedule visibility does not depend on the selected tenant; tenant-scoped scheduling is tested through its own APIs.

## Source Contract

[Schedule policies](../../../../crates/server/src/domains/schedules/mod.rs) and [platform-user contract](../../../security/permissions.md).
