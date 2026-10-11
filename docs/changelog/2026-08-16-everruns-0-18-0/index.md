---
title: "Everruns 0.18.0: The 0.18 neutral-kernel boundary"
description: "Everruns 0.18.0 was released on 2026-08-16 with 5 highlights and 5 changelog entries. The release notes and any upgrade notes are linked here."
date: 2026-08-16
kind: release
significance: notable
products: [platform, framework, cli]
release: "0.18.0"
---

## Highlights

- **The 0.18 neutral-kernel boundary**: `everruns-core` is now a neutral execution kernel. Hosted platform records, sessions, agents and agent versions, harnesses, evals and observers, connectors, system email, feature flags, and OAuth/credential infrastructure, moved out to `everruns-platform` (and sibling crates), and the logic-free `everruns-runtime` compatibility crate was removed. REST/gRPC shapes and stored schema are unchanged; see the [0.18 migration guide](https://github.com/everruns/everruns/pull/3148) and the **Breaking** notes below ([#3160](https://github.com/everruns/everruns/pull/3160), [#3101](https://github.com/everruns/everruns/pull/3101)).
- **Engine-owned sessions and unified execution**: Engines now own the session lifecycle, and a single execution path serves both in-process and durable runs ([#3179](https://github.com/everruns/everruns/pull/3179), [#3173](https://github.com/everruns/everruns/pull/3173), [#3167](https://github.com/everruns/everruns/pull/3167)).
- **Multi-head workspaces**: Framework workspaces now support multiple heads ([#3166](https://github.com/everruns/everruns/pull/3166)).
- **Metered embedding spend**: Retrieval and knowledge-index embedding spend is now metered and ledgered against the organization ([#3149](https://github.com/everruns/everruns/pull/3149), [#3163](https://github.com/everruns/everruns/pull/3163)).
- **Chats as core functionality**: Chats is always present in navigation and search for every organization with no feature opt-in, and now supports chat pinning.

## In the changelog

- [Chats is on for every organization, with pinned chats](/changelog/2026-08-10-chats-for-every-organization/)
- [Session recordings split into Transcript, Timeline and Events](/changelog/2026-08-10-session-transcript-timeline-events/)
- [Knowledge index embedding spend is metered and ledgered](/changelog/2026-08-14-embedding-spend-metered/)
- [Framework workspaces gain multiple isolated Git heads](/changelog/2026-08-14-framework-multi-head-workspaces/)
- [everruns-core 0.18 becomes a neutral execution kernel](/changelog/2026-08-15-everruns-core-neutral-kernel/)

Before upgrading, read the breaking changes: [everruns-core 0.18 becomes a neutral execution kernel](/changelog/2026-08-15-everruns-core-neutral-kernel/).

See the [GitHub release](https://github.com/everruns/everruns/releases/tag/v0.18.0) and the [full release notes](https://github.com/everruns/everruns/blob/main/CHANGELOG.md#0180---2026-08-16) for every pull request in this version.
