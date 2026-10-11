---
title: Every Everruns user starts in a pinned Platform Chat thread
description: New users now land in a precreated, pinned Platform Chat thread instead of an empty Chats list, and Platform Chat gains more built-in capabilities.
date: 2026-09-11
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui]
release: "0.25.0"
links:
  docs: [/built-ins/harnesses/platform-chat/]
  prs: [everruns/everruns#3508, everruns/everruns#3507]
visual: "Chats sidebar for a new user showing the pinned Platform Chat thread open (/chats); crop to sidebar and the chat welcome"
---

A new user used to land on **Chats** with an empty list and a counterpart picker.
The app now creates a thread on the built-in Platform Chat harness on first entry,
pins it, and ends onboarding in it. An existing thread is reused and an archived
one is not recreated.

Platform Chat also runs on a wider capability set, adding current time, message
timestamps, a to-do list, prompt caching, tool call repair and parallel tool
calls. Existing organizations pick these up without a migration.

See [Platform Chat](/built-ins/harnesses/platform-chat/). [#3508](https://github.com/everruns/everruns/pull/3508), [#3507](https://github.com/everruns/everruns/pull/3507)
