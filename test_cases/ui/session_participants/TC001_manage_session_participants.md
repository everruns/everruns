# TC001 Manage session participants

## Description

Verify that the **In this session** rail shows host and member participants, supports inviting a member agent, addressing a member agent for a single turn, and removing a member, with correct host/member and agent/user labeling and join/leave history.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- Two active agents exist: one to host the session and one to invite as a member
- A session hosted by the first agent is open and has at least one completed turn

## Test Data

| Field | Value |
| --- | --- |
| Host agent | Customer Support |
| Member agent | Billing Specialist |
| Addressed message | `Check the billing hold on this order` |

## Steps

1. Open a session hosted by the host agent and locate the **In this session** rail.
2. Verify the rail lists the host agent labeled **Host** / **Agent** and the current user labeled **User**.
3. Select **Invite agent** and choose the member agent (one not already active in the session).
4. Verify the member agent appears in the rail labeled **Member** / **Agent** and the host is unchanged.
5. In the composer, open the **Address** selector and confirm it defaults to **Session host (default)**.
6. Select the member agent, send the addressed message, and confirm the reply is attributed to the member agent.
7. Send a follow-up turn without changing the selector, and confirm the host answers (addressing is per turn).
8. Use **Remove from session** on the member agent.
9. Confirm the host cannot be removed through the ordinary participant action.

## Expected Result

- The rail lists all participants with **Host**/**Member** and **Agent**/**User** labels.
- Inviting a member agent adds it as **Member** / **Agent** without replacing the host or the host's harness.
- The **Address** selector routes a single turn to the selected member agent, and that turn's reply is attributed to the selected participant.
- The next unaddressed turn is answered by the host (addressing does not change the default responder).
- Removing a member marks it as having left; it remains visible in participant history and can no longer be addressed.
- The host cannot leave through the ordinary participant remove action.
