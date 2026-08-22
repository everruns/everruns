# TC001 Manage session participants

## Description

Verify that the session participant rail identifies the host and members, supports inviting and addressing an agent member, attributes its reply, and renders a leave system line.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- Two active agents are available: a host agent and a guest agent with visibly different names
- A session hosted by the host agent is open

## Test Data

| Field | Value |
| --- | --- |
| Host agent | `Participant Host` |
| Guest agent | `Participant Specialist` |
| Current user display name | The name shown in the signed-in profile; `Anonymous` in `AUTH_MODE=none` |
| Addressed message | `Reply with exactly: specialist reply` |

## Steps

1. Open the host agent's session and locate the **In this session** participant rail.
2. Verify the host agent appears with the host role and the current user appears among the members
   under the current profile display name, both in the participant rail and its transcript join marker.
3. Use the invite control to add `Participant Specialist`.
4. Verify the guest appears as an active agent member without replacing the host.
5. Address `Participant Specialist` and send `Reply with exactly: specialist reply`.
6. Wait for the response and verify the reply is attributed to `Participant Specialist`, not the host.
7. Use the leave/remove control for `Participant Specialist`.
8. Verify the participant rail no longer shows the guest as active and the transcript renders a system line recording that it left.
9. Use the leave/remove control for the current user.
10. Verify the current user moves to **Left**, then send `I am back`.
11. Verify the current user returns to the active members and the new message appears after a fresh join marker.

## Expected Result

- The participant rail distinguishes the host, user members, and invited agent members.
- The current user's persisted participant and transcript marker use the same profile display name;
  neither renders the generic label `Participant`.
- Inviting an agent adds it as a member while preserving the original host.
- An addressed turn routes to the selected active agent and attributes its reply to that participant.
- Removing the guest retains membership history and renders a leave system line in the transcript.
- A user who sends after leaving automatically rejoins with a new participation interval; the
  participant rail and transcript show the renewed active membership without a page reload.
