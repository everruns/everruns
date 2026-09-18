---
type: Test Case
title: "TC001: Agent MCP Attachments - Add, authorize, invoke, and remove"
description: "Verify that the Agent detail MCP tab can add, authorize, invoke, and remove preset attachments for service and user identity modes without revoking grants during removal."
tags:
  - everruns
  - test-case
  - ui
  - agent-mcp-attachments
---
# TC001: Agent MCP Attachments - Add, authorize, invoke, and remove

## Description

Verify that the Agent detail MCP tab can add, authorize, invoke, and remove preset attachments for
service and user identity modes without revoking grants during removal.

## Preconditions

- The canonical local stack is running per the root `AGENTS.md` with authentication enabled.
- An active Agent and an active OAuth MCP server preset exist.
- An admin can manage MCP servers and service grants for the Agent.
- A second user can invoke the Agent and authorize their own connection.
- Both users use disposable OAuth accounts.

## Test Data

| Field | Value |
| --- | --- |
| Agent | Active Agent available to both users |
| Preset | Active OAuth MCP server preset |
| Service mode | `actsAs: service` |
| User mode | `actsAs: user` |

## Steps

1. As the admin, open the Agent detail page and select the **MCP** tab.
2. Select **Add MCP server**, search for the preset, select it, choose **Service identity**, and add
   it.
3. Select **Authorize**, complete OAuth with the service account, and return to the Agent MCP tab.
4. Start a session as each user and invoke one tool from the attachment.
5. Remove the attachment and confirm the warning about new sessions and retained grants.
6. Add the same preset again with **Service identity**.
7. Remove it, add it with **Invoking user**, and sign in as the second user.
8. Select **Connect**, complete OAuth with that user's account, and return to the Agent MCP tab.
9. Start a session as the second user and invoke one tool from the attachment.
10. Remove the attachment and confirm the warning.

## Expected Result

- Step 2 adds one explicit Agent attachment whose source and **Service identity** mode are visible.
- Step 3 returns to `/agents/{agentId}?tab=mcp` and shows the connected service account.
- Step 4 succeeds for both users through the same service account.
- Step 5 removes the attachment but does not revoke the service grant.
- Step 6 restores the connected state without another OAuth flow.
- Step 7 adds one explicit Agent attachment whose **Invoking user** mode is visible.
- Step 8 returns to the MCP tab and shows the second user's connected account.
- Step 9 succeeds through that user's account and does not use the service grant.
- Step 10 removes the attachment but does not revoke the user's grant.
