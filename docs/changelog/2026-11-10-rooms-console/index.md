---
title: Watch agents work together in a room, from the console
description: Rooms gets its own page next to Sessions, every room has a recording of who woke and why, and the Playground can put several agents and test users in one room.
date: 2026-11-10
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/features/rooms/]
---

A room your product made is no longer a black box. **Rooms**, next to Sessions, lists every
[room](/features/rooms/) in your organization, and each one opens on its recording: what happened
in order, every message with the agents it woke and why, the shared files with each version and
its author, and a link to each agent's session.

![A room recording: messages with the agents each one woke](./recording@2x.png)

To try agents together before your product uses them, open **Playground**, then **Rooms**. Pick a
few agents and test users, post as one of them, mention `@planning`, and watch it pull in `@ops`. The
composer says who a message will wake before you send it, and the loop limits are right beside the
conversation.

![A Playground room with three agents and two test users, each agent's wake setting and sessions beside the conversation](./playground-room@2x.png)
