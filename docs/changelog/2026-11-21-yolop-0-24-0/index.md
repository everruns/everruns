---
title: "Yolop 0.24.0 release"
description: "ACP protocol v2, enforced worktree choice, the engine completion check, Codex model discovery, and session hardening land in Yolop 0.24.0."
date: 2026-11-21
kind: release
significance: notable
products: [yolop]
release: "0.24.0"
---

## Highlights

- Serve ACP protocol v2 beside v1, with shell command cancel, workspace folders, and tool result images forwarded to the editor.
- Enforce the first-mutation worktree isolation decision in auto mode, and require explicit trust before loading workspace hooks.
- Use the engine completion check and credit wait, discover Codex models from the provider catalog, and mirror session titles into Rex.
- Add an optional MCP interactive discovery budget, isolate local execution databases, and coordinate MCP OAuth refresh across processes.

## In the changelog

- [Yolop speaks Agent Client Protocol v2 for Zed's Delta](/changelog/2026-11-12-yolop-acp-v2/)
- [Choose a worktree before the first edit](/changelog/2026-11-13-yolop-explicit-worktree-choice/)
- [Yolop asks before running a repository's hooks](/changelog/2026-11-07-yolop-workspace-hook-trust/)
- [Yolop names your Rex tabs from session titles](/changelog/2026-11-08-yolop-rex-tab-titles/)
- [Concurrent Yolop sessions share MCP OAuth refreshes safely](/changelog/2026-11-09-yolop-mcp-oauth-refresh/)
- [Yolop stops cutting off long tasks at ten minutes](/changelog/2026-11-09-yolop-sliding-repair-window/)

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.24.0) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0240---2026-11-21) for every pull request in this version.
