---
title: Archive finished chats and show them again on demand
description: Archiving hides a chat thread from the Chats list without deleting it, and a Show archived filter and the archive API bring it back when needed.
date: 2026-08-22
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
release: "0.21.0"
visual: "Chats list at /chats with the Show archived filter on and one archived thread; crop to the filter and rows"
links:
  docs: [/features/ui/]
  prs: [everruns/everruns#3265]
---

Archive a chat when you are done with it. The session and transcript stay intact and reachable by URL; the thread just leaves the list.

**In the UI:** use the archive control on a row in **Chats**, and the **Show archived** filter to list archived threads. The sidebar's **All chats** link stays visible even when no threads are listed.

**In the API:** `PUT /v1/sessions/{id}/archive` archives and `DELETE /v1/sessions/{id}/archive` restores. Both are idempotent. Session lists hide archived sessions unless you pass `include_archived=true`, which widens facet counts too.

See [Web UI](/features/ui/) and [#3265](https://github.com/everruns/everruns/pull/3265).
