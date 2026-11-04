---
title: Bashkit Python builds on Windows again
description: Bashkit's embedded Python no longer crashes the Rust compiler on Windows, while preserving module alignment and Python in the hosted integration catalog.
date: 2026-10-25
kind: fix
significance: minor
products: [bashkit, framework]
release: "0.49.0"
surfaces: [code]
links:
  prs: [everruns/bashkit#2777]
---

Windows applications can build Bashkit's embedded CPython with Rust 1.96.0.
Previously, the embedded module's static alignment made the compiler stop with
an illegal instruction before the application compiled. Windows now uses a
cached allocation that preserves the 64 KiB alignment Wasmtime needs.

Use Bashkit 0.20.1 or later in the 0.20 series and refresh the application's
lockfile. Everruns' hosted integration catalog consumes the corrected
dependency with Python enabled. Apple keeps its existing allocation path,
and ELF hosts keep the aligned static. See the
[Bashkit release notes](https://github.com/everruns/bashkit/releases/tag/v0.20.1).
