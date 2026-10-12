---
title: "Everruns 0.20.0: Register capabilities on a running runtime"
description: "A2A delegation in the everruns crate becomes an opt-in feature, and a running Framework runtime can register new capabilities without a restart."
date: 2026-08-19
kind: release
significance: notable
products: [platform, framework, cli]
release: "0.20.0"
---

## Highlights

- **Register capabilities on a running runtime**: A host holding `Arc<InProcessRuntime>` can now make a capability discovered after composition resolvable without rebuilding the runtime, via `register_capability`/`is_capability_registered`. Registration stays separate from activation, and readers always see a consistent registry snapshot ([#3224](https://github.com/everruns/everruns/pull/3224)).
- **A2A delegation is now opt-in**: Outbound A2A delegation moved behind a new `a2a` Cargo feature, off by default, so the standard build no longer drags a second HTTP/TLS stack into consumers that only run the local runtime. Hosts that delegate to remote A2A agents enable the `a2a` feature explicitly ([#3221](https://github.com/everruns/everruns/pull/3221)).

## In the changelog

- [A2A delegation in the everruns crate is now an opt-in feature](/changelog/2026-08-18-a2a-delegation-opt-in/)
- [Register a capability on a running Framework runtime](/changelog/2026-08-19-register-capabilities-running-runtime/)

Before upgrading, read the breaking changes: [A2A delegation in the everruns crate is now an opt-in feature](/changelog/2026-08-18-a2a-delegation-opt-in/).

See the [GitHub release](https://github.com/everruns/everruns/releases/tag/v0.20.0) and the [full release notes](https://github.com/everruns/everruns/blob/main/CHANGELOG.md#0200---2026-08-19) for every pull request in this version.
