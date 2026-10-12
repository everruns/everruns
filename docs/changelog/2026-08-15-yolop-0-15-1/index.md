---
title: "Yolop 0.15.1 release"
description: "The Meta Model API joins the supported providers, and terminal text selection survives Ctrl and Ctrl+C, on the tuika 0.8.0 toolkit."
date: 2026-08-15
kind: release
significance: minor
products: [yolop]
release: "0.15.1"
---

## Highlights

- Model selection is honest about what is reachable: ACP exposes only connected models, duplicate model names resolve uniquely, and provider model catalogs are listable over ACP.
- The Meta Model API joins the supported providers.
- Long-running turns behave better under stress, stall recovery shares the watchdog's budget, blocked completion status stays hidden, and workspace grep scans stream instead of buffering.
- Terminal selection survives Ctrl and Ctrl+C, on tuika 0.8.0 with matching satellite crates.
- Skill management (`search_skills` / `install_skill` / `delete_skill`) is now actually enabled in the coding harness, matching what the docs already promised.

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.15.1) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0151---2026-08-15) for every pull request in this version.
