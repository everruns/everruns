---
title: "Bashkit 0.17.0 release"
description: "Bashkit 0.17.0 was released on 2026-08-22 with 5 highlights and 2 changelog entries. The release notes and any upgrade notes are linked here."
date: 2026-08-22
kind: release
significance: notable
products: [bashkit]
release: "0.17.0"
---

## Highlights

- **Bashkit runs as a plain wasm component, with no JS engine and no WASI.**
- **Untrusted input costs bounded work everywhere it is parsed.** Script
- **Sandbox escapes closed at the host boundary.** The host bridge no longer
- **The always-on dependency graph is 21 crates smaller and the browser wasm
- **Every cargo lockfile in the tree is audited, clearing 16 wasmtime

## In the changelog

- [TLS backend and timezone data become explicit features](/changelog/2026-08-21-bashkit-tls-tzdata-features/)
- [Runs as a WebAssembly component without JS or WASI](/changelog/2026-08-21-bashkit-wasm-component-no-wasi/)

Before upgrading, read the breaking changes: [TLS backend and timezone data become explicit features](/changelog/2026-08-21-bashkit-tls-tzdata-features/).

See the [GitHub release](https://github.com/everruns/bashkit/releases/tag/v0.17.0) and the [full release notes](https://github.com/everruns/bashkit/blob/main/CHANGELOG.md#0170---2026-08-22) for every pull request in this version.
