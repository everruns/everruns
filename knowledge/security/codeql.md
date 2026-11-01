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
standard security queries, and checks semantic AST evidence for every workspace
target before uploading results. The [coverage query](../../.github/codeql/rust/Files.ql)
and [validator](../../scripts/codeql/verify_rust.py) distinguish indexed filenames
from extracted semantic code. Empty results, missing target roots, failed query
execution, and error diagnostics fail validation. Warning diagnostics and the
complete per-file inventory remain available for review; success does not mean
all macro expansions were complete. Findings are retained, not dismissed by the
coverage gate.

## Target limits

The candidate is explicitly **native Linux** coverage. Native semantic extraction
cannot establish coverage for `cfg(target_arch = "wasm32")` branches, WASM-only
modules, other platform-specific branches, or excluded standalone workspaces.
In particular, Durable Object and Cloudflare example WASM paths require a
separate validated target analysis. A file present in the source archive or a
native Cargo check is insufficient evidence. The candidate must remain draft
until extraction evidence is reviewed and this target gap has an explicit
resolution; do not describe it as complete Rust coverage.

## Related

- [Security Testing](security-testing.md)
- [CI Build Time](../project/ci-build-time.md)
