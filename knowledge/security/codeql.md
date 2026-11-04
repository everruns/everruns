---
type: Specification
title: CodeQL coverage and cadence
description: CodeQL migration prerequisites, Rust extraction evidence, and coverage limits.
tags:
  - security
  - ci
---

# CodeQL coverage and cadence

## Current protection and migration boundary

The repository uses GitHub-generated default setup, not a checked-in workflow.
The generated workflow on [main b4873b90](https://github.com/everruns/everruns/actions/runs/41318089887)
explicitly selected Actions, C/C++ and Python with the default query suite.
Neither Rust nor JavaScript/TypeScript had a job; a successful overall run is
therefore not Rust or UI coverage. The same missing languages were observed on
[PR 5153](https://github.com/everruns/everruns/actions/runs/41315602782) and
[PR 5155](https://github.com/everruns/everruns/actions/runs/41329670774).
The selection is server-side. Rust is [supported by both default and advanced
setup](https://github.blog/changelog/2025-10-14-codeql-scanning-rust-and-c-c-without-builds-is-now-generally-available/),
so lack of language support does not explain its absence. The administrative
cause is unconfirmed because reading the default-setup endpoint returned HTTP 403.

The [advanced workflow candidate](../../.github/codeql/advanced.yml) is deliberately
outside `.github/workflows/` and is **inactive**. Existing default scans remain
untouched. It preserves the three observed languages and analysis categories,
adds JavaScript/TypeScript, and moves the new expensive Rust analysis to weekly
and manual execution only. Ordinary PR/main scans retain the existing frequency.

GitHub [rejects custom CodeQL SARIF while default setup is enabled](https://docs.github.com/en/code-security/reference/code-scanning/sarif-files/troubleshoot-sarif-uploads/default-setup-enabled).
Activating the candidate requires explicit authorization to switch repository
security settings from default to advanced setup. Before that switch, an owner
must confirm the effective default configuration, including organization model
packs and private registry requirements; the observed generated workflow can
receive these dynamically. Preserve any such configuration in the candidate.
Do not disable default setup before the replacement and its rollback are ready.
The activation change moves the candidate into `.github/workflows/codeql.yml`,
then verifies each preserved language and the new Rust/UI scans on the exact
main revision. A failed upload or unprocessed SARIF does not establish coverage.

## Rust success bar

Rust supports [build mode none](https://docs.github.com/en/code-security/reference/code-scanning/codeql/build-options-for-compiled-languages).
It still runs Cargo workspace loading, build scripts and procedural macros;
adding a separate full `cargo build` is not the extraction strategy. Do not
reduce the workspace, prune source paths, disable features, or suppress loading
errors to make a run green or fast.

The candidate records the complete Cargo workspace inventory, executes the
standard security queries, and checks semantic crate/AST evidence for every workspace
target before uploading results. The [coverage query](../../.github/codeql/rust/Files.ql)
and [validator](../../scripts/codeql/verify_rust.py) distinguish indexed filenames
from extracted semantic code. Empty results, missing target roots, failed query
execution, and error diagnostics fail validation. Warning diagnostics and the
complete per-file inventory remain available for review; success does not mean
all macro expansions were complete. Findings are retained, not dismissed by the
coverage gate.

## Validation status

The candidate is not active and has not established production Code Scanning
coverage. Local CodeQL 2.27.3 extraction of the complete checkout succeeded on a
4-CPU, 16 GiB cgroup with disk-backed scratch, one Cargo build job, and no swap.
It processed 3,518 Rust files and finalized the database in about 28 minutes
using the Rust source tree at `0974319cc0515c2fcea00cd90448e0c367bdc7bc`.
The workspace inventory contains 49 packages and 296 target roots. All 296
target roots have semantic AST and crate records in the finalized database.
Of 3,469 tracked Rust files, 3,444 have semantic AST evidence. The complete
database contains 408 crate roots, including independent workspaces, without
partitioning away cross-component relationships. Extraction and AST evidence
alone are not the success bar: the standard query suite must also pass before
results can be published.

The full 37-query standard suite completed successfully in about 56 minutes.
Both ordinary CLI SARIF and the Action's pack-grouped SARIF passed the coverage
gate with all findings preserved. The full local pipeline finished within the
candidate's 120-minute budget. This establishes a viable native analysis path,
not an active GitHub Code Scanning configuration or a completed hosted-runner
upload test.

The SARIF contains zero error-level diagnostics and 83 warnings: 82 extraction
warnings plus one data-flow consistency warning (`Node should have one toString`).
The extraction warnings affect 54 files: 49 pre-existing generated files under
the local `target/` directory and five tracked files (macro-expansion limitations
and the shared eval usage module). GitHub's clean checkout and the candidate's
external build directory avoid that pre-existing generated-file inventory; the
tracked limitations remain. The CLI's "files extracted with errors" summary
includes these warning-bearing files and is not an error-level diagnostic count.
The SARIF also retains 778 unresolved-macro notifications with level `none`.
The validator handles SARIF's default warning severity when `level` is omitted;
it does not silently discard these diagnostics.

An earlier two-thread/two-Cargo-job attempt exited 137 with one OOM kill while
approximately 6.3 GiB of bundle, database, and build files lived on RAM-backed
`/tmp`. The retry moved this scratch to disk, requested one extraction thread,
and used one Cargo job; it completed without another OOM. These changes were tested
together, so the experiment does not isolate their individual contribution. Sampled aggregate
anonymous memory peaked at approximately 12.15 GiB; total cgroup usage also
includes reclaimable disk cache and is not a process-memory requirement.
`--ram` is a suggestion to the Rust extractor, not a hard memory cap. The
candidate rejects tmpfs/ramfs scratch, keeps Cargo build output on disk, records
resource evidence, and uses the CodeQL bundle linked to the pinned action. It
finalizes extraction before releasing only that run's Cargo build cache (about
5.1 GiB in the local experiment), so build and query caches need not coexist.
Extraction/finalization failures stop the job before cleanup or publication.
The database reached approximately 13 GiB during query evaluation, so the
candidate also reuses
the repository's [runner disk preparation](../../scripts/ci-prepare-disk.sh)
before initialization. Its default mode removes unused SDKs and preserves the
CodeQL tool cache. This changes only the ephemeral job filesystem, not runner
size or billing settings.
This is evidence for the tested environment, not a measured minimum capacity or
a guarantee for every hosted runner image.

The coverage gate was also exercised on a tiny real Rust fixture: semantic AST
and crate-root evidence, all 37 default-suite queries, valid SARIF, and no
extraction errors/warnings. Regression tests reject filename-only coverage,
missing crate roots, failed execution, error diagnostics, and RAM-backed scratch.

## Target limits

The candidate is explicitly **native Linux** coverage. Native semantic extraction
cannot establish coverage for `cfg(target_arch = "wasm32")` branches, WASM-only
modules, other platform-specific branches, or sources outside the loaded target
graph. Per-file semantic coverage also does not imply that every conditional
branch or macro expansion inside that file was analyzed.
In particular, Durable Object and Cloudflare example WASM paths require a
separate validated target analysis. The portable host is selected only when
`execution-host` is enabled and `host` is disabled, so all-features extraction
does not cover that alternative either. The 25 tracked files without semantic
AST evidence also include 12 trybuild fixtures, two executable demonstration
fixtures, a historical source without a Cargo manifest, two `include!` test
fragments, and the shared eval usage module. Macro expansion nodes may be
associated with the including file; this inventory alone cannot prove the
semantic completeness of those fragments or the shared module. A file present
in the source archive or a native Cargo check is insufficient evidence. These
are explicit limits of the native analysis; additional target/feature analyses
need their own semantic validation before any broader coverage claim. The
candidate remains inactive pending the settings migration described above.

## Related

- [Security Testing](security-testing.md)
- [CI Build Time](../project/ci-build-time.md)
