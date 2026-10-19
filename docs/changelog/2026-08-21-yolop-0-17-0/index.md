---
title: "Yolop 0.17.0 release"
description: "Config and data directories can be relocated, model switching uses a curated list you manage, and compiled extensions install from prebuilt binaries."
date: 2026-08-21
kind: release
significance: notable
products: [yolop]
release: "0.17.0"
---

## Highlights

- Compiled extensions install end to end. A package may declare `capabilityServer.binaries`, and `yolop extensions install` fetches the prebuilt server for the host's target triple, verifying its `.sha256` before writing anything. `yolop-extension-logfire` 0.2.0 is the first extension published with prebuilt servers.
- The Logfire exporter emits a real trace tree: spans nest by the host's `parent_span_id`, carry the Gen-AI semantic conventions (`gen_ai.request.model`, token counts, tool names), and give thinking its own paired span.
- Model switching serves a curated, cross-provider list the user owns, administered by `yolop models list|add|rm|move|use|reset`, with "Browse all models…" still reaching the per-provider catalog.
- An opt-in `--compact-work` mode keeps each active turn on one mutable summary row, expandable with `Ctrl+O`.
- The global config and data directories are relocatable through `--config-dir` / `--data-dir` or `YOLOP_CONFIG_DIR` / `YOLOP_DATA_DIR`, so isolating a run no longer means moving `HOME`.
- Local inference gains `metal` and `cuda` backends and a default model that runs at small-model speed (Qwen3-30B-A3B-Instruct, Q4 GGUF), both gated in CI.

## In the changelog

- [Config and data directories can be relocated](/changelog/2026-08-18-yolop-relocatable-config-dirs/)
- [Model switching uses a curated list you manage](/changelog/2026-08-20-yolop-curated-model-list/)
- [Compiled extensions install from prebuilt binaries](/changelog/2026-08-20-yolop-extension-prebuilt-binaries/)

Before upgrading, read the breaking changes: [Model switching uses a curated list you manage](/changelog/2026-08-20-yolop-curated-model-list/).

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.17.0) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0170---2026-08-21) for every pull request in this version.
