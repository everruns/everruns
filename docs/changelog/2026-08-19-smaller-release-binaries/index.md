---
title: Everruns CLI, server and worker binaries are about 20% smaller
description: Release builds now strip symbols and the CLI boxes its command dispatch, cutting the CLI by 22% and the server and worker binaries by 18 to 19%.
date: 2026-08-19
kind: improvement
significance: minor
products: [platform, cli]
surfaces: [cli, operations]
release: "0.21.0"
links:
  docs: [/features/cli/]
  prs: [everruns/everruns#3227]
---

Shipped release artifacts are smaller. Symbols are stripped from release builds, and each `everruns` CLI subcommand runs behind a boxed future so command code is no longer duplicated into the entry point.

The CLI binary went from 18.1 MB to 14.0 MB (22.4%). The server and worker binaries are 18.3% and 19.0% smaller from stripping alone. Behavior is unchanged.

See [CLI](/features/cli/) and [#3227](https://github.com/everruns/everruns/pull/3227).
