---
type: Test Case
title: "TC002: Service MCP Grant Lifecycle"
description: "Verify that authorizing a service MCP attachment binds the grant to the agent's identity, that two different users reach the remote as that identity, and that revoking returns the attachment to connection_required."
tags:
  - everruns
  - test-case
  - ui
  - agent-credentials
---
# TC002: Service MCP Grant Lifecycle

## Description

Verify that authorizing a service MCP attachment binds the grant to the agent's
own identity, that two different invoking users both reach the remote as that
identity, and that revoking returns the attachment to `connection_required` on
the next call.

This is the write half of `actsAs: service`. Resolution already reads only
`agent_identity_connections` for a service attachment (EVE-1029), so the
property under test is that authorization puts the grant in that store and
nowhere else — in particular not under the admin who performed it.

## Preconditions

- Canonical local stack is running per the root `AGENTS.md`.
- An org MCP server preset exists with `auth_mode = oauth` and reachable OAuth
  metadata, registered as an active catalog entry.
- An Agent has a scoped MCP attachment referencing that preset with
  `actsAs: service`.
- Two distinct non-admin users exist who can both invoke the Agent.
- One admin user holds both `org:mcp-servers:manage` and
  `org:agent-identities:manage`.
- Use a disposable OAuth account; never authorize a real production account.

## Test Data

| Field | Value |
| --- | --- |
| Attachment `actsAs` | `service` |
| Preset reference | `catalog:<preset-name>` |
| Authorizing user | admin holding both manage permissions |
| Invoking users | two distinct users, neither an admin |

## Steps

1. As one of the non-admin users, invoke the Agent so it calls a tool from the
   service attachment. Observe the result.
2. As the admin, authorize the service attachment for the Agent and complete
   the OAuth consent as the disposable account.
3. Inspect the Agent: confirm it now has an agent identity, and that the
   identity holds a connection for the preset's provider.
4. Inspect the admin's own connections page.
5. As the first non-admin user, invoke the Agent so it calls the tool again.
6. As the second non-admin user, invoke the Agent so it calls the same tool.
7. As a user holding neither manage permission, attempt to authorize the same
   attachment.
8. As the admin, revoke the identity's connection for that provider.
9. As either non-admin user, invoke the Agent so it calls the tool again.

## Expected Result

- Step 1: the call returns `connection_required` rather than succeeding or
  returning a raw 401 — a service attachment with no grant fails closed.
- Step 2: consent completes and returns to the app without error.
- Step 3: the Agent has an identity, and the grant is listed against that
  identity.
- Step 4: **the admin has no new connection for that provider.** The grant
  belongs to the agent, not to the person who authorized it. A grant appearing
  here is a failure even if every other step passes.
- Steps 5 and 6: both calls succeed, and the remote shows the *same* account
  for both users — the disposable account, not either invoking user.
- Step 7: refused with a permission error, and no agent identity is created as
  a side effect of the refused call.
- Step 8: revocation succeeds.
- Step 9: the call returns `connection_required` again, with no cached token
  surviving from the earlier successful calls.
