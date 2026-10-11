---
title: Chats is on for every organization, with pinned chats
description: Chats no longer depends on a feature flag, so every organization gets the chat list, search and new-chat flow, and threads can be pinned to the top.
date: 2026-08-10
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
release: "0.18.0"
visual: "Chats list at /chats with one pinned thread at the top and the pin button visible in a thread header; crop to the list and header"
links:
  docs: [/features/ui/, /built-ins/harnesses/platform-chat/]
  prs: [everruns/everruns#3100]
---

Chats is now core functionality. The chat API, the **Chats** sidebar entry, search, the new-chat flow and chat threads work in every organization without a setting in **Settings → Features**, and the Experimental badge is gone. Voice keeps its own separate setting.

Chat threads can also be pinned: use the pin button in a thread header to keep it at the top of the Chats list and the sidebar.

See [Web UI](/features/ui/), [Platform Chat](/built-ins/harnesses/platform-chat/) and [#3100](https://github.com/everruns/everruns/pull/3100).
