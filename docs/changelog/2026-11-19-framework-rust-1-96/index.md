---
title: Framework crates need Rust 1.96
description: Bashkit 0.21.1 moves the embedded shell and Python to Wasmtime 49.0.6, which fixes six Wasmtime advisories and raises the minimum Rust to 1.96.
date: 2026-11-19
kind: breaking
significance: minor
products: [framework, platform]
surfaces: [code, operations]
links:
  docs: [/framework/serve-tutorial/]
---

The published Everruns crates now build with Rust 1.96.0 or later. Rust 1.94 and
1.95 stop compiling once you take the next release.

The change comes from Bashkit 0.21.1, which Everruns now uses for the shell and
embedded Python. It moves from Wasmtime 48.0.8 to 49.0.6, fixing RustSec
advisories RUSTSEC-2026-0566 through RUSTSEC-2026-0571. Wasmtime 49 needs Rust
1.96, so the crates' `rust-version` follows it.

If your application embeds the Framework, run `rustup update` (or pin
`1.96.0` in `rust-toolchain.toml`) and refresh the lockfile with
`cargo update -p bashkit`. Self-hosted Platform images already build with Rust
1.96 and need no change.
