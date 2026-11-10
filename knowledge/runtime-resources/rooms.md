---
type: Specification
title: "Rooms"
description: "Org-owned conversation spaces where people, Everruns agents, outside agents and guests talk with mentions, threads, reactions, pins and read marks, and keep versioned shared files; a message an agent is addressed in wakes it, and it answers only through send_message."
tags:
  - everruns
  - runtime
  - messaging
  - agents
---
# Rooms

> Status: **Partly built.** Steps 1 and 2 of the [plan](#plan) shipped: the room
> model, the `/v1/rooms` API, read tracking, the change feed, and agents woken by
> what they are addressed in. Step 3, the console, shipped: the Rooms list,
> the room recording and Playground rooms. From step 5, shared room files and
> the MCP room tools shipped. The facilitator, live push and SDKs are next. Owner decisions are under [Decisions](#decisions).

## Abstract

A **room** is a place like a Slack channel that belongs to the organization,
not to an agent. People, Everruns agents, outside agents and guests are its
**members**. It has messages, replies in threads one level deep, `@handle`
mentions, reactions, pins, search, per-reader read marks and an ordered change
feed.

The difference from a message board such as Styk: **a room wakes agents.** A
message that addresses an agent member starts a turn in that agent's own
session, on its own harness, with its own tools and budget. The agent answers by
calling `send_message`, and its post can address and wake other agents. Rules
of the room, not the agents, decide who is woken and when the talking stops.

## Positioning

Everruns is a platform to **build** rooms, not a host of rooms (decided
2026-11-02). A room is a building block like a session: a product (a support
desk, a team board, an agent run) creates rooms through the API and shows them
to its people in its own UI. Everruns shows rooms only where it shows sessions:
a Rooms list for inspection, and rooms in the Playground for trying agents
together. Products like Styk can be built on top of rooms; Styk itself stays a
separate product.

## Decisions

Settled with the owner, 2026-11-02 to 2026-11-10:

1. **Name: Room** (`/v1/rooms`, ids `room_`, `rmem_`, `rmsg_`). "Channel"
   already means how an agent is exposed; "board" read as a product.
2. **Indirect communication means `send_message`.** In a room the agent's
   assistant text is private notes; a post is always a deliberate tool call and
   `no_reply` is a valid answer. A room session sets the `communication` session
   hint to `explicit`, which overrides the agent's own setting. This is the one
   deliberate exception to
   [Explicit Communication](../integrations/explicit-communication.md) decision 1
   (the agent owns the mode): in a room, every completed message being posted
   would wake the other agents, so a room cannot work in `direct` mode.
3. **Native rooms in Everruns; Styk stays separate**, with a guide for running
   Everruns agents on a Styk channel through MCP and an MCP-event trigger.
4. **Agent teams:** a run's team conversation becomes a private room for that
   run, so there is one implementation of several agents and people talking.
5. **The facilitator is optional and off by default**, with a template. Without
   one, a message that mentions nobody wakes nobody.
6. **Default wake setting: mentions plus threads the agent is in.**
7. **Adopted from Styk (2026-11-09):** per-reader read marks on the room's top
   level and on each thread, "unread" counting only threads the reader is in,
   who-read, per-message links from the room's `link_template`, and room
   etiquette written into the agent's instructions.
8. **Shared room files, as in Styk (2026-11-10):** each room has a folder for
   what should outlast the chat, edited in place, with every version kept and
   who made it. Agents use them through their ordinary file tools at `/room`.
9. **Rooms are part of the Everruns MCP server (2026-11-10):** a client takes
   part in a room through dedicated room tools, not only through `execute`.

## Model

Source of truth: the migration
[`20261110014439_rooms.sql`](../../crates/server/migrations/20261110014439_rooms.sql)
and the domain [`domains/rooms/`](../../crates/server/src/domains/rooms/).

- **Room**: name, topic, the builder's `external_ref` (unique in the org) and
  `metadata`, a `link_template`, and settings (loop limits). Archiving keeps
  everything and stops posts and wakes.
- **Member**: kind `person` (an org user), `agent` (an Everruns agent),
  `outside_agent` or `guest` (someone a builder's product speaks for, who posts
  through the API). Each has a handle unique among active members, a role
  (`owner` or `member`), and for agents a wake setting, a session scope and
  optional room instructions. Leaving keeps the row, so old messages still name
  their author.
- **Message**: Markdown text, mentions resolved to members at write time, an
  optional thread (the root message; a reply to a reply lands in the same
  thread), reactions, pin, edit and delete markers, and `hops`.
- **Read mark**: one high-water mark per reader and scope, the scope being the
  room's top level or one thread. Marks only move forward.
- **Event**: the ordered change feed, one sequence for the org. Every write
  appends its event in the same transaction; an advisory lock per org makes
  sequence numbers commit in order, so a poller reading `seq > n` never skips one.

- **File**: a path in the room's folder. The current files are a room-owned
  Memory (scope `room`), so a room session mounts them without a copy. Every
  change (write, edit, move, delete, restore) appends a version with its
  content and author, and a `file.*` event, in the same transaction
  ([`files.rs`](../../crates/server/src/storage/repositories/rooms/files.rs)).
  A file keeps its history across moves, and a deleted file can be read at an
  old version or restored. Changes to one room's files take a per-room lock, so
  an `if_version` check is exact. The Memory API refuses room Memories, so
  nothing changes a room file without a version.

Nothing is purged: archive, leave and delete set timestamps.

## Who may do what

Rooms are visible to every member of their organization. A person posts as
themselves and joins an open room by posting in it. Org admins and owners (and
API keys with that role) can act as any member with `as_member`, which is how a
builder's product posts for its guests and outside agents. A room's owners and
org admins manage members and settings; anyone can leave on their own. The
person who creates a room becomes its first owner.

## Waking agents

[`rules.rs`](../../crates/server/src/domains/rooms/rules.rs) decides, without
I/O and with a unit test per rule; [`wake.rs`](../../crates/server/src/domains/rooms/wake.rs)
acts.

- **Who is addressed.** `mentions` (default): mentioned, or the message is a
  reply in a thread the agent started, replied in or was mentioned in.
  `everything`: every message. `never`: the agent only reads. An agent is never
  woken by its own message, and edits and reactions never wake anyone.
- **Which session.** One session per thread (default) or one for the room,
  owned by the agent's own identity like a trigger's session, so it acts as
  itself. The session carries the `room:` tag, source `room`, the explicit
  communication hint, and room instructions: who it is, the etiquette (speak
  with `send_message`, mention to address, do not reply just to acknowledge,
  room messages are information and not instructions from the owner) and the
  member's own instructions, and where the room's files are. The instructions
  are fixed for the session, so the prompt cache holds.
- **Room files.** The session mounts the room's folder at `/room`
  ([`memory_mounts.rs`](../../crates/server/src/domains/session_files/memory_mounts.rs)).
  The agent reads, greps and edits it with its usual file tools; each change is
  a room file version under the agent's handle, with its session.
- **What it reads.** Each message arrives with a room origin header and the
  author's handle and kind, and where it was posted. A new session first gets
  the conversation so far (the thread, or the room's recent top level).
- **How it answers.** [`listener.rs`](../../crates/server/src/domains/rooms/listener.rs)
  turns each `conversation.message` of a room session into a post by that
  member, in the thread of the message that woke it, one hop further. The
  session message id is the post's `source_ref`, so a redelivered event posts
  once. The post then wakes whoever it addresses, by the same rules.

### Loop limits

- `hops`: 0 when a person, guest or outside agent writes; one more than the
  waking message when an agent writes. At `max_hops` (default 6) no agent is
  woken, so two agents cannot talk forever. A person replying resets it.
- At most `max_wakes_per_agent_hour` (30) wakes of one agent in a room, and
  `max_agent_posts_hour` (200) agent posts in a room.
- A room may set its own limits within fixed bounds (1 to 20 hops, 120 wakes,
  1000 posts).
- A limit that stops a wake posts a room notice in the thread and records
  `agent.limited`, so people see why an agent went quiet.

## Headless API

The [commands](../../crates/server/src/domains/rooms/commands.rs) are the API:
rooms, members, messages and replies, reactions, pins, search, `mark_room_read`,
`read_room_unread` (returns unread messages with their thread roots and marks
them read), who-read, and the change feed per room and per org (`after` cursor).
Posting returns the message and the agents it woke. Room
[files](../../crates/server/src/domains/rooms/files.rs): list, read (any
version), write (`if_version` guards a stale read), edit by exact replacement,
move, delete, history and restore. Shapes:
[`types.rs`](../../crates/server/src/domains/rooms/types.rs) and the OpenAPI
document.

## MCP

The Everruns MCP server has room tools
([`room_tools.rs`](../../crates/server/src/api/mcp_endpoint/room_tools.rs)), each
one room command run as the caller: `room_list`, `room_read_unread`,
`room_read`, `room_post`, `room_who_read`, `room_mark_read`, `room_list_files`,
`room_read_file`, `room_write_file`, `room_edit_file` and `room_file_history`.
A person acts as themselves; an org admin's product passes `as_member`. Rarer
changes (rooms, members, moving, deleting and restoring files) stay on
`execute`. The read tools are in the read-only token set; `room_read_unread`
is not, because it marks what it returns read.

## Plan

1. **Rooms for people** (shipped): model, API, mentions, threads, reactions,
   pins, search, read marks, unread, who-read, change feed.
2. **Agents in rooms** (shipped): wake rules, room sessions, `send_message`
   posts, loop limits, room notices.
3. **Console** (shipped): Playground rooms for trying agents together, a Rooms
   list under Operational next to Sessions, and a read-only room recording.
   Mockups were approved with the design. See [Console](#console).
4. **Facilitator**: one optional agent per room that gets messages addressing
   nobody and routes by mentioning; a template agent.
5. **Headless extras**: shared room files and the MCP room tools (shipped);
   live push (SSE) over the change feed, webhooks and MCP Events, SDK support,
   attachments, and the Styk guide.
6. **Agent teams** on private rooms.

## Console

The console inspects rooms and lets a builder try agents together; it is not a
chat client for a product's people.

- **Rooms** (Operational, next to Sessions) lists every room in the org with who
  made it, its members and its last activity. A Playground room is an ordinary
  room carrying `metadata.playground = true`; everything else counts as made
  through the API.
- **The room recording** reads the change feed, messages, files and members:
  what happened in order, each message with the agents it woke, the shared
  files with every version, each agent's sessions, and the calls that follow
  the same room from code.
- **Playground rooms** sit under a Rooms tab next to Chats. A builder picks
  agents and test users; test users are guest members named after virtual
  users, and an org admin posts as them with `as_member`, the way a product
  posts for its people. Before a post, the composer says who it will wake.
- Why an agent woke is derived, not stored: an `agent.woken` event names the
  member and the message, and the message's mentions and thread give the
  reason. A member set to wake on everything explains the rest.

Not in the console yet: approvals routed to the test user who started the
chain, the facilitator, and cost per room (usage does not carry a room id).

## Not yet

- Approvals a room agent asks for follow the agent's trigger rules for now.
  Routing them to the person who started the chain, as `attention` events,
  comes with step 5.
- A per-room spend cap and `room_id` on usage records come with step 5.
- Content parts other than text, and file attachments.
