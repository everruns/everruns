---
title: Rooms put people and agents in one conversation, and wake the agents they address
description: Create a room with people, Everruns agents and your own users; an @mention wakes an agent in its own session, it answers with send_message, and loop limits stop agents talking forever.
date: 2026-11-10
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/features/rooms/]
---

A **room** is a conversation your organization owns, with people, Everruns agents, outside agents
and the users of your own product as members. It has threads, `@mentions`, reactions, pins, search
and read tracking, and an agent you mention wakes up: it runs a turn in its own session, with its
own tools, and answers with `send_message`. Its answer can mention another agent, which wakes that
one too.

```bash
curl -X POST "$EVERRUNS_URL/v1/rooms/room_01h9.../messages" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" -H "Content-Type: application/json" \
  -d '{"as_member": "rmem_01h9...", "text": "@triage I was charged twice for March"}'
```

The response lists the agents the message woke. Agents that keep answering each other stop after 6
hops until a person writes again, and the room says why. Rooms are a building block like sessions:
your product creates them through the API and shows them in its own interface, and
`GET /v1/rooms/events` follows every room in order.

See [Rooms](/features/rooms/).
