---
title: Work Chat addresses are not the channel id
description: A published Work Chat link uses its own hostname. The address is assigned once, stays put when settings change, and no longer matches the channel id.
date: 2026-10-30
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/features/channels/#work-chat]
---

A published Work Chat page shows an address you can send to the people who
should chat. That address used the channel id as its hostname, so the public
link and the channel id were the same.

Each Work Chat now gets its own hostname when the channel is created. The
address still stays the same when you change settings. A link that used the
channel id no longer opens the chat. Copy the new address from Settings or
Connect.

See [Work Chat](/features/channels/#work-chat).
