---
type: Specification
title: "Session Participants"
description: "Session participants (host/member agents and users, addressed-turn routing, invite-mode handoff)."
tags:
  - everruns
  - runtime-resources
---
# Session Participants

## Abstract

A session is a shared space that more than one actor can occupy. **Session
participants** record which agents and users are attached to a session, in what
role, and for which interval. Participants make multi-actor sessions
first-class: a host agent that supplies the execution environment, invited guest
agents that can be addressed for individual turns, and the users watching or
driving the conversation.

This spec captures the durable model and its invariants. Field-level shapes,
enum variants, and SQL live in code, see `crates/core/src/session.rs`
(`SessionParticipant`, `SessionParticipantKind`, `SessionParticipantRole`), the
command layer in `crates/server/src/domains/sessions/commands.rs`, and migrations
`095_session_participants.sql` / `098_session_participant_user_identity.sql` /
`112_session_participant_display_name.sql`.

## Model

Each participant row binds a principal to a session:

- **kind**: `agent` or `user`. Agent participants carry an `agent_id` (and,
  when known, the immutable `agent_version_id`); user participants do not.
- **role**: `host` or `member`. The host anchors the session; members are
  ordinary participants (invited guest agents, additional users).
- **principal_id**: the principal that joined. This is the provenance anchor:
  turns, resources, and audit attribute back to the participant's principal
  rather than to `system`.
- **display name**: the human-readable identity captured with the participant.
  Signed-in users use the authoritative user profile and existing participant
  rows track profile renames. External-channel participants retain their
  explicit actor name. Missing names fall back to the actor kind rather than
  implying that all unnamed people share the same identity.
- **joined_at / left_at**: the participation interval. `left_at = null` means
  the participant is still active; a set `left_at` marks a past membership that
  is retained for history.

Participant ids use the `part_` prefix (see `knowledge/foundations/id-schema.md`).

## Invariants

- **One active host.** At most one active agent participant may hold the `host`
  role (`knowledge/foundations/concepts.md`). The host is established when the session is created
  and cannot leave through the ordinary leave path.
- **Host supplies the harness.** The session runs on the host agent's harness
  (the agent-owns-harness foundation from P1). Invited members overlay behavior
  only, see routing below, and do not replace the host's execution
  environment.
- **Membership history is append-only in spirit.** Leaving sets `left_at`; rows
  are not deleted, so provenance and the join/leave timeline stay reconstructable.
- **User membership follows activity.** Before an authenticated user's message
  is emitted, that user is ensured as an active member participant
  (`ensure_active_user_session_participant`). An existing active row is reused;
  if the user previously left, the message creates a new row for the new
  participation interval. The message is attributed to that active row.

## API

Participants are managed under a session
(`crates/server/src/api/sessions.rs`):

- `GET /v1/sessions/{session_id}/participants`, full participant history
  (active and left), ordered by `joined_at`. Policy `SESSION_VIEW`.
- `POST /v1/sessions/{session_id}/participants`, add a **member**. Body carries
  `kind` and, for agents, `agent_id`; `role` defaults to `member` and an
  explicit `host` is rejected (the host is owned by session creation). Policy
  `SESSION_MANAGE`.
- `DELETE /v1/sessions/{session_id}/participants/{participant_id}`, leave: sets
  `left_at`. An active host cannot leave through this endpoint (`409`). Policy
  `SESSION_MANAGE`.

There is no participant join/leave event on the SSE stream; consumers that want a
join/leave timeline derive it from `joined_at` / `left_at`.

## Turn routing

By default a user turn is answered by the session **host**. A turn may instead
be **addressed** to a specific participant by setting
`addressed_participant_id` on the create-message request
(`POST /v1/sessions/{session_id}/messages`). Resolution
(`resolve_responder_agent_id` in
`crates/server/src/domains/messages/commands.rs`):

- omitted → the session's default (host) agent responds;
- the addressed participant must exist, still be active (`409` if it already
  left), and be an agent (`400` otherwise);
- the turn is then routed to that participant's `agent_id`, and the reply is
  attributed to that participant for provenance.

This makes a single session support several addressable agents while keeping the
host as the default responder.

## Invite-mode handoff

Guest agents most commonly join through **invite-mode handoff**: an agent with
the `agent_handoff` capability calls `spawn_agent` with `mode = invite`, which
adds the configured target agent as a **member** participant in the current
session instead of creating a child session. Addressed turns then route to that
member. The invited agent contributes only a behavioral overlay during addressed
turns (prompt, capabilities, model defaults, client tools); its own harness stays
reserved for child-session modes. See `knowledge/runtime-resources/agent-handoff.md` for the full
handoff contract and the `invite` vs `background`/`foreground` mode split.

## Surfacing

The session view surfaces participants as an "in this session" rail (roles and
kinds as badges, with invite/leave controls) and derives join/leave system lines
in the transcript from `joined_at` / `left_at`. Because no participant SSE event
exists, this surface is a read/derive over the participants API and does not
change the session or message contracts.
