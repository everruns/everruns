---
title: Yolop 0.23.3 restores Linux download compatibility
description: Yolop 0.23.3 restores Linux downloads for glibc 2.39 and newer, with an ABI check before publication and the MCP OAuth fixes from 0.23.2.
date: 2026-11-06
release: "0.23.3"
kind: release
significance: minor
products: [yolop]
surfaces: [cli]
links:
  prs: [everruns/yolop#918]
---

[Yolop 0.23.3](https://github.com/everruns/yolop/releases/tag/v0.23.3)
restores the glibc 2.39 baseline for Linux downloads. The 0.23.2 Linux binary
required glibc 2.43 and could fail before starting on older systems. Source
installs were unaffected by that packaging error.

Upgrade with `brew upgrade everruns/tap/yolop`, install the new Linux archive,
or build from source with `cargo install yolop --version 0.23.3 --locked`.
Linux release builds now check their actual libc imports before publication.

This patch keeps the [MCP OAuth resource continuity changes](/changelog/2026-11-05-mcp-oauth-resource-continuity/)
from 0.23.2. Existing credentials without a stored resource still require
`yolop mcp login <name>` when their access token next needs refresh.
