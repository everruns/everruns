---
title: Agents read several workspace files in one read_many_files call
description: The File System capability adds read_many_files, which reads 2 to 10 independent files in one ordered call and halves model round trips for discovery.
date: 2026-08-24
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [agent]
release: "0.22.0"
links:
  docs: [/capabilities/file-system/]
  prs: [everruns/everruns#3275]
---

Agents with the File System capability get a `read_many_files` tool. It reads 2 to 10 independent session-workspace files in one call, returns them in order with a per-file result or error, and stops below a 60 KiB combined output. `read_file` is unchanged.

Models that emit one tool call per response no longer need one model round per file. In the PR's tests, three independent reads took 2 model calls instead of 4 and about half the input tokens. Loop detection recognizes repeated batch reads.

See [File System](/capabilities/file-system/) and [#3275](https://github.com/everruns/everruns/pull/3275).
