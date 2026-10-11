---
title: Playground for testing agents, and Chat as one permanent conversation
description: Agent testing moves to a shared Playground where members pick the agent and the user it runs as, while Chat becomes your permanent Platform Chat conversation.
date: 2026-10-03
kind: feature
significance: major
products: [platform, cloud]
surfaces: [ui]
release: "0.38.0"
visual: "Playground library with grouped conversation rows and the New Playground chat setup with an agent selected; route /playground; crop to the list and setup panel."
links:
  docs: [/features/ui/, /built-ins/harnesses/platform-chat/]
  prs: [everruns/everruns#4060, everruns/everruns#4076, everruns/everruns#4121, everruns/everruns#4129]
---

**Playground** is where an organization tests its agents. Members start a
conversation as their linked virtual user; owners and admins can pick another
active end user. Conversations are shared across the organization, can be
searched, filtered by agent or user, and archived, and each links to its
session. On an agent page, **Test in Playground** opens setup with that agent
selected.

**Chat** now always uses the managed Platform Chat agent. The sidebar opens your
permanent conversation, **New chat** starts a side conversation, and
**All chats** lists only your personal conversations. Platform Chat runs on
the shell-based implementation for every organization.

See [Chat and Threads](/features/ui/#chat-and-threads) and
[Platform Chat](/built-ins/harnesses/platform-chat/). PRs [#4060](https://github.com/everruns/everruns/pull/4060),
[#4076](https://github.com/everruns/everruns/pull/4076), [#4121](https://github.com/everruns/everruns/pull/4121), [#4129](https://github.com/everruns/everruns/pull/4129).
