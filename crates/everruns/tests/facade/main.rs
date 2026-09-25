//! Single integration-test binary for the `everruns` facade crate.
//!
//! Each `tests/*.rs` file is its own binary, and every binary links the whole
//! `everruns` crate from scratch (see `knowledge/project/ci-build-time.md`).
//! Merging these modules into one target pays that link cost once instead of
//! once per file. `trybuild.rs` (a compile-fail harness over `tests/ui/`),
//! `tls_startup.rs` (asserts concurrent process-global TLS provider install),
//! and the `local`-feature suites (`tests/local/main.rs`, declared via
//! `[[test]]` in Cargo.toml) stay separate binaries.
//!
//! Run with: `cargo test -p everruns --all-features --test facade`
//! Run one module: `cargo test -p everruns --all-features --test facade <module>::`

mod advanced_capabilities;
mod agent_builder;
mod application_parity;
mod ask_user;
mod capability_configuration;
mod custom_provider;
mod engine_sessions;
mod facade_smoke;
mod function_tools;
mod harness;
mod lifecycle_hooks;
mod model_catalog;
mod session_events;
mod session_history;
mod session_identity;
mod session_work;
