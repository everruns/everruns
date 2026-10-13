---
type: Test Case
title: "TC001: Auth - Credential and Tenant Boundaries"
description: "Verify explicit organization selection, header precedence, tenant isolation, and non-member resource resolution using disposable authenticated fixtures."
tags:
  - everruns
  - test-case
  - api
  - auth
---
# TC001: Auth - Credential and Tenant Boundaries

## Description

Verify explicit organization selection, header precedence, tenant isolation, and non-member resource resolution using disposable authenticated fixtures.

## Preconditions

- Canonical `AUTH_MODE=full` stack; use its proxy origin as `BASE_URL` and `/api/v1` for API requests.
- Account A belongs to Org A and Org B; account C belongs only to Org C.
- A full-access PAT for A, plus disposable revoked and already-expired credential fixtures; disposable agents and sessions in each org, with IDs recorded privately.
- Send only the credential required by each row. A browser cookie must not accidentally authenticate a PAT or logged-out check.

## Test Data

| Credential and org selection | Request | Expected |
| --- | --- | --- |
| No credential, no cookies | GET agents | 401 |
| Invalid/revoked/expired PAT, no cookies | GET agents | 401 |
| A's PAT; header A | GET agents | 200; only A's list |
| A's PAT; cookie B | GET agents | 200; only B's list |
| A's PAT; header A, cookie B | GET agents | 200; header A wins |
| A's PAT; no org header/cookie | GET agents | 400; explicit selection required |
| A's PAT; well-formed non-member Org C | GET agents | 404 |
| A's PAT; selected Org A, Org B resource ID | GET agent/session detail | 404 |
| A's PAT; selected Org A, Org B agent ID | PATCH agent description | 404; unchanged |

## Steps

1. Execute every row against the corresponding `/agents` or `/sessions` route. Compare returned IDs against the recorded fixtures, not total counts.
2. Resolve the Org B agent and session through `GET /api/v1/resolve-org?id=<id>` as A. Expect B's org id/name; the detail endpoint itself still requires B's context.
3. Repeat resolve-org with Org C's fixture, an unknown same-type ID, and an unknown prefix. Compare status and response bodies.
4. Repeat resolution without credentials. Re-read all fixtures using their authorized account and org.
5. With a separate single-org account's PAT and no org selection, verify its own list loads through the single-org fallback.

## Expected Result

- Every row matches its stated status and exact fixture set; no request leaks another org's data.
- Non-member and unknown resolution return the same 404 outcome without an org id/name; unauthenticated resolution returns 401.
- Cross-org mutation has no side effect. The denied requests do not change selected org or membership.
- A full PAT inherits the user's permissions and memberships; it does not grant access to unrelated organizations.

## Source Contract

[Organization resolution](../../../security/multitenancy.md), [credential resolution](../../../../crates/server/src/auth/middleware/mod.rs), and [resource resolution](../../../../crates/server/src/api/resolver.rs).
