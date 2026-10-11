---
title: "Everruns 0.21.0 release"
description: "Everruns 0.21.0 was released on 2026-08-22 with 3 highlights and 5 changelog entries. The release notes and any upgrade notes are linked here."
date: 2026-08-22
kind: release
significance: notable
products: [platform, framework, cli]
release: "0.21.0"
---

## Highlights

- **Chat with harnesses, archive chats, and generated run summaries**: Sessions gained interactive chats against harnesses, the ability to archive chats and toggle showing all of them, and automatically generated run summaries, with count badges on session detail tabs ([#3262](https://github.com/everruns/everruns/pull/3262), [#3265](https://github.com/everruns/everruns/pull/3265), [#3260](https://github.com/everruns/everruns/pull/3260), [#3266](https://github.com/everruns/everruns/pull/3266)).
- **Connection-level custom provider headers**: Providers can now carry custom headers at the connection level, with cache diagnostics to see how they resolve; header values are redacted in diagnostics ([#3232](https://github.com/everruns/everruns/pull/3232), [#3240](https://github.com/everruns/everruns/pull/3240)).
- **Leaner supply chain and smaller binaries**: Owning 114 small third-party contracts in-tree removed those dependencies from the build, and stripping plus CLI dispatch boxing shrank shipped binaries by ~22% ([#3245](https://github.com/everruns/everruns/pull/3245), [#3227](https://github.com/everruns/everruns/pull/3227)).

## In the changelog

- [Everruns CLI, server and worker binaries are about 20% smaller](/changelog/2026-08-19-smaller-release-binaries/)
- [Start a chat with any harness, not only an agent](/changelog/2026-08-21-chat-with-harnesses/)
- [Provider connections carry custom headers and cache diagnostics](/changelog/2026-08-21-provider-connection-custom-headers/)
- [Archive finished chats and show them again on demand](/changelog/2026-08-22-archive-chats/)
- [Session pages show a generated run summary and tab counts](/changelog/2026-08-22-session-run-summary-tab-counts/)

See the [GitHub release](https://github.com/everruns/everruns/releases/tag/v0.21.0) and the [full release notes](https://github.com/everruns/everruns/blob/main/CHANGELOG.md#0210---2026-08-22) for every pull request in this version.
