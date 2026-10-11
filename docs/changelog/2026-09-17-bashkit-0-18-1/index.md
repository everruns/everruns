---
title: "Bashkit 0.18.1 release"
description: "Bashkit 0.18.1 was released on 2026-09-17 with 2 highlights and 1 changelog entry. The release notes and any upgrade notes are linked here."
date: 2026-09-17
kind: release
significance: minor
products: [bashkit]
release: "0.18.1"
---

## Highlights

- **ScriptedTool builtins can now use clap.** `ScriptedToolBuilder::builtin(...)` registers a raw-argv builtin alongside `ToolDef` tools, so commands needing positionals, short flags, `--`, or parse-time required-argument checking work inside a `ScriptedTool` ([#2408](https://github.com/everruns/bashkit/pull/2408)).
- **Closed schemas reject unknown flags.** With `"additionalProperties": false`, an unrecognised flag is now an error naming the valid flags instead of a silently-dropped property, and clap output is stripped of ANSI codes ([#2408](https://github.com/everruns/bashkit/pull/2408)).

## In the changelog

- [ScriptedTool accepts clap-based builtins](/changelog/2026-09-17-bashkit-scripted-tool-clap-builtins/)

See the [GitHub release](https://github.com/everruns/bashkit/releases/tag/v0.18.1) and the [full release notes](https://github.com/everruns/bashkit/blob/main/CHANGELOG.md#0181---2026-09-17) for every pull request in this version.
