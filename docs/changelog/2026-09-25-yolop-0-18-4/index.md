---
title: "Yolop 0.18.4 release"
description: "ACP advertises user-invocable skills to clients and announces finished background work even when the wake turn itself stays silent."
date: 2026-09-25
kind: release
significance: minor
products: [yolop]
release: "0.18.4"
---

## Highlights

- ACP now advertises user-invocable skills to clients, and announces finished background work even when the wake turn stays silent.
- Action-guard gains a Muse-only idle-promise guard on the Jev classifier.
- MCP hardens workspace execution by blocking stdio process spawning and requiring explicit environment credential bindings.
- Observability and provider handling improve: Logfire forwards full LLM cost metadata and span duration, and Codex handles cross-provider reasoning.

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.18.4) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0184---2026-09-25) for every pull request in this version.
