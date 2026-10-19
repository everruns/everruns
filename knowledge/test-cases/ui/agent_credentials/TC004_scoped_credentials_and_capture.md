---
type: Test Case
title: "TC004: Scoped Credentials and Capture Consent"
description: "Verify scoped secure entry, sharing, revocation, and exact-request capture consent in the UI."
tags:
  - everruns
  - test-case
  - ui
  - agent-credentials
---
# TC004: Scoped Credentials and Capture Consent

## Preconditions

- Canonical DB-backed local stack; `AUTH_MODE=none` for service UI smoke,
  authenticated identities for personal isolation and approval authorization.
- Two agents, separate private sessions for Alice and Bob, and a controlled HTTPS
  JSON endpoint returning disposable token sentinels. Never use production keys.
- Exercise light/dark themes and a narrow viewport.

## Steps

1. Open agent Credentials, Settings > My agent experience, a service virtual user,
   Settings > Organization, and a session's Credentials action. Confirm each names
   its owner/usage and shows loading, empty, populated, and error states cleanly.
2. Declare an HTTPS header/form binding. Confirm declaration accepts metadata
   only. Add its value in the masked form; save, fail a save, cancel, and change
   organization while entering a value. Confirm plaintext clears in every case
   and never appears in returned JSON, DOM text, console, screenshots, or caches.
3. Grant one agent access to an org credential. Confirm another agent cannot use
   it. Revoke the grant and verify the next send fails, including within one
   ongoing Bashkit call. Verify expired credentials and stale rotation revisions.
4. Ask web_fetch to capture several secret fields at virtual-user scope. Confirm
   the card appears before dispatch and shows source/method, exact request,
   acting identity, each response field/name, owner, destination, placement,
   expiry and whether it replaces a value. Only approve/decline are offered.
5. Approve once and retry the exact call. Confirm all fields save atomically and
   every known literal/JSON/form echo is absent from results, events, traces,
   files and session export. Repeat with missing/duplicate fields, oversize or
   interrupted bodies, and unavailable storage; confirm no partial capture.
6. Reuse Alice's personal binding in a new private session and another agent.
   Bob, a shared workspace, Playground, a wrong responder and a foreign org must
   not borrow it. Revoke from Settings and verify reconnect guidance before the
   next request. Generic always-allow, replayed consent and rotation while waiting
   must not authorize dispatch or restore the value.

## Expected Result

Scopes, owners and explicit grants are visible and consistent. The model handles
references only. Consent binds one exact request; revocation blocks the next
participating send. Document controlled-fixture evidence separately from a real
upstream account flow, which may require that account holder's provider consent.
