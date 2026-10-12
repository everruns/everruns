---
title: Start a chat with any harness, not only an agent
description: The New chat picker lists active harnesses, including Generic, Base and custom ones, next to agents, so a chat no longer needs a placeholder agent.
date: 2026-08-21
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui]
release: "0.21.0"
visual: "New chat picker at /chats/new showing the Harnesses and Agents groups; crop to the picker"
links:
  docs: [/features/harnesses/, /built-ins/harnesses/generic/]
  prs: [everruns/everruns#3262]
---

**New chat** now shows two labelled groups, **Harnesses** and **Agents**. Pick any active harness, such as Generic, Base or one of your custom harnesses, to start a chat bound directly to it. Before, the picker listed agents plus a single hard-coded Platform Chat option, so chatting with a harness meant creating an otherwise empty agent.

The thread names the harness as its counterpart and appears in the Chats list like any other chat.

See [Harnesses](/features/harnesses/), [Generic harness](/built-ins/harnesses/generic/) and [#3262](https://github.com/everruns/everruns/pull/3262).
