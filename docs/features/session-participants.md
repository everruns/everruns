---
title: Session Participants
description: Invite agents into a shared session, address a specific agent for a turn, and understand host, member, user, join, and leave behavior.
---

A session can include more than one agent and more than one user. Session participants record who is present, whether they are the host or a member, and when they joined or left.

## Host and member roles

An agent-backed session has one active host agent. The host supplies the session's harness and answers user turns by default.

Other agents and users join as members:

- an **agent member** can be addressed for an individual turn;
- a **user member** records a person participating in the session, but cannot be addressed as an agent responder; and
- a member that leaves remains in participant history with a leave time.

Inviting a member agent does not replace the host or the host's harness. The invited agent contributes its behavior, model defaults, capabilities, and client tools only when a turn is addressed to it.

## Invite an agent in the UI

The session view shows multi-party membership in the **In this session** rail on desktop. It labels each participant as **Host** or **Member** and as **Agent** or **User**.

To add an agent:

1. Open **In this session**.
2. Select **Invite agent**.
3. Choose an agent that is not already active in the session.

The agent joins as a member. The rail also shows participants who have left. Use the remove action on an active member to make it leave; the host cannot leave through the ordinary participant action.

Agents can perform the same operation with invite-mode handoff. An agent with the `agent_handoff` capability calls `spawn_agent` with `mode = invite`. Unlike foreground or background handoff, invite mode joins the target agent to the current session instead of creating a child session.

## Address an agent for one turn

When at least one active member agent is available, the composer shows an **Address** selector:

- **Session host (default)** sends the turn to the host agent.
- Selecting a member agent sends that turn to the selected participant.

Addressing is per turn. It does not change the session's host or default responder. A participant must still be active and must be an agent; requests that address a user or a participant who already left are rejected.

## API

Participant membership is managed below a session:

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/sessions/{session_id}/participants` | List active and past participants in join order |
| `POST` | `/v1/sessions/{session_id}/participants` | Add an agent or user member |
| `DELETE` | `/v1/sessions/{session_id}/participants/{participant_id}` | Mark a member as having left |

To address an agent, set `addressed_participant_id` when creating a message with `POST /v1/sessions/{session_id}/messages`. Omit it to use the host.

The participant list is the source of truth for join and leave history. Join and leave do not emit dedicated participant events on the session SSE stream. See the [API reference](/api/) for current schemas and authorization requirements.

## See also

- [Agent triggers](/features/agent-triggers/), proactive runs start sessions with the trigger's agent as host.
- [Sub Agents capability](/capabilities/sub-agents/), foreground, background, and invite handoff modes.
- [Core concepts](/explanation/concepts/), sessions, turns, and agents.
