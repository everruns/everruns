---
title: Framework crates need Rust 1.100
description: The July dependency group moves the published crates to Rust 1.100, because progenitor 0.17 generates clients with a compiler that needs 1.97 or later.
date: 2026-11-27
kind: breaking
significance: minor
products: [framework, platform]
surfaces: [code, operations]
links:
  docs: [/framework/serve-tutorial/]
  prs: [everruns/everruns#5693]
---

The published Everruns crates now build with Rust 1.100.0 or later. Rust 1.96
through 1.99 stop compiling once you take the next release.

The change comes from the July dependency group, which moves the OpenAPI client
generator to progenitor 0.17. Progenitor 0.17 needs Rust 1.97, so the crates'
`rust-version` and the repository's `rust-toolchain.toml` follow it to the
current stable release.

If your application embeds the Framework, as in the
[serve tutorial](/framework/serve-tutorial/), update the toolchain and refresh
the lockfile:

```sh
rustup update 1.100.0
cargo update
```

Pin `1.100.0` in `rust-toolchain.toml` if you build with a fixed toolchain.
Self-hosted Platform images already build with Rust 1.100 and need no change.
