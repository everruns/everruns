---
title: "Built-in harnesses reorganized as a tree by use case"
description: "Base, Conversation, Worker, Bashkit Worker and Sandbox Worker now form a tree of built-in harnesses, and existing Worker bindings move by sandbox policy."
date: 2026-10-10
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, agent]
links:
  docs: [/features/harnesses/#built-in-harnesses]
  prs: [everruns/everruns#4484]
---

The built-in harnesses now form a tree where each layer answers one question.
**Base** carries the system essentials. **Conversation** (the default) adds chat
features with no workspace, and **Worker** adds files, skills and delegation
with no compute. **Bashkit Worker** and the new **Sandbox Worker** split Worker
by where commands run, and each enforces that rule on agents and sessions.

Organizations are migrated once: agents bound to the old Worker move to Bashkit
Worker or Sandbox Worker based on their sandbox policy. Worker Base and Generic
stay as deprecated harnesses for existing bindings.

See [Built-in harnesses](/features/harnesses/#built-in-harnesses) and [#4484](https://github.com/everruns/everruns/pull/4484).
