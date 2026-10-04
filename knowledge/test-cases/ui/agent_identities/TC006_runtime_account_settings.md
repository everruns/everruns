---
type: Test Case
title: "TC006: Virtual users - Runtime settings and account isolation"
description: "Verify that agent-facing settings and connections use the current organization virtual user."
tags:
  - everruns
  - test-case
  - ui
  - virtual-users
---
# TC006: Virtual users - Runtime settings and account isolation

## Description

Verify that agent-facing settings and connections use the current organization virtual user.

## Preconditions

An authenticated management user belongs to two organizations. Each has a default virtual user. The local stack is running with encryption configured.

## Test Data

| Field | Value |
|---|---|
| Runtime name | Runtime Alex |
| Locale | en-US |
| Timezone | America/Chicago |
| Service name | Test service |

## Steps

1. Open Settings → My agent experience, change the runtime name and defaults, and save.
2. Open Account and verify the management name remains unchanged.
3. On My agent experience, confirm the connections list, then open the same virtual user's Connections tab; verify they show the same grants. Visiting `/settings/connections` opens the same page.
4. Switch organizations and verify the second runtime profile and connections remain separate.
5. Create a service virtual user, assign it to an agent in Overview, and open its Connections tab.
6. Open Linked identities and Sessions; verify the console binding and owned chat are visible.
7. Return to Chats, pin and archive a chat, reload, and verify the state persists.
8. Send a message and verify the participant displays the runtime name while the console account still displays the management name.
9. Archive a test virtual user, restore it from Overview, and reload to verify its active state persists.

## Expected Result

The runtime profile persists without changing the management account. Connections and chats resolve through the current organization's runtime account. End-user credentials remain private; service account selection accepts only active service users in the agent's organization.
