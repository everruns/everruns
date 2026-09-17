---
type: Specification
title: "Framework API Stability Tiers"
description: "Stable versus alpha markers for framework APIs and their promises."
tags:
  - everruns
  - framework
  - rust
---

# Framework API Stability Tiers

Application authors should know which framework surfaces they can rely on.
The direct-LLM surface is the settled contract; the classifier surface is new
and still taking shape. This concept records that split and the marking
convention so later changes stay deliberate.

## Tiers

* **Stable** promises no breaking change without a major version bump.
  Applies to the direct-LLM surface: `crates/everruns/src/llm.rs`,
  `Model::complete` / `Model::completion` in `crates/everruns/src/agent.rs`,
  and the provider traits behind them (`Provider`, `ChatDriver`,
  `DriverRegistry` in `crates/everruns/src/providers/`).
* **Alpha** may break without a major bump. Applies to the classifier
  surface: `crates/everruns/src/classifier.rs` and its `everruns-core`
  re-exports in `crates/everruns/src/lib.rs`.
* **Unmarked** public items are provisional: treat as alpha until marked.

## Marking convention

Rust's `#[stable]` / `#[unstable]` attributes are nightly-only (`staged_api`),
so markers are one-line rustdoc banners, defined in
`crates/everruns/src/stability.rs`:

* Modules carry `/// Stability: stable — ...` on the declaration plus a `//!`
  banner at the top of the module file.
* New alpha types that can grow are also `#[non_exhaustive]`.

## Success bars

* Every public module in `crates/everruns` carries a `Stability:` banner.
* `cargo public-api` diffs touching stable paths are reviewed as breaking
  unless the release says otherwise.

## Non-goals

* No Cargo feature gates for alpha yet; add them only if someone needs a
  compile-time block rather than a documented promise.
