//! Stability tiers for the framework surface: what each marker promises.
//!
//! Rust has no stable-toolchain equivalent of `rustc`'s `#[stable]` /
//! `#[unstable]` attributes (nightly-only `staged_api`), so this crate marks
//! stability in rustdoc instead: each public module carries a one-line
//! `Stability:` banner at the top of its docs, and this module defines what
//! each tier means. `grep -rn "Stability:" crates/everruns/src` lists every
//! marker; the maintained policy lives in
//! `knowledge/framework/api-stability.md`.
//!
//! | Tier | Promise |
//! |------|---------|
//! | Stable | No breaking change without a major version bump. |
//! | Alpha | May break without a major bump. New alpha types that can grow (enums, option structs) should be `#[non_exhaustive]`. |
//!
//! Items without a marker are provisional: treat them as alpha until marked.
//!
//! First pass: the direct-LLM surface ([`llm`], [`Model`], and the provider
//! traits behind them) is stable; the [`classifier`] surface is alpha.
