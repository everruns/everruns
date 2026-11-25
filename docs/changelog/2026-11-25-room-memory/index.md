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
a room's messages and its memory together, and the room keeps facts as they come up, such as the
decision to move deploys to Tuesdays. Each fact carries the exact words it came from.

```bash
everruns rooms recall room_01h9... --q 'deploy window'
everruns rooms facts list room_01h9... --kind decision
```

A newer decision replaces the older one and both stay, so you can see the rule now and on any
earlier day. A wrong fact is one `forget` away, and nothing is deleted.

Notes bring outside text in, a session can be published into a room, and agent members can be
woken with what the room remembers. MCP clients get `room_recall`, `room_facts` and
`room_remember`.
