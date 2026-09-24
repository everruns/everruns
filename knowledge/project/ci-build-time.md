---
type: Specification
title: "CI Build Time"
description: "Why the Rust CI jobs spend most of their wall-clock compiling, and which levers actually move it."
tags:
  - everruns
  - project
  - build
  - ci
---
# CI Build Time

## Abstract

The PostgreSQL integration shards were the longest jobs on PR runs, and the obvious
reading — that the tests are slow — was wrong: they spent roughly six times longer
compiling than executing tests. This concept records the measured breakdown so the
question does not have to be re-investigated, and separates the levers that work from
the ones that look plausible and do not. The shape generalises to the other Rust jobs,
which share the same cache and linker characteristics.

## Measured Baseline

Taken from the domain shard before any change ([run 35165812202](https://github.com/everruns/everruns/actions/runs/35165812202), 16.2 min total):

| Step | Build | Tests |
|---|---|---|
| blob offload | 5m36s | 1.0s |
| domain integration | 3m07s | ~50s |
| previously-unenumerated | 3m18s | ~110s |
| **Total** | **12m01s** | **~2m45s** |

After the levers below, on a warm cache: domain 16.2m to 7-9m, server 14.3m to 10m,
durable 9.1m to 2-3m.

## Constraints

- **Invocation count drives build time.** Each `cargo test` invocation pays its own
  codegen and link pass, so CI keeps the count per shard to a minimum: the domain shard
  runs one invocation, the server shard two because its suites disagree on
  `--test-threads` and merging them would serialise the lib's 2451 unit tests.
- **`Compiling <pkg>` does not mean the crate was rebuilt.** Cargo prints it whenever it
  builds any unit of a package, including a single test binary. Reading those lines as
  rebuilds is what first made this look like a 3x redundant compile.
- **`--lib` and `--test` need different lib builds.** `--lib` compiles a test-mode lib,
  `--test` a plain one. They are distinct compilation units, so splitting them across
  invocations forces one genuine extra compilation. That cost is inherent to cargo.
- Measured on `everruns-server`: `--lib` cold 6m00, the first `--test` after it 3m41,
  each further `--test` ~29s. The residue is linking that test binary, which is
  unavoidable — the crate has 51 of them. CI shows the same shape (4m25 then 3m00 for
  three binaries). The current layout is close to the floor; do not re-cut it without
  new measurements.
- **Fewer test binaries, fewer links.** The domain shard's server tests are one target,
  `crates/server/tests/domain/main.rs`, with each former file as a module; it replaced 40
  binaries that each linked the whole server crate. A test file stays its own binary only
  when it installs process-global state (a metrics recorder, env vars), since those leak
  across tests sharing a process. New server integration tests go into `domain/` by default.
- **A `--no-run` prebuild does not help.** It was tried and removed: the targets it
  builds are not reused by the run steps, so it only adds a wave.
- **One producer per `rust-cache` `shared-key`.** Cache entries are immutable, so the
  first job to finish writes the key. When ten jobs shared one key, a light job could
  store a `target/` shaped for a different workload and every heavy consumer rebuilt on
  top of it — the key reported a full match while 490 of 532 crates recompiled. Keys are
  now split by workload with the heaviest consumer pinned as the sole producer.
- **Rotate the key name when changing producers.** Immutability means a pinned producer
  cannot replace an entry written under the old race; the `-v2` suffixes exist for that
  reason. Bump the suffix again if a key is ever poisoned the same way.
- **Jobs sharing a key must agree on `RUSTFLAGS`.** It is part of cargo's fingerprint, so
  a job joining a key without the linker flag silently invalidates the shared `target/`.
- **Linking, not codegen, dominates the test build.** `crates/server` alone has 51 test
  binaries, each linking the whole server crate. CI uses `lld` on the heavy jobs through
  a job-scoped `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS` rather than a checked-in
  `.cargo/config.toml`, so a checkout without `lld` still builds.

## Success Bar

- A claim that a CI job is slow cites the build/test split from its log, not the job
  duration alone.
- Timings are compared warm against warm. A run that rotates a cache key — any change to
  a manifest, the lockfile, or `RUSTFLAGS` — starts cold and is not comparable to a warm
  one; several apparent regressions during this work were only that.
- Changes that move work between jobs state what the required check still covers.

Relevant references:
- [`knowledge/project/build-artifact-size.md`](build-artifact-size.md) - why the crates compile large in the first place.
- [`knowledge/project/dismissed-options.md`](dismissed-options.md) - `codegen-units` tuning.
- [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) - the shards, cache keys, and linker wiring.
