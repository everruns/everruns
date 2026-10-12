---
title: "Yolop 0.17.3 release"
description: "Turn activity compacts into one summary row, yolop mcp covers show and disable, cargo install works again, and web_fetch reaches public URLs."
date: 2026-09-10
kind: release
significance: minor
products: [yolop]
release: "0.17.3"
---

## Highlights

- Fullscreen turns compact their activity by default. `--no-compact-work` keeps the expanded narration and tool rows for anyone who prefers the old projection.
- `yolop mcp` is a complete configuration surface: `mcp show <name>` inspects global, workspace, or effective scope, and `disable` is its own command instead of `enable --disable`.
- Fresh `cargo install yolop` works again. Every `everruns-*` requirement is pinned exactly, so a from-scratch resolve picks the set that was actually built and reviewed, and that set is now the everruns 0.20 facade release.
- Reasoning effort resolves from every source that knows about it, curated registry, then provider-advertised parameters, then the reasoning-required families, so a model the registry has never seen keeps its `/effort` control and repairs a mandated-reasoning rejection once on its own.
- The runtime stops doing work twice: a background task the foreground turn already reported no longer wakes a duplicate turn, and the environment context reports the shell's real network access instead of a stale no-network claim.
- `web_fetch` reaches public URLs again, and MCP restart/reconnect/reload land on the typed `mcp reload` command.

## In the changelog

- [Turn activity compacts into one summary row by default](/changelog/2026-09-03-yolop-compact-work-default/)

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.17.3) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0173---2026-09-10) for every pull request in this version.
