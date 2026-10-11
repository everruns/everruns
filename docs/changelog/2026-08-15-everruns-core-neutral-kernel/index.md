---
title: everruns-core 0.18 becomes a neutral execution kernel
description: Hosted platform records and services moved out of everruns-core into everruns-platform and sibling crates, and the everruns-runtime crate was removed.
date: 2026-08-15
kind: breaking
significance: major
products: [framework]
surfaces: [code]
release: "0.18.0"
links:
  docs: [/framework/upgrade-notes/]
  prs: [everruns/everruns#3101, everruns/everruns#3160, everruns/everruns#3179]
---

In 0.18, `everruns-core` keeps only the execution kernel. Persisted records and hosted services (sessions, agents and agent versions, harnesses, evals and observers, connectors, system email, feature flags, OAuth and credential infrastructure) moved to `everruns-platform`, `everruns-host`, `everruns-provider` and other owning crates. The logic-free `everruns-runtime` compatibility crate is removed, and engines now own the session lifecycle on one execution path for in-process and durable runs.

REST and gRPC shapes and the stored schema are unchanged; code that imports from `everruns_core` needs new paths. Self-hosted servers need no change.

See [Upgrade notes: 0.18](/framework/upgrade-notes/), [#3160](https://github.com/everruns/everruns/pull/3160), [#3101](https://github.com/everruns/everruns/pull/3101) and [#3179](https://github.com/everruns/everruns/pull/3179).

## Update 2026-10-04

Several crates named here were later consolidated: `everruns-platform` became `everruns-capabilities` in 0.39, and `everruns-host` became an `everruns-core` feature in 0.40. See [Framework crates consolidated into everruns-core and the facade](/changelog/2026-10-04-framework-crates-consolidated/).
