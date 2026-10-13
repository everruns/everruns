---
type: Test Case
title: "TC002: Auth - Scope and Permission Parity"
description: "Verify read-only credential restrictions and caller permissions agree across REST, command dispatch, and MCP, with no side effects on denial."
tags:
  - everruns
  - test-case
  - api
  - auth
---
# TC002: Auth - Scope and Permission Parity

## Description

Verify read-only credential restrictions and caller permissions agree across REST, command dispatch, and MCP, with no side effects on denial.

## Preconditions

- Authenticated stack and disposable owner, admin, and member fixtures with known org roles.
- Full-access and read-only PATs for the owner; cookies excluded from PAT requests.
- A disposable active MCP preset, an archived preset, an agent, and a harness in the selected org.
- Requests use the current command schemas from `GET /api/v1/commands`. For MCP, initialize normally and follow the published tool input schemas; do not guess payloads.

## Test Data

| Caller | Operation | Expected |
| --- | --- | --- |
| Owner, read PAT | List agents via REST and `list_agents` command | Allowed |
| Owner, read PAT | Create/update agent via REST and matching commands | 403 `insufficient_scope` |
| Owner, read PAT | Token list/create, account export | 403 `insufficient_scope` |
| Member, full credential | List harnesses | Allowed |
| Member, full credential | Create harness | 403 |
| Member, full credential | Assign a current High-risk capability to an agent | 403; agent unchanged |
| Admin/member, full credential | Archive MCP preset | Allowed |
| Admin/member, full credential | Destroy archived MCP preset | 403 |
| Owner, full credential | Destroy archived MCP preset | Allowed |

## Steps

1. Record fixture state and execution counts. Execute each row over its REST route, then the matching `POST /api/v1/commands/<name>` entry point with the same credential and org.
2. Initialize MCP using each owner's PAT. Inspect tools/list and discover the available commands.
3. With the read-only PAT, perform an authorized read, then attempt the equivalent write with `tools/call` using the schema saved from full-access discovery.
4. Inspect the MCP result, including its error flag. A successful HTTP transport alone is not a successful operation.
5. Re-read fixtures and execution history using the owner session. Inspect resource `/config` policy results for member/admin/owner and compare allowed actions.
6. Repeat a denied mutation as the allowed caller on a separate disposable fixture to prove the operation itself is runnable.

## Expected Result

- REST and direct command dispatch agree on credential scope and permission outcomes.
- REST/direct-command scope denials return 403 with `insufficient_scope`; scope-denied HTTP responses carry the Bearer insufficient-scope challenge.
- Read-only MCP discovery filters unavailable operations; an explicit write call produces a tool/operation error and changes nothing.
- Full access cannot override the caller's role. Ordinary org role does not imply platform-user status.
- Denied operations neither create records nor alter fixtures nor start agent work. Authorized controls and `/config` hints agree with server enforcement.

## Source Contract

[Permissions](../../../security/permissions.md), [scope classification and enforcement](../../../../crates/server/src/auth/pat_scope/mod.rs), and [command runner](../../../../crates/server/src/domains/common/mod.rs).
