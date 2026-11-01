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
The selection is server-side. Its administrative cause is unconfirmed because
reading the default-setup endpoint returned HTTP 403.

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
It processed 3,518 Rust files and finalized the database in about 28 minutes.
The workspace inventory contains 49 packages and 296 target roots. All 296
target roots have semantic AST and crate records in the finalized database.
Of 3,469 tracked Rust files, 3,444 have semantic AST evidence. The complete
database contains 408 crate roots, including independent workspaces, without
partitioning away cross-component relationships. Extraction and AST evidence
alone are not the success bar: the standard query suite must also pass before
results can be published.

An earlier attempt exited 137 with one OOM kill because approximately 6.3 GiB of
bundle, database, and build files lived on RAM-backed `/tmp`. Moving this scratch
to disk allowed the complete extraction without another OOM. Sampled aggregate
anonymous memory peaked at approximately 12.15 GiB; total cgroup usage also
includes reclaimable disk cache and is not a process-memory requirement.
`--ram` is a suggestion to the Rust extractor, not a hard memory cap. The
candidate rejects tmpfs/ramfs scratch, keeps Cargo build output on disk, records
resource evidence, and uses the CodeQL bundle linked to the pinned action.
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
semantic completeness of those fragments or the shared module. A file present in the source archive or a
native Cargo check is insufficient evidence. The candidate must remain draft
until extraction evidence is reviewed and this target gap has an explicit
resolution; do not describe it as complete Rust coverage.

## Related

- [Security Testing](security-testing.md)
- [CI Build Time](../project/ci-build-time.md)
