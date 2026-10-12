---
title: "Yolop 0.18.0 release"
description: "Manage and log in to MCP servers from the CLI, and session search and self-configuration move to CLI commands, with fixes listed in the release notes."
date: 2026-09-11
kind: release
significance: notable
products: [yolop]
release: "0.18.0"
---

## Highlights

- `yolop sessions` CLI replaces the `search_sessions` model tool, with `search` and `list` subcommands.
- Capability names unify behind CLI routes: `yolop` prefix stripped to bare IDs, MCP/provider/model self-configuration moves through CLI control routes.
- First-class env-context entries with `--env-context` flag and Paseo detection.
- OpenRouter attestation-gate guidance, account gates surfaced as links, plus `yolop mcp login` for remote OAuth servers.
- Root-managed cancellation with automatic conflict choice, and upstream v0.25.0 cone batch adoption.

## In the changelog

- [Manage and log in to MCP servers from the CLI](/changelog/2026-09-10-yolop-mcp-cli-login/)
- [Session search and self-configuration move to CLI commands](/changelog/2026-09-11-yolop-sessions-cli-capability-ids/)

Before upgrading, read the breaking changes: [Session search and self-configuration move to CLI commands](/changelog/2026-09-11-yolop-sessions-cli-capability-ids/).

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.18.0) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0180---2026-09-11) for every pull request in this version.
