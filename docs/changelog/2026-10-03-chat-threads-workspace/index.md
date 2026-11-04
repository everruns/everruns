---
title: Chat threads keep conversations and ongoing work beside Chat
description: Organizations can opt into Chat threads, a panel beside the permanent Chat for searching conversations, following background work and resolving threads.
date: 2026-10-03
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui]
release: "0.39.0"
links:
  docs: [/features/ui/]
  prs: [everruns/everruns#4146]
---

Organization owners and admins can turn on **Chat threads** in
**Settings > Features**. With it on, the sidebar holds only Chat, and a
**Threads** panel beside the permanent conversation lists your other
conversations and the work running in the background.

![Chat with the side panel open, listing Open and Resolved subchats](./chat-subchats-panel@2x.png)

Create a **New thread**, search conversations, and open work from its card,
where you can answer pending input or stop it. **Resolve thread** puts a
conversation away without losing history, and **Reopen thread** continues it.
The panel becomes a drawer on mobile. Turning the feature off restores the
previous Chat and keeps conversations and their URLs.

See [Chat and Threads](/features/ui/#chat-and-threads). PR [#4146](https://github.com/everruns/everruns/pull/4146).
