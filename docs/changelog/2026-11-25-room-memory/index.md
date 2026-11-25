---
title: "Rooms remember what they decided"
description: "Rooms keep facts and notes next to their messages: recall what was said, see the current decision and the one it replaced, and publish a session into a room."
date: "2026-11-25"
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [api, cli, agent]
links:
  docs: [/features/rooms/]
---

"Did we already decide this?" now has an answer in every [room](/features/rooms/). Recall searches
a room's messages and its memory together, and the room keeps facts as they come up: the decision
to move deploys to Tuesdays, the rule about who approves a release, the deadline for the importer.
Each fact carries the exact words it came from, and the room records them on its own unless you
turn that off.

```bash
everruns rooms recall room_01h9... --q 'deploy window'
everruns rooms facts list room_01h9... --kind decision
```

When the team changes its mind, the newer decision replaces the older one and both stay, so you
can see what the rule is now and what it was on any earlier day. A wrong fact is one `forget` away,
and nothing is ever deleted.

Notes bring outside text into the same memory, a finished session can be published into a room
(the conversation only, with secrets scrubbed), and agent members can be woken with what the room
remembers about the message in front of them. MCP clients get `room_recall`, `room_facts` and
`room_remember`.
