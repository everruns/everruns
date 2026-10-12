---
title: "Everruns 0.22.0: Reasoning modeled as ordered artifacts"
description: "Agents read several workspace files in one read_many_files call with bounded batches, and the global chat session endpoints are removed from the API."
date: 2026-08-25
kind: release
significance: notable
products: [platform, framework, cli]
release: "0.22.0"
---

## Highlights

- **Reasoning modeled as ordered artifacts**: Model reasoning is now captured as ordered artifacts with a published classification, so reasoning steps replay in the order they were produced and their kind is visible to consumers ([#3276](https://github.com/everruns/everruns/pull/3276)).
- **Bounded batch file reads**: The filesystem capability gained bounded batch reads, letting agents fetch several files in a single guarded call instead of one round-trip per file ([#3275](https://github.com/everruns/everruns/pull/3275)).

## In the changelog

- [Agents read several workspace files in one read_many_files call](/changelog/2026-08-24-read-many-files-tool/)
- [Global chat session endpoints removed from the Everruns API](/changelog/2026-08-25-chat-singleton-endpoints-removed/)
- [Messages API reports reasoning in order and the message phase](/changelog/2026-08-25-reasoning-ordered-message-phase/)

Before upgrading, read the breaking changes: [Global chat session endpoints removed from the Everruns API](/changelog/2026-08-25-chat-singleton-endpoints-removed/).

See the [GitHub release](https://github.com/everruns/everruns/releases/tag/v0.22.0) and the [full release notes](https://github.com/everruns/everruns/blob/main/CHANGELOG.md#0220---2026-08-25) for every pull request in this version.
