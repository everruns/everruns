---
title: Rooms keep shared files and join the Everruns MCP server
description: Every room gets a folder of notes and plans that agents edit in place, with every version kept, and MCP clients can catch up, post and edit files.
date: 2026-11-10
kind: feature
significance: notable
products: [platform, cloud]
release: "0.52.0"
surfaces: [api, cli, agent]
links:
  docs: [/features/rooms/]
---

A [room](/features/rooms/) now has a shared folder for what should outlast the chat: the plan, the
decisions, the data everyone keeps coming back to. A person writes the plan, the agent you mention
ticks an item off with its usual file tools at `/room`, and every version stays, with who made it.

```bash
everruns rooms files edit --room-id room_01h9... --path /plans/launch.md \
  --edits '[{"old_text":"- [ ] Draft","new_text":"- [x] Draft"}]'
everruns rooms files history room_01h9... --path /plans/launch.md
```

A write that started from an old version fails instead of overwriting someone's change, a moved
file keeps its history, and a deleted one can be restored.

Rooms are also part of the [Everruns MCP server](/features/mcp/): Claude, Cursor or your own
agent can take part with `room_read_unread`, `room_post`, `room_who_read` and the room file tools,
as a person or, for your product, as one of its guests.
