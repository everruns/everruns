---
type: Test Case
title: "TC001: User MCP - Shared approval policy"
description: "Verify personal MCP add and enable honor the shared approval mode while management and identity restrictions remain enforced."
tags:
  - everruns
  - test-case
  - agents
  - user-mcp
---
# TC001: User MCP - Shared approval policy

## Description

Verify personal MCP add and enable honor the shared approval mode while management and identity restrictions remain enforced.

## Preconditions

- Canonical DB-backed local stack and a real LLM configured.
- One authenticated initiating person, a disposable agent, and an allowed catalog server.
- Record the agent, person, session and server IDs plus the starting event cursor for every run.
- Disable soft approval for this test so model-requested consent cannot mask the hard gate.

## Test Data

| Shared Tool Approval | Add and enable |
| --- | --- |
| Absent | Run without a hard approval request |
| Off | Run without a hard approval request |
| Normal | Wait for the shared approval request |
| Protective | Wait for the shared approval request |

## Steps

1. For each row, configure `user_mcp` with `manage` enabled and `use` disabled to avoid discovering remote tools during the test. Use a fresh session and unique server name.
2. Ask the agent to add the allowed catalog server under that name. Wait at most 90 seconds for completion or the shared approval request.
3. Read the person's server list. In enabled modes, verify no row was created before approval; reject the request and verify the list stays unchanged. Repeat and approve, then verify the row exists.
4. Disable the row through Settings/API. Ask the agent to enable it, with the same 90-second deadline. Verify enabled modes leave it disabled until approval; rejection leaves it disabled and approval enables it.
5. In absent/off modes, verify both operations complete and persist without any `approve_tool_call` request after the recorded cursor.
6. Set `manage` false and verify neither management tool is offered. Attempting a forged management call through the host must leave the person's list unchanged.
7. With custom URLs disabled, request a custom endpoint and verify refusal with no row created. Repeat an allowed operation as another person and verify the original person's list is unchanged.
8. Repeat add with `scope: "chat"`; check the session MCP list instead of the personal list and verify the same approval behavior.
9. Request a connection for the new server and verify OAuth consent still occurs separately; tool approval never supplies a credential.

## Expected Result

- Shared mode controls hard approvals for both add and enable, including calls added only for the chat.
- Rejected or pending calls cause no mutation. Absent/off permits autonomous management.
- Management authorization, initiating-user isolation, catalog/URL validation and OAuth consent remain independent boundaries.

## Source Contract

[User MCP servers](../../../integrations/user-mcp-servers.md) and
[Tool Approval](../../../execution/tool-approval.md).
