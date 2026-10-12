---
title: "Yolop 0.16.0 release"
description: "Yolop sets itself up as a Buzz Desktop harness in one command, extensions are managed from the extensions CLI, and profiles can define a whole agent."
date: 2026-08-19
kind: release
significance: notable
products: [yolop]
release: "0.16.0"
---

## Highlights

- Sessions can hand work to each other: the built-in `session_coordination` capability discovers opt-in local sessions in the same project, reserves one, dispatches a durable task, and wakes the coordinator on completion, all through `yolop coordination`.
- Extension and coordination administration moved from model tools to an attached CLI. `yolop extensions ...` and `yolop coordination ...` run against the live session over one-shot anonymous pipes, so enable, disable, and reload affect the session you are in without spending model context on tool schemas.
- Profiles now define a whole agent, not just execution settings: `capabilities`, `mcp`, `skills_dir`, and standing `instructions` are all selectable per run.
- `run_command` covers the entire command registry on every host. The nine-command allowlist is gone, and ACP and `--print` sessions get command dispatch for the first time.
- An experimental in-process `local` provider runs a model inside the binary, with `yolop models pull` owning the weights. Off in default features so source builds stay fast; the release binaries carry it.
- Memory is scoped: `remember`, `recall`, and `forget` take `global` (the default) or `repository`, with repository memory shared across linked worktrees.
- Editor integration widens: `yolop into buzz` writes a Buzz Desktop harness, and ACP clients can be handed an opt-in loopback setup page (`--acp-setup-page`) so API-key providers have an in-conversation path.
- Copying wrapped text out of the transcript no longer inserts newlines that were never there, on tuika 0.10.

## In the changelog

- [Set up as a Buzz Desktop harness in one command](/changelog/2026-08-15-yolop-buzz-desktop-harness/)
- [Extension management moves to the extensions CLI](/changelog/2026-08-17-yolop-extensions-cli-only/)
- [Profiles can define a whole agent](/changelog/2026-08-17-yolop-profiles-define-agents/)
- [Local sessions can hand work to each other](/changelog/2026-08-18-yolop-coordinate-local-sessions/)
- [Memory can be scoped to a repository](/changelog/2026-08-18-yolop-repository-scoped-memory/)

Before upgrading, read the breaking changes: [Extension management moves to the extensions CLI](/changelog/2026-08-17-yolop-extensions-cli-only/).

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.16.0) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0160---2026-08-19) for every pull request in this version.
