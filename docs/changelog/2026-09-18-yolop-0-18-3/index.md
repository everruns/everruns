---
title: "Yolop 0.18.3 release"
description: "Yolop 0.18.3 was released on 2026-09-18. Release notes, upgrade notes and every change are linked here. See the full release notes on GitHub."
date: 2026-09-18
kind: release
significance: minor
products: [yolop]
release: "0.18.3"
---

Highlights of Yolop 0.18.3:

- Coordinator staffing: `yolop coordination spawn-workers` prints validated per-worker launch commands, and `yolop_spawn` routes agent versus session spawns.
- Setup can log out: leave model setup without saving, and drivers log out on exit.
- Settings and connections hot-reload; integration tests use isolated spawns.
- TUI adopts tuika 0.12 ToolShell and drops legacy shell paths.

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.18.3) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0183---2026-09-18) for every pull request in this version.
