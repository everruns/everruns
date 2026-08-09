---
title: Embedding Everruns
description: Choose between the application-facing Framework and low-level host composition.
sidebar:
  hidden: true
---

For an application that runs agents in its own Rust process, start with the
[Everruns Framework](/framework/) and the `everruns` crate. Its offline
[quickstart](/framework/quickstart/) needs no server, worker, database, network,
or credentials.

Low-level embedding is for applications that are themselves execution hosts:
servers, evaluation harnesses, research runtimes, or specialized systems that
must replace backend stores, platform definitions, or orchestration phases.
Those hosts compose `everruns` with `everruns-host` and the focused crates they
need.

The useful low-level boundary, security obligations, and crate-selection
guidance now live in [Custom backends](/framework/custom-backends/). Existing
0.17 runtime users should follow the
[runtime migration guide](/framework/runtime-compatibility/).
