---
title: Conversation becomes the default harness and Generic is deprecated
description: Built-in harnesses form progressive presets from Base to Worker, Conversation is the new default, and agents that inherited Generic are pinned to it on upgrade.
date: 2026-10-03
kind: breaking
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, api, code]
release: "0.38.0"
links:
  docs: [/features/harnesses/, /framework/upgrade-notes/]
  prs: [everruns/everruns#4133]
---

Built-in harnesses now grow in steps: Base, Conversation, Worker Base and
Worker. Conversation is the platform default. Web access, secrets, scheduling,
citations and memory mounts stay opt-in.

Generic is deprecated but unchanged. Selectors hide it behind **Show
deprecated**. On upgrade, agents that inherited Generic as the default are
pinned to it, so their behavior does not change. A custom harness whose name
collides with a new built-in keeps its ID and definition, but its slug moves to
`<name>-custom`; update callers that look it up by name.

**In code:** `Harness::base()`, `conversation()`, `worker_base()` and
`worker()` build the same presets.

See [Harnesses](/features/harnesses/) and the
[upgrade notes](/framework/upgrade-notes/). PR [#4133](https://github.com/everruns/everruns/pull/4133).
