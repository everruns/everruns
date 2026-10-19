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

## Validation policy

Pull requests run formatting, linting, repository guards, affected workspace tests,
and relevant UI/docs checks. The affected set includes reverse Cargo dependencies;
server persistence, authorization, and migration changes add focused contracts.
Hosted capability graph validation selects both portable builtins and environment
capabilities, matching server/worker composition; either catalog alone can omit
dependencies supplied by the other.
Database-backend changes also exercise production-router fixtures that configure
their own environment outside the shared API harness.
Ordinary Rust changes do not build the server/worker/CLI binaries, full PostgreSQL
shards, SDK cohorts, or Docker images. Public API and packaging inputs retain their
specific consumers. Chaos functional coverage uses the existing shared CI binaries
in broad batches and PRs touching the harness or cancellation controls; its
separate release-profile build is a manual benchmark. See
[selection](../../.github/scripts/ci/focused.py) and
[workflow routing](../../.github/workflows/ci.yml).

Main uses push-triggered coalescing: one batch runs to completion while only the
newest pending revision survives. There are no code-validation scheduler ticks when
main is idle. Change detection compares the selected revision against the last
successful ancestor batch, covering intervening merges. Failed, cancelled, PR, and
label-only runs never advance the baseline. Without a baseline every surface runs.
Actions history records attempts; a failed revision receives no automatic same-SHA
retry. See [baseline selection](../../.github/scripts/ci/plan.py).

Nightly validates feature-isolation shapes and portability/MSRV when code changed
since the previous nightly attempt. Provider health still runs without source
changes because external behavior can change independently. Dependency advisories
retain their independent schedule. Explicit full validation always runs all surfaces.

Functional PR/main database tests use native PostgreSQL and private template copies.
Nightly/full suites supply a disposable external PostgreSQL service with durability
on. Durable-store tests keep their external service in every broad batch. Tests that
share process-global state retain serialization within each process. Native domain
and API integration targets use a few independent serial batches, each with its own
PostgreSQL cluster; explicitly configured external databases retain one serial
process because raw-pool fixtures share them. A fixture readiness handshake warms
the native installation before other batches start. The executable is compiled once
and its inventory is partitioned without dropping tests or creating a process per
test. Private-database server unit tests retain bounded thread concurrency. See [Embedded PostgreSQL](embedded-postgresql.md).

Release workflows require successful full validation of the exact main revision
before publishing. A successful ordinary batch or a different revision cannot
satisfy that gate. A manual full run can select a main ancestor while main advances;
every consumer checks out that selected revision. See
[release gate](../../.github/scripts/ci/wait_full.py).

A trusted completion workflow maintains one owned Linear issue per failing check,
with revision range, run/check links, and candidate PRs. It preserves human analysis,
ignores stale events, and resolves only after a later successful batch containing
the failing revision passes the check. A same-revision green rerun needs human
infrastructure triage. See [failure tracking](../../.github/scripts/ci/track_failures.py).

The accepted tradeoff is that cross-component regressions can be found after merge.
Target ordinary PR wall time below ten minutes; measure it rather than infer savings
from removed jobs. Compare warm runs with warm runs and record both wall time and
runner minutes. The manual [test archive pilot](../../.github/workflows/ci-test-archive.yml) compiles
one server libtest executable and runs identical verified bytes on native and durable
PostgreSQL runners. The durable leg also verifies a committed write survives an
abrupt restart of its disposable PostgreSQL service. Compare its compile/transfer/execute times before adopting it
on the critical path. Broader workspace artifact caching and larger runners need
warm-run benchmarks after workload reduction. The pilot compares dependency-only
and workspace-crate caches with separate revision-scoped keys and identical build
flags. Run it twice at the same revision, record exact hits/misses and cache sizes,
and compare gzip speed/size alongside per-directory cleanup timings. Its gzip payload
is uploaded without a second compression pass. Experiment keys must not become the
normal CI cache baseline without evidence; account for repository cache pressure.

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
  runs one invocation. The server shard builds library, contracts and repository
  artifacts together, then directly executes them with target-specific concurrency.
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
  across tests sharing a process. New server integration tests go into `domain/` by default. The same shape
  holds for the server shard (`contracts`, `server_integration`), `everruns` (`facade`,
  `local`, `host`), `everruns-core` (`consolidated`) and `everruns-test-support` (`integration`).
  Core consolidation replaces the former engine/builtins/MCP/AG-UI host binaries.
  Two narrow host-shim feature runs intentionally verify MCP and host-shell without
  feature union hiding their transport or integration dependencies; the legacy host
  fixtures retain serial execution.
- **A `--no-run` prebuild does not help.** It was tried and removed: the targets it
  builds are not reused by later `cargo test` steps, so it only adds a wave.
  Focused server PR tests and the broad server shard instead compile their selected targets in one invocation
  and execute Cargo's emitted libtest binaries directly, retaining each target's
  filters, package working directory, and manifest environment. This avoids a
  second Cargo build without dropping the authorization contracts. The runner
  reports compilation and per-target execution separately; other affected-crate
  groups retain Cargo execution and their doctests.
- **Compile workflow tests before binaries are ready.** The workflow suite used to
  start compiling only after the server/worker build completed. Its libtest producer
  now runs alongside that build, using the integration dependency cache read-only.
  Execution downloads the same-run executable, verifies its checksum and supplies
  Cargo's package context. Provider credentials remain confined to trusted-main
  execution. See [artifact execution](../../.github/scripts/ci/libtest.py).
- **One producer per `rust-cache` `shared-key`.** Cache entries are immutable, so the
  first job to finish writes the key. When ten jobs shared one key, a light job could
  store a `target/` shaped for a different workload and every heavy consumer rebuilt on
  top of it — the key reported a full match while 490 of 532 crates recompiled. Keys are
  now split by workload with the heaviest consumer pinned as the sole producer.
- **Keep large CI Rust cache writes on main.** PRs restore the canonical
  dependency caches but do not retain their own Rust copies. Immutable generations
  already compete under a fixed repository budget; branch copies can evict the
  useful main baseline. The trusted [closed-PR cleanup](../../.github/workflows/ci-cache-cleanup.yml)
  also removes remaining Node and tool caches after closure, rechecking current PR
  state and accepting only exact PR merge refs. It never deletes branch caches or
  executes PR code. Manual cleanup sweeps existing closed PRs; no periodic tick or
  paid cache-capacity increase is required.
- **Benchmark optimization and concurrency separately.** The manual
  [pilot](../../.github/workflows/ci-test-archive.yml) compares CI binary optimization
  on a pinned runner image using empty targets followed by dependency-only warm
  targets. It writes no experiment caches. A separate server trial compiles once
  and repeats the same library executable at bounded concurrency in both orders,
  preserving each test's private PostgreSQL database. Global-state integration
  targets retain their serial policy. Profile choices require compile, archive and
  execution evidence plus the existing downstream functional assertions.
- **Rotate the key name when changing producers.** Immutability means a pinned producer
  cannot replace an entry written under the old race; the `-v2` suffixes exist for that
  reason. Bump the suffix again if a key is ever poisoned the same way.
- **Jobs sharing a key must agree on `RUSTFLAGS`.** It is part of cargo's fingerprint, so
  a job joining a key without the linker flag silently invalidates the shared `target/`.
- **Linking, not codegen, dominates the test build.** `crates/server` alone has 51 test
  binaries, each linking the whole server crate. CI uses `lld` on the heavy jobs through
  a job-scoped `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS` rather than a checked-in
  `.cargo/config.toml`, so a checkout without `lld` still builds.
- **Measure before deleting installed SDKs.** Two controlled hosted runners began
  with 85 GiB free, yet Android deletion consumed 76–100 seconds. Focused PR groups
  and the largest unit-test shards now skip SDK cleanup when at least 50 GiB is
  available. Smaller images retain their previous cleanup paths, including the
  extended tooling list for the heaviest shards; the free-space threshold covers
  cache extraction, build artifacts and temporary linker output together. See
  [disk preparation](../../scripts/ci-prepare-disk.sh). This avoids trading latency
  for the earlier disk-exhaustion failure.

The workflow-test producer also uses conditional disk preparation: an unconditional
SDK cleanup plus linker package setup exhausted its five-minute startup budget in
[main run 38663303410](https://github.com/everruns/everruns/actions/runs/38663303410).
Rust jobs now reuse the image's existing linker, exposing its versioned executable
under the standard command name in runner-local temporary storage when needed.
Package installation remains a bounded fallback, and shared Cargo linker flags
stay identical. See [linker preparation](../../scripts/ci-prepare-linker.sh).

## CI-only optimization measurements

The first same-revision, pinned-image profile trial measured opt-level 1 versus 0:
20m18s versus 9m14s cold, and 13m24s versus 5m37s with dependencies retained.
Both compiled successfully; a later probe incorrectly tried to request worker help
and was repaired because the worker starts directly from environment configuration.
The corrected trial repeats both profiles and validates archive size plus retained
line-number/symbol sections. See [initial trial](https://github.com/everruns/everruns/actions/runs/38659951614)
and [corrected trial](https://github.com/everruns/everruns/actions/runs/38661696094).
These are paired samples, not a latency distribution or an ordinary-PR SLO.

Hosted functional CI adopts the faster optimization setting only for its shared
server/worker/CLI producer, retaining line-number backtraces and the existing
workflow, chaos, CLI, SDK and full-stack consumers. Its dependency key is isolated
from the previous profile; published release settings and the test dependency
caches are unchanged. Faster artifact compression limits transfer preparation
cost for the larger executables. See [producer](../../.github/workflows/ci.yml).

The same library executable passed all 3,704 tests (four ignored) in each of six
native-PostgreSQL runs, in thread order 2/4/8/8/4/2. Mean execution was 309.8s,
265.1s and 262.3s respectively. Eight threads saved only about one percent versus
four, so CI retains four and the existing serial integration targets. The pilot
compiled the library in 524.4s; a separate warm focused-server sample spent 430.5s
compiling and 251.9s executing library/contracts. Remaining ordinary-PR latency is
still dominated by Rust compilation; sub-ten-minute whole PR latency is unproven.
Private database isolation is unchanged. See [concurrency trial](https://github.com/everruns/everruns/actions/runs/38659951614).

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
