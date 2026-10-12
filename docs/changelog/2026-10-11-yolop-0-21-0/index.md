---
title: "Yolop 0.21.0 release"
description: "Code search works without ripgrep installed, coordinators run threads as separate processes, and new setups default to the latest model on each provider."
date: 2026-10-11
kind: release
significance: notable
products: [yolop]
release: "0.21.0"
---

## Highlights

- Treat everruns coordination threads as yolop processes and close the thread follow-ups.
- Seed connector defaults from the upstream catalog and register E2B tools.
- Default to the latest model on every provider.
- Search with the built-in rg fallback on machines without ripgrep, and ask structured questions across hosts.

## In the changelog

- [Code search works on machines without ripgrep](/changelog/2026-10-10-yolop-built-in-rg-search/)
- [Coordinators run threads as separate processes](/changelog/2026-10-10-yolop-coordination-threads/)
- [New setups default to the latest model on every provider](/changelog/2026-10-10-yolop-latest-default-models/)

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.21.0) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0210---2026-10-11) for every pull request in this version.
